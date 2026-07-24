use std::{path::Path, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    routing::{delete, get, post},
};
use chrono::{Duration, Utc};
use muriarc_core::{
    AiConversationSource, AiConversationSourceFilter, AiConversationSourceKind,
    AiConversationSourceStatus, Attachment, Permission, RecordMeta,
};
use muriarc_data::{
    AttachmentContentKind, AttachmentFiles, AttachmentInspection, AttachmentInspectionError,
    DEFAULT_MAX_UPLOAD_BYTES, extract_ai_source_material, inspect_attachment,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse,
    attachment_files::{
        AttachmentFileError, StoredObject, remove_installed_object, write_object_with_limit,
    },
    collection, item, scope, store,
};

const RETENTION_DAYS: i64 = 30;
const MAX_FILE_NAME_BYTES: usize = 255;
const MAX_MEDIA_TYPE_BYTES: usize = 127;

pub(super) fn router() -> Router<AppState> {
    let upload = Router::new()
        .route("/ai/sources/upload", post(upload))
        .layer(DefaultBodyLimit::disable());
    upload.merge(
        Router::new()
            .route("/ai/sources", get(list))
            .route("/ai/sources/{id}/archive", post(archive))
            .route("/ai/sources/{id}", delete(discard)),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSourceView {
    id: Uuid,
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
    kind: AiConversationSourceKind,
    status: AiConversationSourceStatus,
    file_name: String,
    media_type: String,
    size_bytes: i64,
    revision: i64,
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
}

impl AiSourceView {
    fn new(source: AiConversationSource, attachment: Attachment) -> Self {
        Self {
            id: source.id,
            conversation_id: source.conversation_id,
            project_id: source.project_id,
            kind: source.kind,
            status: source.status,
            file_name: attachment.file_name,
            media_type: attachment
                .media_type
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            size_bytes: attachment.size_bytes,
            revision: source.meta.revision,
            created_at: source.meta.created_at,
            expires_at: source.expires_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    file_name: String,
    media_type: Option<String>,
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
}

async fn upload(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<ItemResponse<AiSourceView>>), ApiError> {
    ensure_human(&principal, &metadata)?;
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|size| size > DEFAULT_MAX_UPLOAD_BYTES)
    {
        return Err(payload_too_large(&metadata));
    }

    let conversation_id = query.conversation_id.ok_or_else(|| {
        ApiError::validation("conversation_id is required")
            .with_request_id(metadata.request_id.clone())
    })?;
    let project_id = source_context(
        &state,
        &principal,
        &metadata,
        Some(conversation_id),
        query.project_id,
        true,
    )
    .await?;
    let file_name = valid_file_name(query.file_name, &metadata)?;
    let header_media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value != "application/octet-stream")
        .map(str::to_owned);
    let declared_media_type = valid_media_type(query.media_type.or(header_media_type), &metadata)?;

    let root = attachment_root(&state, &metadata)?;
    let attachment_id = Uuid::new_v4();
    let object =
        write_object_with_limit(root.as_ref(), attachment_id, body, DEFAULT_MAX_UPLOAD_BYTES)
            .await
            .map_err(|error| upload_file_error(error, &metadata))?;
    let inspected = match inspect_attachment(
        &object.absolute_path,
        &file_name,
        declared_media_type.as_deref(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            cleanup_object(root, &object, attachment_id, &metadata).await;
            return Err(inspection_error(error, &metadata));
        }
    };
    let (kind, media_type) = match classify_source(
        &object.absolute_path,
        &file_name,
        declared_media_type.as_deref(),
        inspected,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            cleanup_object(root, &object, attachment_id, &metadata).await;
            return Err(error.with_request_id(metadata.request_id));
        }
    };
    let bytes = match tokio::fs::read(&object.absolute_path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            cleanup_object(root, &object, attachment_id, &metadata).await;
            tracing::error!(
                request_id = %metadata.request_id,
                %attachment_id,
                error = %error,
                "failed to read an installed AI source for validation"
            );
            return Err(ApiError::internal().with_request_id(metadata.request_id));
        }
    };
    if let Err(error) = extract_ai_source_material(kind, &file_name, Some(&media_type), &bytes) {
        cleanup_object(root, &object, attachment_id, &metadata).await;
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_ai_source",
            error.to_string(),
        )
        .with_request_id(metadata.request_id));
    }

    let now = Utc::now();
    let source_id = Uuid::new_v4();
    let attachment = Attachment {
        id: attachment_id,
        lab_id: principal.lab_id,
        // A source remains private until archive promotes it atomically.
        project_id: None,
        entity_type: "ai_conversation_source".to_owned(),
        entity_id: source_id,
        file_name,
        media_type: Some(media_type),
        relative_path: object.relative_path.clone(),
        size_bytes: object.size_bytes,
        sha256: object.sha256.clone(),
        version: 1,
        meta: RecordMeta::new(now),
    };
    let source = AiConversationSource {
        id: source_id,
        lab_id: principal.lab_id,
        user_id: principal.user_id,
        conversation_id: Some(conversation_id),
        project_id,
        attachment_id,
        kind,
        status: AiConversationSourceStatus::Ready,
        last_activity_at: now,
        expires_at: now + Duration::days(RETENTION_DAYS),
        archived_at: None,
        error_code: None,
        meta: RecordMeta::new(now),
    };
    if let Err(error) = state
        .store
        .create_ai_conversation_source(&attachment, &source, &principal.audit_context(&metadata))
        .await
    {
        cleanup_object(root, &object, attachment_id, &metadata).await;
        return Err(ApiError::from_store(error).with_request_id(metadata.request_id));
    }

    Ok((
        StatusCode::CREATED,
        item(AiSourceView::new(source, attachment), &metadata),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
    status: Option<AiConversationSourceStatus>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<AiSourceView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let conversation_id = query.conversation_id.ok_or_else(|| {
        ApiError::validation("conversation_id is required")
            .with_request_id(metadata.request_id.clone())
    })?;
    let project_id = source_context(
        &state,
        &principal,
        &metadata,
        Some(conversation_id),
        query.project_id,
        false,
    )
    .await?;
    let mut sources = store(
        state
            .store
            .list_ai_conversation_sources(&AiConversationSourceFilter {
                lab_id: principal.lab_id,
                user_id: principal.user_id,
                conversation_id: Some(conversation_id),
                project_id,
                status: query.status,
                unconsumed_only: true,
            }),
        &metadata,
    )
    .await?;
    // `None` in the store filter means “no predicate”, so enforce the
    // requested nullable project scope again before exposing metadata.
    sources.retain(|source| {
        source.project_id == project_id && source.conversation_id == Some(conversation_id)
    });
    let mut views = Vec::with_capacity(sources.len());
    for source in sources {
        let attachment = store(state.store.get_attachment(source.attachment_id), &metadata).await?;
        views.push(AiSourceView::new(source, attachment));
    }
    Ok(collection(views, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveInput {
    project_id: Uuid,
    expected_revision: i64,
}

async fn archive(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ArchiveInput>,
) -> Result<Json<ItemResponse<AiSourceView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let current = owned_source(&state, &principal, &metadata, id).await?;
    let conversation_id = current.conversation_id.ok_or_else(|| {
        ApiError::not_found("AI source was not found").with_request_id(metadata.request_id.clone())
    })?;
    let conversation_project = source_context(
        &state,
        &principal,
        &metadata,
        Some(conversation_id),
        Some(input.project_id),
        true,
    )
    .await?;
    if conversation_project != Some(input.project_id) {
        return Err(ApiError::not_found("AI source was not found")
            .with_request_id(metadata.request_id.clone()));
    }
    if current
        .project_id
        .is_some_and(|project_id| project_id != input.project_id)
    {
        return Err(
            ApiError::not_found("AI source was not found").with_request_id(metadata.request_id)
        );
    }
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        input.project_id,
        Permission::WriteAttachment,
    )
    .await?;
    let source = store(
        state.store.archive_ai_conversation_source(
            id,
            input.project_id,
            input.expected_revision,
            Utc::now(),
            &principal.audit_context(&metadata),
        ),
        &metadata,
    )
    .await?;
    let attachment = store(state.store.get_attachment(source.attachment_id), &metadata).await?;
    Ok(item(AiSourceView::new(source, attachment), &metadata))
}

async fn discard(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_human(&principal, &metadata)?;
    let current = match owned_source(&state, &principal, &metadata, id).await {
        Ok(source) => source,
        Err(error) if error.status() == StatusCode::NOT_FOUND => {
            // Deletion is intentionally idempotent. This also keeps missing
            // and foreign source identifiers indistinguishable.
            return Ok(StatusCode::NO_CONTENT);
        }
        Err(error) => return Err(error),
    };
    let attachment = store(state.store.get_attachment(current.attachment_id), &metadata).await?;
    let root = attachment_root(&state, &metadata)?.clone();
    let audit = principal.audit_context(&metadata);
    store(
        state
            .store
            .discard_ai_conversation_source(id, current.meta.revision, Utc::now(), &audit),
        &metadata,
    )
    .await?;
    match AttachmentFiles::new(root.as_ref())
        .remove_verified_object(&attachment)
        .await
    {
        Ok(()) => {
            if let Err(error) = state
                .store
                .complete_ai_conversation_source_object_deletion(
                    id,
                    attachment.id,
                    Utc::now(),
                    &audit,
                )
                .await
            {
                tracing::warn!(
                    request_id = %metadata.request_id,
                    source_id = %id,
                    attachment_id = %attachment.id,
                    error = %error,
                    "AI source object was removed but cleanup completion remains queued"
                );
            }
        }
        Err(error) => tracing::warn!(
            request_id = %metadata.request_id,
            source_id = %id,
            attachment_id = %attachment.id,
            error = %error,
            "AI source metadata was discarded; durable object cleanup remains queued"
        ),
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn source_context(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    conversation_id: Option<Uuid>,
    requested_project_id: Option<Uuid>,
    require_writable: bool,
) -> Result<Option<Uuid>, ApiError> {
    let conversation_project = if let Some(conversation_id) = conversation_id {
        let operations = state.ai_operations.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "ai_runtime_not_configured",
                "the AI runtime is not configured for this deployment",
            )
            .with_request_id(metadata.request_id.clone())
        })?;
        let conversation = store(operations.get_ai_conversation(conversation_id), metadata).await?;
        if conversation.lab_id != principal.lab_id || conversation.user_id != principal.user_id {
            return Err(ApiError::not_found("AI conversation was not found")
                .with_request_id(metadata.request_id.clone()));
        }
        if require_writable && conversation.archived_at.is_some() {
            return Err(ApiError::conflict("AI conversation is archived")
                .with_request_id(metadata.request_id.clone()));
        }
        if require_writable {
            if conversation.legacy_read_only {
                return Err(ApiError::conflict("legacy AI conversation is read-only")
                    .with_request_id(metadata.request_id.clone()));
            }
            let binding = conversation.model_profile.ok_or_else(|| {
                ApiError::conflict("legacy AI conversation is read-only")
                    .with_request_id(metadata.request_id.clone())
            })?;
            super::ai_api::ensure_conversation_model_available(state, principal, binding, metadata)
                .await?;
        }
        conversation.project_id
    } else {
        None
    };
    if conversation_id.is_some()
        && requested_project_id.is_some()
        && conversation_project != requested_project_id
    {
        return Err(ApiError::not_found("AI conversation was not found")
            .with_request_id(metadata.request_id.clone()));
    }
    let project_id = conversation_project.or(requested_project_id);
    if let Some(project_id) = project_id {
        scope::project_with_permission(
            state,
            principal,
            metadata,
            project_id,
            Permission::ReadAttachment,
        )
        .await?;
    }
    Ok(project_id)
}

async fn owned_source(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
) -> Result<AiConversationSource, ApiError> {
    let source = store(state.store.get_ai_conversation_source(id), metadata).await?;
    if source.lab_id != principal.lab_id || source.user_id != principal.user_id {
        return Err(ApiError::not_found("AI source was not found")
            .with_request_id(metadata.request_id.clone()));
    }
    Ok(source)
}

async fn classify_source(
    path: &Path,
    file_name: &str,
    declared_media_type: Option<&str>,
    inspection: AttachmentInspection,
) -> Result<(AiConversationSourceKind, String), ApiError> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(unsupported_source)?;
    let declared = declared_media_type
        .map(|value| value.split(';').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty());
    let (kind, canonical_media_type, allowed_media_types): (
        AiConversationSourceKind,
        &'static str,
        &'static [&'static str],
    ) = match extension.as_str() {
        "xlsx" if inspection.kind == AttachmentContentKind::Opaque && has_zip_magic(path).await => {
            (
                AiConversationSourceKind::Spreadsheet,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &[
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    "application/zip",
                ],
            )
        }
        "csv" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/csv",
            &["text/csv", "application/csv", "text/plain"],
        ),
        "tsv" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/tab-separated-values",
            &["text/tab-separated-values", "text/plain"],
        ),
        "txt" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "text/plain",
            &["text/plain"],
        ),
        "md" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "text/markdown",
            &["text/markdown", "text/plain"],
        ),
        "json" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "application/json",
            &["application/json", "text/json", "text/plain"],
        ),
        "pdf" if inspection.kind == AttachmentContentKind::Pdf => (
            AiConversationSourceKind::Pdf,
            "application/pdf",
            &["application/pdf"],
        ),
        "png" if inspection.kind == AttachmentContentKind::Png => {
            (AiConversationSourceKind::Image, "image/png", &["image/png"])
        }
        "jpg" | "jpeg" if inspection.kind == AttachmentContentKind::Jpeg => (
            AiConversationSourceKind::Image,
            "image/jpeg",
            &["image/jpeg"],
        ),
        "tif" | "tiff" if inspection.kind == AttachmentContentKind::Tiff => (
            AiConversationSourceKind::Image,
            "image/tiff",
            &["image/tiff"],
        ),
        _ => return Err(unsupported_source()),
    };
    if declared.is_some_and(|value| {
        value != "application/octet-stream" && !allowed_media_types.contains(&value)
    }) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "source_media_type_mismatch",
            "the declared media type does not match the AI source file extension",
        ));
    }
    Ok((
        kind,
        inspection
            .media_type
            .filter(|value| value != "application/octet-stream")
            .unwrap_or_else(|| canonical_media_type.to_owned()),
    ))
}

