use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse,
    ai_api::provider_api_error,
    attachment_files::{open_verified, remove_installed_object},
    collection, item, scope, store,
};
use crate::{
    ApiError, AppState, AuthPrincipal, AuthenticationMethod, RequestMetadata, ResolvedAiProvider,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{delete, get, post},
};
use chrono::{Duration, Utc};
use futures_util::TryStreamExt;
use muriarc_ai::{
    DataCellVisionCandidate, DataCellVisionExtractionError, DataCellVisionExtractionRequest,
    MAX_SANITIZED_VISION_INPUT_BYTES, MAX_VISION_TOTAL_BASE64_BYTES, PreparedAssistantImage,
    ProviderCredentials, extract_data_cell_vision, sanitize_vision_input,
};
use muriarc_core::{
    AiExtractionApprovalInput, AiExtractionApprovalSelection, AiExtractionDraft,
    AiExtractionEvidence, AiExtractionItem, AiExtractionModelTrace, AiExtractionRejectionInput,
    AiExtractionStatus, AiModelPurpose, AiObservationDataCell, AppliedAiExtraction, Attachment,
    AttachmentDerivative, AuditAction, DerivativeKind, DerivativeStatus, EntityType, Observation,
    ObservationSubjectType, ObservationValueData, ObservationValueRecord, ParticipationFilter,
    Permission, PrivateAiImage, PrivateImageFilter, PrivateImageStats, PrivateImageStatus,
    RecordMeta, StoreError,
};
use muriarc_data::{
    AttachmentContentKind, AttachmentFileError, AttachmentFiles, AttachmentInspectionError,
    inspect_attachment,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::Arc};
use tokio_util::io::{ReaderStream, StreamReader};
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    let upload = Router::new()
        .route("/ai/images/upload", post(upload))
        .layer(DefaultBodyLimit::disable());
    upload.merge(
        Router::new()
            .route("/ai/images", get(list_own))
            .route("/ai/images/{id}/content", get(image_content))
            .route("/ai/images/{id}/archive", post(archive))
            .route("/admin/ai/images/stats", get(stats))
            .route("/admin/ai/images", get(list_admin))
            .route("/admin/ai/images/users/{id}/enter", post(enter_admin_view))
            .route("/admin/ai/images/users/{id}/exit", delete(exit_admin_view))
            .route(
                "/ai/extractions",
                get(list_extractions).post(create_extraction),
            )
            .route("/ai/extractions/{id}", get(get_extraction))
            .route("/ai/extractions/{id}/approve", post(approve_extraction))
            .route("/ai/extractions/{id}/reject", post(reject_extraction)),
    )
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivateImageView {
    image: PrivateAiImage,
    file_name: String,
    media_type: Option<String>,
    size_bytes: i64,
    sha256: String,
    content_href: String,
    preview_href: String,
    retention_days: i64,
}
async fn image_view(
    state: &AppState,
    m: &RequestMetadata,
    image: PrivateAiImage,
) -> Result<PrivateImageView, ApiError> {
    let a = store(state.store.get_attachment(image.attachment_id), m).await?;
    Ok(PrivateImageView {
        content_href: format!("/api/v1/ai/images/{}/content", image.id),
        preview_href: format!("/api/v1/ai/images/{}/content", image.id),
        file_name: a.file_name,
        media_type: a.media_type,
        size_bytes: a.size_bytes,
        sha256: a.sha256,
        retention_days: (image.expires_at - Utc::now()).num_days().max(0),
        image,
    })
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    file_name: String,
    media_type: Option<String>,
    conversation_id: Option<Uuid>,
}
async fn upload(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiQuery(q): ApiQuery<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<ItemResponse<PrivateImageView>>), ApiError> {
    ensure_human(&p, &m)?;
    if let Some(value) = headers.get(header::CONTENT_LENGTH) {
        let size = value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                ApiError::validation("Content-Length is invalid")
                    .with_request_id(m.request_id.clone())
            })?;
        if size > MAX_SANITIZED_VISION_INPUT_BYTES as u64 {
            return Err(image_upload_too_large(&m));
        }
        if size == 0 {
            return Err(image_upload_invalid(&m));
        }
    }
    let file_name = valid_file_name(q.file_name, &m)?;
    preflight_upload_conversation(&state, &p, &m, q.conversation_id).await?;
    let declared = q.media_type.or_else(|| {
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .filter(|v| *v != "application/octet-stream")
            .map(str::to_owned)
    });
    let root = attachment_root(&state, &m)?;
    let attachment_id = Uuid::new_v4();
    let stream = body
        .into_data_stream()
        .map_err(|error| io::Error::other(error.to_string()));
    let files = AttachmentFiles::with_limit(root.as_ref(), MAX_SANITIZED_VISION_INPUT_BYTES as u64);
    let object = files
        .write_reader(attachment_id, StreamReader::new(stream))
        .await
        .map_err(|error| image_upload_storage_error(error, &m))?;
    if object.size_bytes == 0 {
        remove_installed_object(root.as_ref(), &object).await.ok();
        return Err(image_upload_invalid(&m));
    }
    let inspection =
        match inspect_attachment(&object.absolute_path, &file_name, declared.as_deref()).await {
            Ok(v)
                if matches!(
                    v.kind,
                    AttachmentContentKind::Jpeg
                        | AttachmentContentKind::Png
                        | AttachmentContentKind::Webp
                        | AttachmentContentKind::Gif
                ) =>
            {
                v
            }
            Ok(_) => {
                remove_installed_object(root.as_ref(), &object).await.ok();
                return Err(ApiError::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "image_media_type_unsupported",
                    "private AI space accepts JPEG, PNG, WebP, or GIF images only",
                )
                .with_request_id(m.request_id));
            }
            Err(e) => {
                remove_installed_object(root.as_ref(), &object).await.ok();
                return Err(inspection_error(e, &m));
            }
        };
    let original = match tokio::fs::read(&object.absolute_path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            remove_installed_object(root.as_ref(), &object).await.ok();
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_upload_failed",
                "the uploaded image could not be validated",
            )
            .with_request_id(m.request_id));
        }
    };
    let media_type = inspection
        .media_type
        .as_deref()
        .ok_or_else(|| image_upload_invalid(&m))?;
    if sanitize_vision_input(media_type, &original).is_err() {
        remove_installed_object(root.as_ref(), &object).await.ok();
        return Err(image_upload_invalid(&m));
    }
    let now = Utc::now();
    let image_id = Uuid::new_v4();
    let a = Attachment {
        id: attachment_id,
        lab_id: p.lab_id,
        project_id: None,
        entity_type: "ai_private_image".into(),
        entity_id: image_id,
        file_name,
        media_type: inspection.media_type,
        relative_path: object.relative_path.clone(),
        size_bytes: object.size_bytes,
        sha256: object.sha256.clone(),
        version: 1,
        meta: RecordMeta::new(now),
    };
    let image = PrivateAiImage {
        id: image_id,
        lab_id: p.lab_id,
        user_id: p.user_id,
        conversation_id: q.conversation_id,
        attachment_id,
        project_id: None,
        status: PrivateImageStatus::Active,
        last_activity_at: now,
        expires_at: now + Duration::days(30),
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    if let Err(e) = state
        .store
        .create_private_ai_image(&a, &image, &p.audit_context(&m))
        .await
    {
        remove_installed_object(root.as_ref(), &object).await.ok();
        return Err(ApiError::from_store(e).with_request_id(m.request_id));
    }
    let view = image_view(&state, &m, image).await?;
    Ok((StatusCode::CREATED, item(view, &m)))
}

