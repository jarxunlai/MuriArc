use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_core::{
    Attachment, AttachmentDerivative, AttachmentLink, AttachmentLinkTarget, Permission, RecordMeta,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, collection, ensure_lab, item,
    scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/library", get(list_library))
        .route("/attachments/{id}/links", get(list_links).post(create_link))
        .route("/attachments/{id}/derivatives", get(list_derivatives))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryQuery {
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryItem {
    attachment: Attachment,
    links: Vec<AttachmentLink>,
    derivatives: Vec<AttachmentDerivative>,
    preview_supported: bool,
    preview_href: Option<String>,
    preview_reason: Option<&'static str>,
    status: &'static str,
}

async fn list_library(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<LibraryQuery>,
) -> Result<Json<CollectionResponse<LibraryItem>>, ApiError> {
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAttachment,
    )
    .await?;
    if let Some(experiment_id) = query.experiment_id {
        let experiment = scope::experiment_with_permission(
            &state,
            &principal,
            &metadata,
            experiment_id,
            Permission::ReadAttachment,
        )
        .await?;
        if experiment.project_id != query.project_id {
            return Err(ApiError::not_found("experiment was not found")
                .with_request_id(metadata.request_id));
        }
    }

    let mut attachments = store(
        state
            .store
            .list_project_attachments(principal.lab_id, query.project_id),
        &metadata,
    )
    .await?;
    truncate(&mut attachments, collection_limit(query.limit, &metadata)?);
    let mut items = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let links = store(state.store.list_attachment_links(attachment.id), &metadata).await?;
        if query.experiment_id.is_some_and(|experiment_id| {
            !(links.iter().any(|link| {
                link.target_type == AttachmentLinkTarget::Experiment
                    && link.target_id == experiment_id
            }) || (attachment.entity_type == "experiment"
                && attachment.entity_id == experiment_id))
        }) {
            continue;
        }
        let derivatives = store(
            state.store.list_attachment_derivatives(attachment.id),
            &metadata,
        )
        .await?;
        let preview_supported = preview_media_type(attachment.media_type.as_deref());
        let id = attachment.id;
        items.push(LibraryItem {
            attachment,
            links,
            derivatives,
            preview_supported,
            preview_href: preview_supported.then(|| format!("/api/v1/attachments/{id}/preview")),
            preview_reason: (!preview_supported)
                .then_some("该科研文件可保存和下载，但当前格式不支持在线预览"),
            status: "ready",
        });
    }
    Ok(collection(items, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLinkInput {
    project_id: Uuid,
    target_type: AttachmentLinkTarget,
    target_id: Uuid,
}

async fn create_link(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(attachment_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<CreateLinkInput>,
) -> Result<(StatusCode, Json<ItemResponse<AttachmentLink>>), ApiError> {
    let attachment = authorized_attachment(
        &state,
        &principal,
        &metadata,
        attachment_id,
        Permission::WriteAttachment,
    )
    .await?;
    if attachment.project_id != Some(input.project_id) {
        return Err(
            ApiError::not_found("attachment was not found").with_request_id(metadata.request_id)
        );
    }
    authorize_link_target(
        &state,
        &principal,
        &metadata,
        input.project_id,
        input.target_type,
        input.target_id,
    )
    .await?;
    let link = AttachmentLink {
        id: Uuid::new_v4(),
        lab_id: principal.lab_id,
        project_id: input.project_id,
        attachment_id,
        target_type: input.target_type,
        target_id: input.target_id,
        created_by: principal.user_id,
        meta: RecordMeta::new(chrono::Utc::now()),
    };
    store(
        state
            .store
            .create_attachment_link(&link, &principal.audit_context(&metadata)),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(link, &metadata)))
}

async fn list_links(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(attachment_id): ApiPath<Uuid>,
) -> Result<Json<CollectionResponse<AttachmentLink>>, ApiError> {
    authorized_attachment(
        &state,
        &principal,
        &metadata,
        attachment_id,
        Permission::ReadAttachment,
    )
    .await?;
    let links = store(state.store.list_attachment_links(attachment_id), &metadata).await?;
    Ok(collection(links, &metadata))
}

async fn list_derivatives(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(attachment_id): ApiPath<Uuid>,
) -> Result<Json<CollectionResponse<AttachmentDerivative>>, ApiError> {
    authorized_attachment(
        &state,
        &principal,
        &metadata,
        attachment_id,
        Permission::ReadAttachment,
    )
    .await?;
    let derivatives = store(
        state.store.list_attachment_derivatives(attachment_id),
        &metadata,
    )
    .await?;
    Ok(collection(derivatives, &metadata))
}

async fn authorized_attachment(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
    permission: Permission,
) -> Result<Attachment, ApiError> {
    let attachment = store(state.store.get_attachment(id), metadata).await?;
    ensure_lab(attachment.lab_id, principal, metadata)?;
    let Some(project_id) = attachment.project_id else {
        return Err(ApiError::not_found("attachment was not found")
            .with_request_id(metadata.request_id.clone()));
    };
    scope::project_with_permission(state, principal, metadata, project_id, permission).await?;
    Ok(attachment)
}

async fn authorize_link_target(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    project_id: Uuid,
    target_type: AttachmentLinkTarget,
    target_id: Uuid,
) -> Result<(), ApiError> {
    scope::project_with_permission(
        state,
        principal,
        metadata,
        project_id,
        Permission::WriteAttachment,
    )
    .await?;
    match target_type {
        AttachmentLinkTarget::Project if target_id == project_id => Ok(()),
        AttachmentLinkTarget::Project => Err(ApiError::not_found("project was not found")
            .with_request_id(metadata.request_id.clone())),
        AttachmentLinkTarget::Experiment => {
            let experiment = scope::experiment_with_permission(
                state,
                principal,
                metadata,
                target_id,
                Permission::WriteAttachment,
            )
            .await?;
            if experiment.project_id == project_id {
                Ok(())
            } else {
                Err(ApiError::not_found("experiment was not found")
                    .with_request_id(metadata.request_id.clone()))
            }
        }
        AttachmentLinkTarget::Animal => {
            scope::animal_with_permission(
                state,
                principal,
                metadata,
                target_id,
                Some(project_id),
                Permission::WriteAttachment,
            )
            .await?;
            Ok(())
        }
        AttachmentLinkTarget::Worksheet
        | AttachmentLinkTarget::CollectionNode
        | AttachmentLinkTarget::DataCell => Ok(()),
    }
}

fn preview_media_type(value: Option<&str>) -> bool {
    matches!(
        value,
        Some(
            "image/jpeg"
                | "image/png"
                | "image/webp"
                | "image/gif"
                | "image/bmp"
                | "image/tiff"
                | "image/heic"
                | "image/heif"
                | "application/pdf"
        )
    )
}