async fn has_zip_magic(path: &Path) -> bool {
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut prefix = [0_u8; 4];
    matches!(file.read(&mut prefix).await, Ok(4))
        && matches!(
            prefix,
            [b'P', b'K', 3, 4] | [b'P', b'K', 5, 6] | [b'P', b'K', 7, 8]
        )
}

fn unsupported_source() -> ApiError {
    ApiError::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_ai_source",
        "AI sources accept XLSX, CSV, TSV, TXT, MD, JSON, PDF, PNG, JPEG, and TIFF only",
    )
}

fn valid_file_name(value: String, metadata: &RequestMetadata) -> Result<String, ApiError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_FILE_NAME_BYTES
        || matches!(value.as_str(), "." | "..")
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(ApiError::validation("file_name is invalid")
            .with_request_id(metadata.request_id.clone()));
    }
    Ok(value)
}

fn valid_media_type(
    value: Option<String>,
    metadata: &RequestMetadata,
) -> Result<Option<String>, ApiError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| {
        value.len() > MAX_MEDIA_TYPE_BYTES
            || !value.is_ascii()
            || value.chars().any(char::is_control)
    }) {
        return Err(ApiError::validation("media_type is invalid")
            .with_request_id(metadata.request_id.clone()));
    }
    Ok(value)
}