async fn preflight_upload_conversation(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    conversation_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(conversation_id) = conversation_id else {
        return Ok(());
    };
    let operations = state.ai_operations.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ai_runtime_not_configured",
            "the AI runtime is not configured for this deployment",
        )
        .with_request_id(metadata.request_id.clone())
    })?;
    let conversation = match operations.get_ai_conversation(conversation_id).await {
        Ok(conversation) => conversation,
        Err(StoreError::NotFound { .. }) => {
            return Err(ApiError::not_found("AI conversation was not found")
                .with_request_id(metadata.request_id.clone()));
        }
        Err(error) => {
            return Err(ApiError::from_store(error).with_request_id(metadata.request_id.clone()));
        }
    };
    if conversation.lab_id != principal.lab_id || conversation.user_id != principal.user_id {
        return Err(ApiError::not_found("AI conversation was not found")
            .with_request_id(metadata.request_id.clone()));
    }
    if conversation.archived_at.is_some() {
        return Err(ApiError::conflict("AI conversation is archived")
            .with_request_id(metadata.request_id.clone()));
    }
    if conversation.legacy_read_only {
        return Err(ApiError::conflict("legacy AI conversation is read-only")
            .with_request_id(metadata.request_id.clone()));
    }
    let binding = conversation.model_profile.ok_or_else(|| {
        ApiError::conflict("legacy AI conversation is read-only")
            .with_request_id(metadata.request_id.clone())
    })?;
    super::ai_api::ensure_conversation_model_available(state, principal, binding, metadata).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageListQuery {
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
    status: Option<PrivateImageStatus>,
}
async fn list_own(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiQuery(q): ApiQuery<ImageListQuery>,
) -> Result<Json<CollectionResponse<PrivateImageView>>, ApiError> {
    ensure_human(&p, &m)?;
    let images = store(
        state.store.list_private_ai_images(&PrivateImageFilter {
            lab_id: p.lab_id,
            user_id: Some(p.user_id),
            conversation_id: q.conversation_id,
            project_id: q.project_id,
            status: q.status,
        }),
        &m,
    )
    .await?;
    let mut views = Vec::new();
    for image in images {
        views.push(image_view(&state, &m, image).await?)
    }
    Ok(collection(views, &m))
}
async fn image_content(
    State(state): State<AppState>,
    p: AuthPrincipal,
    auth: AuthenticationMethod,
    m: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    ensure_human(&p, &m)?;
    let image = store(state.store.get_private_ai_image(id), &m).await?;
    if image.lab_id != p.lab_id {
        return Err(
            ApiError::not_found("private image was not found").with_request_id(m.request_id)
        );
    }
    if image.user_id != p.user_id {
        ensure_admin(&p, &m)?;
        let AuthenticationMethod::Session { session_id } = auth else {
            return Err(ApiError::forbidden().with_request_id(m.request_id));
        };
        if !state
            .admin_private_views
            .read()
            .await
            .contains(&(session_id, image.user_id))
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "admin_private_view_required",
                "enter administrator private-space view first",
            )
            .with_request_id(m.request_id));
        }
    }
    ensure_private_image_readable(image.status, &m)?;
    let a = store(state.store.get_attachment(image.attachment_id), &m).await?;
    let object = open_verified(attachment_root(&state, &m)?.as_ref(), &a)
        .await
        .map_err(|_| {
            ApiError::conflict("image integrity verification failed")
                .with_request_id(m.request_id.clone())
        })?;
    let mut r = Response::new(Body::from_stream(ReaderStream::new(object.file)));
    *r.status_mut() = StatusCode::OK;
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&object.size_bytes.to_string())
            .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?,
    );
    h.insert(
        header::CONTENT_TYPE,
        a.media_type
            .as_deref()
            .and_then(|v| HeaderValue::from_str(v).ok())
            .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; img-src 'self' data:"),
    );
    Ok(r)
}

fn ensure_private_image_readable(
    status: PrivateImageStatus,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    match status {
        PrivateImageStatus::Active
        | PrivateImageStatus::PendingApproval
        | PrivateImageStatus::Archived => Ok(()),
        PrivateImageStatus::Expired => Err(ApiError::new(
            StatusCode::GONE,
            "image_expired",
            "private image retention expired",
        )
        .with_request_id(metadata.request_id.clone())),
        PrivateImageStatus::Processing | PrivateImageStatus::Failed => Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "image_unavailable",
            "private image content is not available in its current state",
        )
        .with_request_id(metadata.request_id.clone())),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveInput {
    project_id: Uuid,
    expected_revision: i64,
}
async fn archive(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(q): ApiJson<ArchiveInput>,
) -> Result<Json<ItemResponse<PrivateImageView>>, ApiError> {
    ensure_human(&p, &m)?;
    let current = store(state.store.get_private_ai_image(id), &m).await?;
    if current.lab_id != p.lab_id || current.user_id != p.user_id {
        return Err(
            ApiError::not_found("private image was not found").with_request_id(m.request_id)
        );
    }
    preflight_upload_conversation(&state, &p, &m, current.conversation_id).await?;
    scope::project_with_permission(&state, &p, &m, q.project_id, Permission::WriteAttachment)
        .await?;
    let image = store(
        state.store.archive_private_ai_image(
            id,
            q.project_id,
            q.expected_revision,
            Utc::now(),
            &p.audit_context(&m),
        ),
        &m,
    )
    .await?;
    Ok(item(image_view(&state, &m, image).await?, &m))
}
async fn stats(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
) -> Result<Json<CollectionResponse<PrivateImageStats>>, ApiError> {
    ensure_admin(&p, &m)?;
    let v = store(state.store.private_ai_image_stats(p.lab_id, Utc::now()), &m).await?;
    Ok(collection(v, &m))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminListQuery {
    user_id: Uuid,
}
async fn list_admin(
    State(state): State<AppState>,
    p: AuthPrincipal,
    auth: AuthenticationMethod,
    m: RequestMetadata,
    ApiQuery(q): ApiQuery<AdminListQuery>,
) -> Result<Json<CollectionResponse<PrivateImageView>>, ApiError> {
    ensure_admin(&p, &m)?;
    let AuthenticationMethod::Session { session_id } = auth else {
        return Err(ApiError::forbidden().with_request_id(m.request_id));
    };
    if !state
        .admin_private_views
        .read()
        .await
        .contains(&(session_id, q.user_id))
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "admin_private_view_required",
            "enter administrator private-space view first",
        )
        .with_request_id(m.request_id));
    }
    let images = store(
        state.store.list_private_ai_images(&PrivateImageFilter {
            lab_id: p.lab_id,
            user_id: Some(q.user_id),
            ..PrivateImageFilter::default()
        }),
        &m,
    )
    .await?;
    let mut v = Vec::new();
    for image in images {
        v.push(image_view(&state, &m, image).await?)
    }
    Ok(collection(v, &m))
}
async fn enter_admin_view(
    State(state): State<AppState>,
    p: AuthPrincipal,
    auth: AuthenticationMethod,
    m: RequestMetadata,
    ApiPath(user): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_admin(&p, &m)?;
    let AuthenticationMethod::Session { session_id } = auth else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "session_required",
            "browser session required",
        )
        .with_request_id(m.request_id));
    };
    let images = store(
        state.store.list_private_ai_images(&PrivateImageFilter {
            lab_id: p.lab_id,
            user_id: Some(user),
            ..PrivateImageFilter::default()
        }),
        &m,
    )
    .await?;
    state
        .admin_private_views
        .write()
        .await
        .insert((session_id, user));
    store(
        state.store.record_workspace_operation(
            muriarc_core::WorkspaceOperationInput {
                lab_id: p.lab_id,
                project_id: None,
                entity_type: EntityType::User,
                entity_id: user,
                action: AuditAction::EnterAdminView,
                before: None,
                after: Some(serde_json::json!({"target_user_id":user,"image_count":images.len()})),
            },
            &p.audit_context(&m),
        ),
        &m,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn exit_admin_view(
    State(state): State<AppState>,
    p: AuthPrincipal,
    auth: AuthenticationMethod,
    m: RequestMetadata,
    ApiPath(user): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_admin(&p, &m)?;
    let AuthenticationMethod::Session { session_id } = auth else {
        return Err(ApiError::forbidden().with_request_id(m.request_id));
    };
    state
        .admin_private_views
        .write()
        .await
        .remove(&(session_id, user));
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn prepare_assistant_images(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
    image_ids: &[Uuid],
) -> Result<Vec<PreparedAssistantImage>, ApiError> {
    Ok(prepare_private_images(
        state,
        principal,
        metadata,
        conversation_id,
        project_id,
        image_ids,
    )
    .await?
    .into_iter()
    .map(|image| image.provider)
    .collect())
}

#[derive(Debug)]
struct PreparedPrivateImage {
    provider: PreparedAssistantImage,
    image: PrivateAiImage,
    attachment: Attachment,
}

async fn prepare_private_images(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    conversation_id: Option<Uuid>,
    project_id: Option<Uuid>,
    image_ids: &[Uuid],
) -> Result<Vec<PreparedPrivateImage>, ApiError> {
    if image_ids.is_empty()
        || image_ids.len() > muriarc_ai::MAX_VISION_IMAGES
        || image_ids.iter().any(Uuid::is_nil)
        || image_ids.iter().copied().collect::<BTreeSet<_>>().len() != image_ids.len()
    {
        return Err(image_evidence_invalid(metadata));
    }
    let mut prepared = Vec::with_capacity(image_ids.len());
    let mut total_base64_bytes = 0_usize;
    for image_id in image_ids {
        let image = state
            .store
            .get_private_ai_image(*image_id)
            .await
            .map_err(|error| image_evidence_store_error(error, metadata))?;
        if image.lab_id != principal.lab_id
            || image.user_id != principal.user_id
            || image.status != PrivateImageStatus::Active
            || image
                .conversation_id
                .is_some_and(|bound| Some(bound) != conversation_id)
            || image
                .project_id
                .is_some_and(|bound| Some(bound) != project_id)
        {
            return Err(image_evidence_invalid(metadata));
        }
        let attachment = state
            .store
            .get_attachment(image.attachment_id)
            .await
            .map_err(|error| image_evidence_store_error(error, metadata))?;
        if attachment.lab_id != principal.lab_id
            || attachment.entity_type != "ai_private_image"
            || attachment.entity_id != image.id
        {
            return Err(image_evidence_invalid(metadata));
        }
        let media_type = attachment
            .media_type
            .as_deref()
            .ok_or_else(|| image_evidence_invalid(metadata))?;
        let bytes = ai_input_copy(state, principal, metadata, &attachment).await?;
        let sanitized = sanitize_vision_input(media_type, &bytes)
            .map_err(|_| image_evidence_invalid(metadata))?;
        let provider = sanitized
            .prepared_image(image.id)
            .map_err(|_| image_evidence_invalid(metadata))?;
        total_base64_bytes = total_base64_bytes
            .checked_add(provider.provider_input().data_base64.len())
            .ok_or_else(|| image_evidence_invalid(metadata))?;
        if total_base64_bytes > MAX_VISION_TOTAL_BASE64_BYTES {
            return Err(image_evidence_invalid(metadata));
        }
        prepared.push(PreparedPrivateImage {
            provider,
            image,
            attachment,
        });
    }
    Ok(prepared)
}

fn image_evidence_store_error(error: StoreError, metadata: &RequestMetadata) -> ApiError {
    match error {
        StoreError::NotFound { .. } => image_evidence_invalid(metadata),
        error => ApiError::from_store(error).with_request_id(metadata.request_id.clone()),
    }
}

fn image_evidence_invalid(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "image_evidence_invalid",
        "one or more images are unavailable, unsafe, or outside the current conversation scope",
    )
    .with_request_id(metadata.request_id.clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateExtractionInput {
    #[serde(default)]
    image_ids: Vec<Uuid>,
    #[serde(default, alias = "private_image_id")]
    private_image_id: Option<Uuid>,
    project_id: Uuid,
    experiment_id: Uuid,
    experiment_event_id: Uuid,
    current_data_cell: ExtractionDataCellInput,
    #[serde(default)]
    vision_model_profile_id: Option<Uuid>,
}

impl CreateExtractionInput {
    fn normalized_image_ids(&self) -> Option<Vec<Uuid>> {
        match (self.image_ids.is_empty(), self.private_image_id) {
            (false, None) => Some(self.image_ids.clone()),
            (true, Some(image_id)) => Some(vec![image_id]),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtractionDataCellInput {
    definition_id: Uuid,
    subject_type: ObservationSubjectType,
    subject_id: Uuid,
}

#[allow(clippy::too_many_arguments)]
fn build_extraction_item(
    candidate: DataCellVisionCandidate,
    lab_id: Uuid,
    user_id: Uuid,
    project_id: Uuid,
    experiment_id: Uuid,
    experiment_event_id: Uuid,
    data_cell: &AiObservationDataCell,
    now: chrono::DateTime<Utc>,
    metadata: &RequestMetadata,
) -> Result<AiExtractionItem, ApiError> {
    let (candidate_value, confidence, source_label) = candidate.into_parts();
    let observation = Observation::new(
        lab_id,
        project_id,
        experiment_id,
        experiment_event_id,
        data_cell.definition_id,
        data_cell.subject_type,
        data_cell.subject_id,
        now,
    )
    .map_err(|_| invalid_vision("invalid observation", metadata))?;
    let mut value = ObservationValueRecord::new(observation.id, 1, candidate_value, now, now)
        .map_err(|_| invalid_vision("invalid value", metadata))?;
    value.recorded_by = Some(user_id);
    value.notes = Some("AI visual extraction; pending human approval".into());
    let item = AiExtractionItem {
        observation,
        value,
        confidence,
        selected: false,
        source_label,
    };
    item.validate()
        .map_err(|_| invalid_vision("candidate is invalid", metadata))?;
    Ok(item)
}

async fn validate_extraction_data_cell_scope(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    project_id: Uuid,
    experiment_id: Uuid,
    cell: &AiObservationDataCell,
) -> Result<(), ApiError> {
    let valid = match cell.subject_type {
        ObservationSubjectType::Experiment => cell.subject_id == experiment_id,
        ObservationSubjectType::Animal => !state
            .store
            .list_participations(&ParticipationFilter {
                project_id,
                experiment_id: Some(experiment_id),
                animal_id: Some(cell.subject_id),
                cohort_id: None,
            })
            .await
            .map_err(|error| {
                ApiError::from_store(error).with_request_id(metadata.request_id.clone())
            })?
            .is_empty(),
        ObservationSubjectType::Sample => {
            let sample = state
                .store
                .get_sample(cell.subject_id)
                .await
                .map_err(|error| data_cell_store_error(error, metadata))?;
            (sample.lab_id, sample.project_id, sample.experiment_id)
                == (principal.lab_id, project_id, Some(experiment_id))
        }
        ObservationSubjectType::Artifact => {
            let attachment = state
                .store
                .get_attachment(cell.subject_id)
                .await
                .map_err(|error| data_cell_store_error(error, metadata))?;
            (attachment.lab_id, attachment.project_id) == (principal.lab_id, Some(project_id))
        }
    };
    if valid {
        Ok(())
    } else {
        Err(data_cell_invalid(metadata))
    }
}

fn data_cell_store_error(error: StoreError, metadata: &RequestMetadata) -> ApiError {
    match error {
        StoreError::NotFound { .. } => data_cell_invalid(metadata),
        error => ApiError::from_store(error).with_request_id(metadata.request_id.clone()),
    }
}

fn data_cell_invalid(metadata: &RequestMetadata) -> ApiError {
    ApiError::validation("currentDataCell is outside the selected experiment scope")
        .with_request_id(metadata.request_id.clone())
}

async fn create_extraction(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiJson(q): ApiJson<CreateExtractionInput>,
) -> Result<(StatusCode, Json<ItemResponse<AiExtractionDraft>>), ApiError> {
    ensure_human(&p, &m)?;
    let image_ids = q
        .normalized_image_ids()
        .ok_or_else(|| image_evidence_invalid(&m))?;
    let data_cell = AiObservationDataCell {
        definition_id: q.current_data_cell.definition_id,
        subject_type: q.current_data_cell.subject_type,
        subject_id: q.current_data_cell.subject_id,
    };
    data_cell.validate().map_err(|_| {
        ApiError::validation("currentDataCell is invalid").with_request_id(m.request_id.clone())
    })?;
    let experiment = scope::experiment_with_permission(
        &state,
        &p,
        &m,
        q.experiment_id,
        Permission::WriteMeasurementDraft,
    )
    .await?;
    if experiment.project_id != q.project_id {
        return Err(ApiError::not_found("experiment was not found").with_request_id(m.request_id));
    }
    let event = store(state.store.get_experiment_event(q.experiment_event_id), &m).await?;
    if (event.lab_id, event.project_id, event.experiment_id)
        != (p.lab_id, q.project_id, q.experiment_id)
    {
        return Err(
            ApiError::not_found("experiment event was not found").with_request_id(m.request_id)
        );
    }
    let defs = store(
        state.store.list_observation_definitions(q.experiment_id),
        &m,
    )
    .await?;
    let definition = defs
        .iter()
        .find(|definition| definition.id == data_cell.definition_id)
        .ok_or_else(|| {
            ApiError::validation("currentDataCell references an unknown observation definition")
                .with_request_id(m.request_id.clone())
        })?;
    if data_cell.subject_type == ObservationSubjectType::Experiment
        && data_cell.subject_id != q.experiment_id
    {
        return Err(
            ApiError::validation("currentDataCell experiment subject is invalid")
                .with_request_id(m.request_id),
        );
    }
    validate_extraction_data_cell_scope(&state, &p, &m, q.project_id, q.experiment_id, &data_cell)
        .await?;
    let images =
        prepare_private_images(&state, &p, &m, None, Some(q.project_id), &image_ids).await?;
    let resolved =
        super::ai_api::resolve_turn_vision_provider(&state, &p, q.vision_model_profile_id, &m)
            .await?;
    let ResolvedAiProvider {
        provider,
        api_key,
        runtime,
        model_profile,
        supports_vision,
    } = resolved;
    if !supports_vision {
        return Err(super::ai_api::vision_model_unavailable(&m));
    }
    let credentials = match api_key.as_ref() {
        Some(s) => ProviderCredentials::bearer(s.as_str())
            .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?,
        None => ProviderCredentials::none(),
    };
    let provider_images = images
        .iter()
        .map(|image| image.provider.clone())
        .collect::<Vec<_>>();
    let extraction = extract_data_cell_vision(
        &provider,
        credentials,
        DataCellVisionExtractionRequest {
            model_profile,
            runtime,
            definition,
            images: &provider_images,
        },
    )
    .await
    .map_err(|error| data_cell_extraction_error(error, &m))?;
    let muriarc_ai::DataCellVisionExtraction {
        candidate,
        model_profile,
        provider_id,
        model,
        provider_request_id,
        usage,
        estimated_input_tokens,
    } = extraction;
    let now = Utc::now();
    let candidate_item = build_extraction_item(
        candidate,
        p.lab_id,
        p.user_id,
        q.project_id,
        q.experiment_id,
        q.experiment_event_id,
        &data_cell,
        now,
        &m,
    )?;
    let items = vec![candidate_item];
    let evidence = images
        .iter()
        .enumerate()
        .map(|(index, image)| AiExtractionEvidence {
            display_order: i32::try_from(index).unwrap_or(i32::MAX),
            private_image_id: image.image.id,
            private_attachment_id: image.attachment.id,
            promoted_attachment_id: None,
            original_sha256: image.attachment.sha256.clone(),
            sanitized_sha256: image.provider.evidence().sanitized_sha256.clone(),
            meta: RecordMeta::new(now),
        })
        .collect::<Vec<_>>();
    let first_evidence = evidence.first().ok_or_else(|| image_evidence_invalid(&m))?;
    let draft = AiExtractionDraft {
        id: Uuid::new_v4(),
        lab_id: p.lab_id,
        user_id: p.user_id,
        project_id: q.project_id,
        experiment_id: q.experiment_id,
        experiment_event_id: q.experiment_event_id,
        private_image_id: first_evidence.private_image_id,
        attachment_id: first_evidence.private_attachment_id,
        image_sha256: first_evidence.original_sha256.clone(),
        provider: provider_id,
        model,
        tool_run_id: None,
        data_cell: Some(data_cell),
        evidence,
        model_trace: Some(AiExtractionModelTrace {
            profile_id: model_profile.profile_id,
            profile_version: model_profile.profile_version,
            purpose: AiModelPurpose::Vision,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            provider_request_id,
            trace: serde_json::json!({
                "purpose": "vision",
                "imageCount": image_ids.len(),
                "estimatedInputTokens": estimated_input_tokens,
                "inputTokenCountIsEstimate": true,
            }),
        }),
        status: AiExtractionStatus::PendingApproval,
        items,
        error_code: None,
        meta: RecordMeta::new(now),
    };
    store(
        state
            .store
            .create_ai_extraction_draft(&draft, &p.audit_context(&m)),
        &m,
    )
    .await?;
    Ok((StatusCode::CREATED, item(draft, &m)))
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractionListQuery {
    project_id: Option<Uuid>,
}
async fn list_extractions(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiQuery(q): ApiQuery<ExtractionListQuery>,
) -> Result<Json<CollectionResponse<AiExtractionDraft>>, ApiError> {
    ensure_human(&p, &m)?;
    if let Some(project) = q.project_id {
        scope::project_with_permission(&state, &p, &m, project, Permission::ReadMeasurement)
            .await?;
    }
    let v = store(
        state
            .store
            .list_ai_extraction_drafts(p.lab_id, p.user_id, q.project_id),
        &m,
    )
    .await?;
    Ok(collection(v, &m))
}
async fn get_extraction(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AiExtractionDraft>>, ApiError> {
    ensure_human(&p, &m)?;
    let d = store(state.store.get_ai_extraction_draft(id), &m).await?;
    if d.lab_id != p.lab_id || d.user_id != p.user_id {
        return Err(
            ApiError::not_found("extraction draft was not found").with_request_id(m.request_id)
        );
    }
    scope::project_with_permission(&state, &p, &m, d.project_id, Permission::ReadMeasurement)
        .await?;
    Ok(item(d, &m))
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApproveInput {
    expected_revision: i64,
    #[serde(default)]
    selections: Vec<ApproveSelectionInput>,
    #[serde(default, alias = "selected_indexes")]
    selected_indexes: Vec<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApproveSelectionInput {
    item_index: usize,
    value: ObservationValueData,
    notes: Option<String>,
}
async fn approve_extraction(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(q): ApiJson<ApproveInput>,
) -> Result<Json<ItemResponse<AppliedAiExtraction>>, ApiError> {
    ensure_human(&p, &m)?;
    let d = store(state.store.get_ai_extraction_draft(id), &m).await?;
    if d.lab_id != p.lab_id || d.user_id != p.user_id {
        return Err(
            ApiError::not_found("extraction draft was not found").with_request_id(m.request_id)
        );
    }
    for permission in [
        Permission::WriteMeasurementDraft,
        Permission::SignMeasurement,
        Permission::WriteAttachment,
    ] {
        scope::project_with_permission(&state, &p, &m, d.project_id, permission).await?;
    }
    let selections = match (q.selections.is_empty(), q.selected_indexes.is_empty()) {
        (false, true) => q
            .selections
            .into_iter()
            .map(|selection| AiExtractionApprovalSelection {
                item_index: selection.item_index,
                value: selection.value,
                notes: selection.notes,
            })
            .collect(),
        // Compatibility for already-persisted legacy drafts only. Versioned
        // data-cell drafts require an explicit human-edited value payload.
        (true, false) if d.data_cell.is_none() => q
            .selected_indexes
            .into_iter()
            .map(|item_index| {
                let item = d.items.get(item_index).ok_or_else(|| {
                    ApiError::validation("selected extraction item is out of range")
                        .with_request_id(m.request_id.clone())
                })?;
                Ok(AiExtractionApprovalSelection {
                    item_index,
                    value: item.value.value.clone(),
                    notes: item.value.notes.clone(),
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?,
        _ => {
            return Err(ApiError::validation(
                "provide exactly one extraction approval selection format",
            )
            .with_request_id(m.request_id));
        }
    };
    let approval = AiExtractionApprovalInput {
        expected_revision: q.expected_revision,
        selections,
    };
    approval.validate().map_err(|_| {
        ApiError::validation("extraction approval is invalid").with_request_id(m.request_id.clone())
    })?;
    let v = store(
        state
            .store
            .apply_ai_extraction_draft(id, &approval, &p.audit_context(&m)),
        &m,
    )
    .await?;
    Ok(item(v, &m))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RejectInput {
    expected_revision: i64,
}

async fn reject_extraction(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(q): ApiJson<RejectInput>,
) -> Result<Json<ItemResponse<AiExtractionDraft>>, ApiError> {
    ensure_human(&p, &m)?;
    let draft = store(state.store.get_ai_extraction_draft(id), &m).await?;
    if draft.lab_id != p.lab_id || draft.user_id != p.user_id {
        return Err(
            ApiError::not_found("extraction draft was not found").with_request_id(m.request_id)
        );
    }
    scope::project_with_permission(
        &state,
        &p,
        &m,
        draft.project_id,
        Permission::WriteMeasurementDraft,
    )
    .await?;
    let rejection = AiExtractionRejectionInput {
        expected_revision: q.expected_revision,
    };
    rejection.validate().map_err(|_| {
        ApiError::validation("extraction rejection is invalid")
            .with_request_id(m.request_id.clone())
    })?;
    let rejected = store(
        state
            .store
            .reject_ai_extraction_draft(id, &rejection, &p.audit_context(&m)),
        &m,
    )
    .await?;
    Ok(item(rejected, &m))
}

async fn ai_input_copy(
    state: &AppState,
    p: &AuthPrincipal,
    m: &RequestMetadata,
    a: &Attachment,
) -> Result<Vec<u8>, ApiError> {
    let files = AttachmentFiles::new(attachment_root(state, m)?.as_ref());
    if let Some(d) = store(state.store.list_attachment_derivatives(a.id), m)
        .await?
        .into_iter()
        .find(|d| d.kind == DerivativeKind::AiInput && d.status == DerivativeStatus::Ready)
    {
        let surrogate = Attachment {
            id: d.id,
            lab_id: d.lab_id,
            project_id: d.project_id,
            entity_type: "attachment_derivative".into(),
            entity_id: a.id,
            file_name: format!("ai-input-{}", a.file_name),
            media_type: d.media_type,
            relative_path: d
                .relative_path
                .ok_or_else(|| ApiError::internal().with_request_id(m.request_id.clone()))?,
            size_bytes: d
                .size_bytes
                .ok_or_else(|| ApiError::internal().with_request_id(m.request_id.clone()))?,
            sha256: d
                .sha256
                .ok_or_else(|| ApiError::internal().with_request_id(m.request_id.clone()))?,
            version: 1,
            meta: d.meta,
        };
        return files.read_verified_bytes(&surrogate).await.map_err(|_| {
            ApiError::conflict("AI input derivative integrity failed")
                .with_request_id(m.request_id.clone())
        });
    }
    let original = files.read_verified_bytes(a).await.map_err(|_| {
        ApiError::conflict("source image integrity failed").with_request_id(m.request_id.clone())
    })?;
    let media_type = a
        .media_type
        .as_deref()
        .ok_or_else(|| image_evidence_invalid(m))?;
    let prepared = sanitize_vision_input(media_type, &original)
        .map_err(|_| image_evidence_invalid(m))?
        .bytes()
        .to_vec();
    debug_assert!(prepared.len() <= MAX_SANITIZED_VISION_INPUT_BYTES);
    let id = Uuid::new_v4();
    let object = files
        .write_bytes(id, &prepared)
        .await
        .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?;
    let d = AttachmentDerivative {
        id,
        lab_id: p.lab_id,
        // A derived copy of a private image is not project data until the
        // separate human approval transaction promotes the evidence.
        project_id: None,
        attachment_id: a.id,
        kind: DerivativeKind::AiInput,
        media_type: a.media_type.clone(),
        relative_path: Some(object.relative_path.clone()),
        size_bytes: Some(object.size_bytes),
        sha256: Some(object.sha256.clone()),
        status: DerivativeStatus::Ready,
        error_code: None,
        meta: RecordMeta::new(Utc::now()),
    };
    if let Err(e) = state
        .store
        .create_attachment_derivative(&d, &p.audit_context(m))
        .await
    {
        files.remove_installed_object(&object).await.ok();
        return Err(ApiError::from_store(e).with_request_id(m.request_id.clone()));
    }
    Ok(prepared)
}
fn data_cell_extraction_error(
    error: DataCellVisionExtractionError,
    metadata: &RequestMetadata,
) -> ApiError {
    match error {
        DataCellVisionExtractionError::Provider(error) => provider_api_error(error, metadata),
        DataCellVisionExtractionError::ContextExceeded {
            estimated_input_tokens,
            max_input_tokens,
        } => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "context_exceeded",
            "the image and extraction prompt exceed the configured input token budget",
        )
        .with_details(serde_json::json!({
            "estimatedInputTokens": estimated_input_tokens,
            "inputTokenCountIsEstimate": true,
            "maxInputTokens": max_input_tokens,
            "currentInputTruncated": false,
        }))
        .with_request_id(metadata.request_id.clone()),
        DataCellVisionExtractionError::InvalidImageEvidence => image_evidence_invalid(metadata),
        DataCellVisionExtractionError::InvalidResponse => invalid_vision(
            "provider returned an invalid data-cell extraction candidate",
            metadata,
        ),
        DataCellVisionExtractionError::InvalidRequest => {
            ApiError::internal().with_request_id(metadata.request_id.clone())
        }
    }
}

fn invalid_vision(msg: &'static str, m: &RequestMetadata) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "vision_response_invalid", msg)
        .with_request_id(m.request_id.clone())
}
fn image_upload_too_large(m: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        "image must not exceed 10 MiB",
    )
    .with_request_id(m.request_id.clone())
}
fn image_upload_invalid(m: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "image_invalid",
        "image is empty, malformed, or exceeds safe image dimensions",
    )
    .with_request_id(m.request_id.clone())
}
fn image_upload_storage_error(error: AttachmentFileError, m: &RequestMetadata) -> ApiError {
    if matches!(error, AttachmentFileError::TooLarge) {
        image_upload_too_large(m)
    } else {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "image_upload_failed",
            "the image upload could not be stored",
        )
        .with_request_id(m.request_id.clone())
    }
}
fn valid_file_name(v: String, m: &RequestMetadata) -> Result<String, ApiError> {
    let v = v.trim();
    if v.is_empty()
        || v.len() > 255
        || matches!(v, "." | "..")
        || v.chars()
            .any(|c| matches!(c, '/' | '\\' | '\0') || c.is_control())
    {
        Err(ApiError::validation("file_name is invalid").with_request_id(m.request_id.clone()))
    } else {
        Ok(v.to_owned())
    }
}
fn attachment_root<'a>(
    state: &'a AppState,
    m: &RequestMetadata,
) -> Result<&'a Arc<std::path::PathBuf>, ApiError> {
    state
        .attachment_root
        .as_ref()
        .ok_or_else(|| ApiError::internal().with_request_id(m.request_id.clone()))
}
fn inspection_error(e: AttachmentInspectionError, m: &RequestMetadata) -> ApiError {
    let (status, code) = match e {
        AttachmentInspectionError::ExecutableContent => {
            (StatusCode::UNPROCESSABLE_ENTITY, "unsafe_attachment")
        }
        AttachmentInspectionError::SignatureMismatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "attachment_signature_mismatch",
        ),
        AttachmentInspectionError::ResourceLimit => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "attachment_resource_limit",
        ),
        AttachmentInspectionError::Io => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "attachment_inspection_failed",
        ),
    };
    ApiError::new(status, code, e.to_string()).with_request_id(m.request_id.clone())
}
fn ensure_human(p: &AuthPrincipal, m: &RequestMetadata) -> Result<(), ApiError> {
    if p.is_external_ai() {
        Err(ApiError::forbidden().with_request_id(m.request_id.clone()))
    } else {
        Ok(())
    }
}
fn ensure_admin(p: &AuthPrincipal, m: &RequestMetadata) -> Result<(), ApiError> {
    ensure_human(p, m)?;
    if p.is_lab_admin() {
        Ok(())
    } else {
        Err(ApiError::forbidden().with_request_id(m.request_id.clone()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StaticTokenAuthenticator, StoreJobRepository};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use muriarc_ai::{AssistantRuntimeConfig, CompletionResponse, MockProvider};
    use muriarc_core::{
        AiModelProfileBinding, AuditContext, Experiment, ExperimentEvent, Lab, LabRole,
        MuriArcStore, ObservationDefinition, ObservationPolicy, ObservationValueType, Project,
        ProjectRole, User, WorkspaceStore, WriteSource,
    };
    use muriarc_data::DataFiles;
    use muriarc_store_sqlite::SqliteStore;
    use tempfile::TempDir;

    fn jpeg_with_exif() -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1, 0, 6, b'E', b'x', b'i', b'f'];
        jpeg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        jpeg.extend_from_slice(&[0xff, 0xda, 0x00, 0x08, 1, 1, 0, 0, 63, 0]);
        jpeg.extend_from_slice(&[0x11, 0xff, 0x00, 0x22, 0xff, 0xd9]);
        jpeg
    }

    fn valid_png() -> Vec<u8> {
        STANDARD
            .decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
            )
            .unwrap()
    }

    struct ImageFixture {
        _temp: TempDir,
        state: AppState,
        store: Arc<SqliteStore>,
        principal: AuthPrincipal,
        metadata: RequestMetadata,
        project_id: Uuid,
        experiment_id: Uuid,
        experiment_event_id: Uuid,
        definition_id: Uuid,
        image_id: Uuid,
        attachment_id: Uuid,
    }

    impl ImageFixture {
        async fn new(media_type: &str, bytes: &[u8]) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let attachment_root = temp.path().join("attachments");
            let store = Arc::new(SqliteStore::in_memory().await.unwrap());
            store.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new("AI image route test", now).unwrap();
            store.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(lab.id, "image-route@example.test", "Image tester", now).unwrap();
            store.create_user(&user, &bootstrap).await.unwrap();
            let project = Project::new(lab.id, "Image route project", now).unwrap();
            store.create_project(&project, &bootstrap).await.unwrap();
            let experiment =
                Experiment::new(lab.id, project.id, "Image route experiment", now).unwrap();
            store
                .create_experiment(&experiment, &bootstrap)
                .await
                .unwrap();
            let event = ExperimentEvent::new(
                lab.id,
                project.id,
                experiment.id,
                "current_cell",
                "Current cell",
                now,
                now,
            )
            .unwrap();
            store
                .create_experiment_event(&event, &bootstrap)
                .await
                .unwrap();
            let definition = ObservationDefinition::new(
                lab.id,
                project.id,
                experiment.id,
                "visual_text",
                "Visual text",
                ObservationValueType::Text,
                ObservationPolicy::Versioned,
                now,
            )
            .unwrap();
            store
                .create_observation_definition(&definition, &bootstrap)
                .await
                .unwrap();
            let principal = AuthPrincipal::human(
                user.id,
                user.display_name.clone(),
                lab.id,
                [LabRole::LabAdmin],
            );
            let metadata = RequestMetadata {
                request_id: "image-route-test".to_owned(),
                reason: Some("verify private AI evidence".to_owned()),
            };
            let attachment_id = Uuid::new_v4();
            let image_id = Uuid::new_v4();
            let files = AttachmentFiles::new(&attachment_root);
            let object = files.write_bytes(attachment_id, bytes).await.unwrap();
            let attachment = Attachment {
                id: attachment_id,
                lab_id: lab.id,
                project_id: None,
                entity_type: "ai_private_image".to_owned(),
                entity_id: image_id,
                file_name: "evidence.bin".to_owned(),
                media_type: Some(media_type.to_owned()),
                relative_path: object.relative_path,
                size_bytes: object.size_bytes,
                sha256: object.sha256,
                version: 1,
                meta: RecordMeta::new(now),
            };
            let image = PrivateAiImage {
                id: image_id,
                lab_id: lab.id,
                user_id: user.id,
                conversation_id: None,
                attachment_id,
                project_id: None,
                status: PrivateImageStatus::Active,
                last_activity_at: now,
                expires_at: now + Duration::days(1),
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            store
                .create_private_ai_image(&attachment, &image, &principal.audit_context(&metadata))
                .await
                .unwrap();
            let authenticator = StaticTokenAuthenticator::new([(
                "image-route-token".to_owned(),
                principal.clone(),
            )])
            .unwrap();
            let state = AppState::new(
                store.clone(),
                Arc::new(authenticator),
                Arc::new(StoreJobRepository::new(store.clone())),
            )
            .with_data_storage(DataFiles::new(temp.path().join("data")), attachment_root);
            Self {
                _temp: temp,
                state,
                store,
                principal,
                metadata,
                project_id: project.id,
                experiment_id: experiment.id,
                experiment_event_id: event.id,
                definition_id: definition.id,
                image_id,
                attachment_id,
            }
        }
    }

    async fn own_image_count(fixture: &ImageFixture) -> usize {
        fixture
            .store
            .list_private_ai_images(&PrivateImageFilter {
                lab_id: fixture.principal.lab_id,
                user_id: Some(fixture.principal.user_id),
                conversation_id: None,
                project_id: None,
                status: None,
            })
            .await
            .unwrap()
            .len()
    }

    #[tokio::test]
    async fn upload_enforces_the_same_ten_mib_limit_from_header_and_stream() {
        let fixture = ImageFixture::new("image/png", &valid_png()).await;
        let before = own_image_count(&fixture).await;
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("10485761"));
        let header_error = upload(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiQuery(UploadQuery {
                file_name: "large.png".to_owned(),
                media_type: Some("image/png".to_owned()),
                conversation_id: None,
            }),
            headers,
            Body::from("not-read"),
        )
        .await
        .unwrap_err();
        assert_eq!(header_error.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let stream_error = upload(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiQuery(UploadQuery {
                file_name: "large.png".to_owned(),
                media_type: Some("image/png".to_owned()),
                conversation_id: None,
            }),
            HeaderMap::new(),
            Body::from(vec![0_u8; MAX_SANITIZED_VISION_INPUT_BYTES + 1]),
        )
        .await
        .unwrap_err();
        assert_eq!(stream_error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(own_image_count(&fixture).await, before);
    }

    #[tokio::test]
    async fn upload_accepts_strict_portable_images_and_rejects_bmp_or_malformed_png() {
        let png = valid_png();
        sanitize_vision_input("image/png", &png).unwrap();
        let fixture = ImageFixture::new("image/png", &png).await;
        let before = own_image_count(&fixture).await;
        let (status, response) = upload(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiQuery(UploadQuery {
                file_name: "portable.png".to_owned(),
                media_type: Some("image/png".to_owned()),
                conversation_id: None,
            }),
            HeaderMap::new(),
            Body::from(png),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response.0.data.image.status, PrivateImageStatus::Active);
        assert_eq!(own_image_count(&fixture).await, before + 1);

        let bmp_error = upload(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiQuery(UploadQuery {
                file_name: "unsupported.bmp".to_owned(),
                media_type: Some("image/bmp".to_owned()),
                conversation_id: None,
            }),
            HeaderMap::new(),
            Body::from("BMprivate"),
        )
        .await
        .unwrap_err();
        assert_eq!(bmp_error.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let malformed_error = upload(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiQuery(UploadQuery {
                file_name: "malformed.png".to_owned(),
                media_type: Some("image/png".to_owned()),
                conversation_id: None,
            }),
            HeaderMap::new(),
            Body::from(b"\x89PNG\r\n\x1a\n".as_slice()),
        )
        .await
        .unwrap_err();
        assert_eq!(malformed_error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(own_image_count(&fixture).await, before + 1);
    }

    #[tokio::test]
    async fn owner_can_read_archived_private_image_content() {
        let png = valid_png();
        let fixture = ImageFixture::new("image/png", &png).await;
        let current = fixture
            .store
            .get_private_ai_image(fixture.image_id)
            .await
            .unwrap();
        fixture
            .store
            .archive_private_ai_image(
                fixture.image_id,
                fixture.project_id,
                current.meta.revision,
                Utc::now(),
                &fixture.principal.audit_context(&fixture.metadata),
            )
            .await
            .unwrap();

        let response = image_content(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            AuthenticationMethod::Bearer,
            fixture.metadata.clone(),
            ApiPath(fixture.image_id),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
    }

    #[test]
    fn server_private_image_read_policy_matches_desktop_status_contract() {
        let metadata = RequestMetadata {
            request_id: "status-contract".to_owned(),
            reason: None,
        };
        for status in [
            PrivateImageStatus::Active,
            PrivateImageStatus::PendingApproval,
            PrivateImageStatus::Archived,
        ] {
            assert!(ensure_private_image_readable(status, &metadata).is_ok());
        }
        for status in [PrivateImageStatus::Processing, PrivateImageStatus::Failed] {
            assert_eq!(
                ensure_private_image_readable(status, &metadata)
                    .unwrap_err()
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        assert_eq!(
            ensure_private_image_readable(PrivateImageStatus::Expired, &metadata)
                .unwrap_err()
                .status(),
            StatusCode::GONE
        );
    }

    #[test]
    fn shared_sanitizer_strips_exif() {
        let b = jpeg_with_exif();
        let sanitized = sanitize_vision_input("image/jpeg", &b).unwrap();
        assert!(!sanitized.bytes().windows(4).any(|window| window == b"Exif"));
    }

    #[test]
    fn shared_extraction_candidate_leaves_the_server_draft_item_unselected() {
        let metadata = RequestMetadata {
            request_id: "candidate-parser-test".to_owned(),
            reason: None,
        };
        let now = Utc::now();
        let lab_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let experiment_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let definition = ObservationDefinition::new(
            lab_id,
            project_id,
            experiment_id,
            "visual_text",
            "Visual text",
            ObservationValueType::Text,
            ObservationPolicy::Versioned,
            now,
        )
        .unwrap();
        let data_cell = AiObservationDataCell {
            definition_id: definition.id,
            subject_type: ObservationSubjectType::Experiment,
            subject_id: experiment_id,
        };
        let candidate = DataCellVisionCandidate::new(
            &definition,
            ObservationValueData::Text("candidate".to_owned()),
            0.8,
            Some("visible".to_owned()),
        )
        .unwrap();
        let item = build_extraction_item(
            candidate,
            lab_id,
            user_id,
            project_id,
            experiment_id,
            event_id,
            &data_cell,
            now,
            &metadata,
        )
        .unwrap();
        assert!(
            !item.selected,
            "a model candidate must remain unselected until human approval"
        );
    }

    #[tokio::test]
    async fn sanitized_private_derivative_stays_outside_project_scope_before_approval() {
        let jpeg = jpeg_with_exif();
        let fixture = ImageFixture::new("image/jpeg", &jpeg).await;
        let prepared = prepare_private_images(
            &fixture.state,
            &fixture.principal,
            &fixture.metadata,
            None,
            Some(fixture.project_id),
            &[fixture.image_id],
        )
        .await
        .unwrap();
        assert_ne!(
            prepared[0].provider.evidence().sanitized_sha256,
            prepared[0].attachment.sha256,
            "EXIF removal must be reflected in the immutable evidence hash"
        );
        let derivatives = fixture
            .store
            .list_attachment_derivatives(fixture.attachment_id)
            .await
            .unwrap();
        assert_eq!(derivatives.len(), 1);
        assert_eq!(derivatives[0].kind, DerivativeKind::AiInput);
        assert_eq!(
            derivatives[0].project_id, None,
            "an unapproved private derivative must remain outside project/snapshot scope"
        );
    }

    #[tokio::test]
    async fn approval_permission_denial_leaves_the_draft_unchanged() {
        let fixture = ImageFixture::new("image/png", b"private-image").await;
        let now = Utc::now();
        let observation = Observation::new(
            fixture.principal.lab_id,
            fixture.project_id,
            fixture.experiment_id,
            fixture.experiment_event_id,
            fixture.definition_id,
            ObservationSubjectType::Experiment,
            fixture.experiment_id,
            now,
        )
        .unwrap();
        let mut value = ObservationValueRecord::new(
            observation.id,
            1,
            ObservationValueData::Text("candidate".to_owned()),
            now,
            now,
        )
        .unwrap();
        value.recorded_by = Some(fixture.principal.user_id);
        let attachment = fixture
            .store
            .get_attachment(fixture.attachment_id)
            .await
            .unwrap();
        let draft = AiExtractionDraft {
            id: Uuid::new_v4(),
            lab_id: fixture.principal.lab_id,
            user_id: fixture.principal.user_id,
            project_id: fixture.project_id,
            experiment_id: fixture.experiment_id,
            experiment_event_id: fixture.experiment_event_id,
            private_image_id: fixture.image_id,
            attachment_id: fixture.attachment_id,
            image_sha256: attachment.sha256,
            provider: "permission-test".to_owned(),
            model: "permission-test-model".to_owned(),
            tool_run_id: None,
            data_cell: None,
            evidence: Vec::new(),
            model_trace: None,
            status: AiExtractionStatus::PendingApproval,
            items: vec![AiExtractionItem {
                observation,
                value,
                confidence: 0.8,
                selected: false,
                source_label: None,
            }],
            error_code: None,
            meta: RecordMeta::new(now),
        };
        fixture
            .store
            .create_ai_extraction_draft(&draft, &fixture.principal.audit_context(&fixture.metadata))
            .await
            .unwrap();

        let viewer = AuthPrincipal::human(
            fixture.principal.user_id,
            "Extraction viewer",
            fixture.principal.lab_id,
            [],
        )
        .with_project_role(fixture.project_id, ProjectRole::Viewer);
        let error = approve_extraction(
            State(fixture.state.clone()),
            viewer,
            fixture.metadata.clone(),
            ApiPath(draft.id),
            ApiJson(ApproveInput {
                expected_revision: draft.meta.revision,
                selections: vec![ApproveSelectionInput {
                    item_index: 0,
                    value: ObservationValueData::Text("approved".to_owned()),
                    notes: Some("human review".to_owned()),
                }],
                selected_indexes: Vec::new(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);

        let unchanged = fixture
            .store
            .get_ai_extraction_draft(draft.id)
            .await
            .unwrap();
        assert_eq!(unchanged.status, AiExtractionStatus::PendingApproval);
        assert_eq!(unchanged.meta.revision, draft.meta.revision);
        assert!(
            unchanged.items.iter().all(|item| !item.selected),
            "permission denial must not select or apply a candidate"
        );
    }

    #[tokio::test]
    async fn human_owner_can_reject_pending_extraction_without_provider_access() {
        let fixture = ImageFixture::new("image/png", &valid_png()).await;
        let now = Utc::now();
        let observation = Observation::new(
            fixture.principal.lab_id,
            fixture.project_id,
            fixture.experiment_id,
            fixture.experiment_event_id,
            fixture.definition_id,
            ObservationSubjectType::Experiment,
            fixture.experiment_id,
            now,
        )
        .unwrap();
        let mut value = ObservationValueRecord::new(
            observation.id,
            1,
            ObservationValueData::Text("candidate".to_owned()),
            now,
            now,
        )
        .unwrap();
        value.recorded_by = Some(fixture.principal.user_id);
        let attachment = fixture
            .store
            .get_attachment(fixture.attachment_id)
            .await
            .unwrap();
        let draft = AiExtractionDraft {
            id: Uuid::new_v4(),
            lab_id: fixture.principal.lab_id,
            user_id: fixture.principal.user_id,
            project_id: fixture.project_id,
            experiment_id: fixture.experiment_id,
            experiment_event_id: fixture.experiment_event_id,
            private_image_id: fixture.image_id,
            attachment_id: fixture.attachment_id,
            image_sha256: attachment.sha256,
            provider: "must-not-run".to_owned(),
            model: "must-not-run".to_owned(),
            tool_run_id: None,
            data_cell: None,
            evidence: Vec::new(),
            model_trace: None,
            status: AiExtractionStatus::PendingApproval,
            items: vec![AiExtractionItem {
                observation,
                value,
                confidence: 0.8,
                selected: false,
                source_label: None,
            }],
            error_code: None,
            meta: RecordMeta::new(now),
        };
        fixture
            .store
            .create_ai_extraction_draft(&draft, &fixture.principal.audit_context(&fixture.metadata))
            .await
            .unwrap();

        let rejected = reject_extraction(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiPath(draft.id),
            ApiJson(RejectInput {
                expected_revision: draft.meta.revision,
            }),
        )
        .await
        .unwrap();
        assert_eq!(rejected.0.data.status, AiExtractionStatus::Rejected);
        assert_eq!(
            fixture
                .store
                .get_ai_extraction_draft(draft.id)
                .await
                .unwrap()
                .status,
            AiExtractionStatus::Rejected
        );
    }

    #[tokio::test]
    async fn nonportable_media_and_cross_scope_cells_fail_in_pre_provider_validation() {
        let fixture = ImageFixture::new("image/bmp", b"BMprivate-image").await;
        let media_error = prepare_private_images(
            &fixture.state,
            &fixture.principal,
            &fixture.metadata,
            None,
            Some(fixture.project_id),
            &[fixture.image_id],
        )
        .await
        .unwrap_err();
        assert_eq!(media_error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            fixture
                .store
                .list_attachment_derivatives(fixture.attachment_id)
                .await
                .unwrap()
                .is_empty(),
            "unsupported media must fail before sanitization or Provider preparation"
        );

        let cell_error = create_extraction(
            State(fixture.state.clone()),
            fixture.principal.clone(),
            fixture.metadata.clone(),
            ApiJson(CreateExtractionInput {
                image_ids: vec![fixture.image_id],
                private_image_id: None,
                project_id: fixture.project_id,
                experiment_id: fixture.experiment_id,
                experiment_event_id: fixture.experiment_event_id,
                current_data_cell: ExtractionDataCellInput {
                    definition_id: fixture.definition_id,
                    subject_type: ObservationSubjectType::Artifact,
                    subject_id: fixture.attachment_id,
                },
                vision_model_profile_id: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(cell_error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            fixture
                .store
                .list_attachment_derivatives(fixture.attachment_id)
                .await
                .unwrap()
                .is_empty(),
            "cross-scope cells must fail before image preparation or Provider resolution"
        );
        assert!(
            validate_extraction_data_cell_scope(
                &fixture.state,
                &fixture.principal,
                &fixture.metadata,
                fixture.project_id,
                fixture.experiment_id,
                &AiObservationDataCell {
                    definition_id: fixture.definition_id,
                    subject_type: ObservationSubjectType::Artifact,
                    subject_id: fixture.attachment_id,
                },
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn visual_extraction_wire_format_never_accepts_model_authored_entity_ids() {
        let definition = ObservationDefinition::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "body_weight",
            "Body weight",
            ObservationValueType::Text,
            ObservationPolicy::Versioned,
            Utc::now(),
        )
        .unwrap();
        let images = vec![
            PreparedAssistantImage::new(
                Uuid::new_v4(),
                "a".repeat(64),
                "image/png",
                "iVBORw0KGgo=".to_owned(),
            )
            .unwrap(),
        ];
        let response = |content: &str| CompletionResponse {
            id: None,
            model: None,
            content: Some(content.to_owned()),
            tool_calls: Vec::new(),
            finish_reason: Some("stop".to_owned()),
            usage: None,
        };

        let safe = MockProvider::new(
            "vision-provider",
            "vision-model",
            [Ok(response(
                r#"{"candidates":[{"value":{"type":"text","value":"23.4 g"},"confidence":0.92,"sourceLabel":"23.4 g"}]}"#,
            ))],
        );
        let parsed = extract_data_cell_vision(
            &safe,
            ProviderCredentials::none(),
            DataCellVisionExtractionRequest {
                model_profile: AiModelProfileBinding {
                    profile_id: Uuid::new_v4(),
                    profile_version: 1,
                },
                runtime: AssistantRuntimeConfig::default(),
                definition: &definition,
                images: &images,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            parsed.candidate.value(),
            &ObservationValueData::Text("23.4 g".to_owned())
        );

        for forbidden in [
            format!(
                r#"{{"candidates":[{{"subjectId":"{}","value":{{"type":"text","value":"23.4 g"}},"confidence":0.92,"sourceLabel":null}}]}}"#,
                Uuid::new_v4()
            ),
            format!(
                r#"{{"candidates":[{{"definitionId":"{}","value":{{"type":"text","value":"23.4 g"}},"confidence":0.92,"sourceLabel":null}}]}}"#,
                Uuid::new_v4()
            ),
        ] {
            let provider = MockProvider::new(
                "vision-provider",
                "vision-model",
                [Ok(response(&forbidden))],
            );
            assert!(matches!(
                extract_data_cell_vision(
                    &provider,
                    ProviderCredentials::none(),
                    DataCellVisionExtractionRequest {
                        model_profile: AiModelProfileBinding {
                            profile_id: Uuid::new_v4(),
                            profile_version: 1,
                        },
                        runtime: AssistantRuntimeConfig::default(),
                        definition: &definition,
                        images: &images,
                    },
                )
                .await,
                Err(DataCellVisionExtractionError::InvalidResponse)
            ));
        }
    }
}