fn ensure_human(principal: &AuthPrincipal, metadata: &RequestMetadata) -> Result<(), ApiError> {
    if principal.is_external_ai() {
        Err(ApiError::forbidden().with_request_id(metadata.request_id.clone()))
    } else {
        Ok(())
    }
}

fn attachment_root<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a Arc<std::path::PathBuf>, ApiError> {
    state
        .attachment_root
        .as_ref()
        .ok_or_else(|| ApiError::internal().with_request_id(metadata.request_id.clone()))
}

async fn cleanup_object(
    root: &Arc<std::path::PathBuf>,
    object: &StoredObject,
    attachment_id: Uuid,
    metadata: &RequestMetadata,
) {
    if let Err(error) = remove_installed_object(root.as_ref(), object).await {
        tracing::error!(
            request_id = %metadata.request_id,
            %attachment_id,
            error = %error,
            "failed to clean rejected AI source object"
        );
    }
}

fn upload_file_error(error: AttachmentFileError, metadata: &RequestMetadata) -> ApiError {
    match error {
        AttachmentFileError::TooLarge => payload_too_large(metadata),
        AttachmentFileError::AlreadyExists => ApiError::conflict("AI source object already exists")
            .with_request_id(metadata.request_id.clone()),
        _ => {
            tracing::error!(
                request_id = %metadata.request_id,
                error = %error,
                "AI source upload failed"
            );
            ApiError::internal().with_request_id(metadata.request_id.clone())
        }
    }
}

fn inspection_error(error: AttachmentInspectionError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        AttachmentInspectionError::ExecutableContent => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsafe_ai_source",
            "executable or script content is not accepted as an AI source",
        ),
        AttachmentInspectionError::SignatureMismatch => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_source_signature_mismatch",
            "the file extension, media type, and content signature do not agree",
        ),
        AttachmentInspectionError::ResourceLimit => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_source_resource_limit",
            "the image or document exceeds safe resource limits",
        ),
        AttachmentInspectionError::Io => {
            tracing::error!(
                request_id = %metadata.request_id,
                "AI source inspection failed"
            );
            return ApiError::internal().with_request_id(metadata.request_id.clone());
        }
    };
    error.with_request_id(metadata.request_id.clone())
}

fn payload_too_large(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!("AI source must not exceed {DEFAULT_MAX_UPLOAD_BYTES} bytes"),
    )
    .with_request_id(metadata.request_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_rejects_unlisted_extensions_before_persistence() {
        use axum::response::IntoResponse as _;

        assert_eq!(
            unsupported_source().into_response().status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }

    #[test]
    fn file_names_are_plain_names() {
        let metadata = RequestMetadata {
            request_id: "test".to_owned(),
            reason: None,
        };
        assert!(valid_file_name("source.csv".to_owned(), &metadata).is_ok());
        assert!(valid_file_name("../source.csv".to_owned(), &metadata).is_err());
    }

    #[test]
    fn source_view_hides_internal_attachment_identity_and_digest() {
        let now = Utc::now();
        let source_id = Uuid::new_v4();
        let attachment_id = Uuid::new_v4();
        let source = AiConversationSource {
            id: source_id,
            lab_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            conversation_id: Some(Uuid::new_v4()),
            project_id: Some(Uuid::new_v4()),
            attachment_id,
            kind: AiConversationSourceKind::Text,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + Duration::days(30),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        };
        let attachment = Attachment {
            id: attachment_id,
            lab_id: source.lab_id,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source_id,
            file_name: "notes.txt".to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: "private/path".to_owned(),
            size_bytes: 12,
            sha256: "a".repeat(64),
            version: 1,
            meta: RecordMeta::new(now),
        };

        let value = serde_json::to_value(AiSourceView::new(source, attachment)).unwrap();
        assert!(value.get("attachmentId").is_none());
        assert!(value.get("sha256").is_none());
        assert!(value.get("relativePath").is_none());
    }
}
