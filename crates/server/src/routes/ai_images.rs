use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse,
    ai_api::{provider_api_error, provider_resolve_error},
    attachment_files::{open_verified, remove_installed_object, write_object},
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
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Duration, Utc};
use muriarc_ai::{
    AiProvider, ChatMessage, CompletionRequest, ProviderCredentials, VisionImageInput,
    estimate_completion_input_tokens,
};
use muriarc_core::{
    AiExtractionDraft, AiExtractionItem, AiExtractionStatus, AppliedAiExtraction, Attachment,
    AttachmentDerivative, AuditAction, DerivativeKind, DerivativeStatus, EntityType, Observation,
    ObservationDefinition, ObservationSubjectType, ObservationValueData, ObservationValueRecord,
    Permission, PrivateAiImage, PrivateImageFilter, PrivateImageStats, PrivateImageStatus,
    RecordMeta, StoreError,
};
use muriarc_data::{
    AttachmentContentKind, AttachmentFiles, AttachmentInspectionError, MAX_ATTACHMENT_BYTES,
    inspect_attachment,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio_util::io::ReaderStream;
use uuid::Uuid;
const MAX_AI_INPUT_BYTES: usize = 10 * 1024 * 1024;

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
            .route("/ai/extractions/{id}/approve", post(approve_extraction)),
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
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|v| v > MAX_ATTACHMENT_BYTES)
    {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "image must not exceed 100 MiB",
        )
        .with_request_id(m.request_id));
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
    let object = write_object(root.as_ref(), attachment_id, body)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_upload_failed",
                e.to_string(),
            )
            .with_request_id(m.request_id.clone())
        })?;
    let inspection =
        match inspect_attachment(&object.absolute_path, &file_name, declared.as_deref()).await {
            Ok(v)
                if !matches!(
                    v.kind,
                    AttachmentContentKind::Opaque | AttachmentContentKind::Pdf
                ) =>
            {
                v
            }
            Ok(_) => {
                remove_installed_object(root.as_ref(), &object).await.ok();
                return Err(ApiError::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "image_required",
                    "private AI space accepts supported images only",
                )
                .with_request_id(m.request_id));
            }
            Err(e) => {
                remove_installed_object(root.as_ref(), &object).await.ok();
                return Err(inspection_error(e, &m));
            }
        };
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
    if conversation.legacy_read_only {
        return Err(ApiError::conflict("legacy AI conversation is read-only")
            .with_request_id(metadata.request_id.clone()));
    }
    Ok(())
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
    if image.status == PrivateImageStatus::Expired {
        return Err(ApiError::new(
            StatusCode::GONE,
            "image_expired",
            "private image retention expired",
        )
        .with_request_id(m.request_id));
    }
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateExtractionInput {
    private_image_id: Uuid,
    project_id: Uuid,
    experiment_id: Uuid,
    experiment_event_id: Uuid,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireExtraction {
    items: Vec<WireItem>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireItem {
    definition_id: Uuid,
    subject_type: ObservationSubjectType,
    subject_id: Uuid,
    value: ObservationValueData,
    confidence: f64,
    source_label: Option<String>,
}
async fn create_extraction(
    State(state): State<AppState>,
    p: AuthPrincipal,
    m: RequestMetadata,
    ApiJson(q): ApiJson<CreateExtractionInput>,
) -> Result<(StatusCode, Json<ItemResponse<AiExtractionDraft>>), ApiError> {
    ensure_human(&p, &m)?;
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
    let image = store(state.store.get_private_ai_image(q.private_image_id), &m).await?;
    if image.lab_id != p.lab_id || image.user_id != p.user_id {
        return Err(
            ApiError::not_found("private image was not found").with_request_id(m.request_id)
        );
    }
    if matches!(
        image.status,
        PrivateImageStatus::Processing
            | PrivateImageStatus::PendingApproval
            | PrivateImageStatus::Expired
    ) {
        return Err(
            ApiError::conflict("private image is busy or expired").with_request_id(m.request_id)
        );
    }
    let a = store(state.store.get_attachment(image.attachment_id), &m).await?;
    let defs = store(
        state.store.list_observation_definitions(q.experiment_id),
        &m,
    )
    .await?;
    if defs.is_empty() {
        return Err(
            ApiError::validation("experiment has no observation definitions")
                .with_request_id(m.request_id),
        );
    }
    let bytes = ai_input_copy(&state, &p, &m, &a, q.project_id).await?;
    if bytes.len() > MAX_AI_INPUT_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "vision_image_too_large",
            "derived AI input must not exceed 10 MiB",
        )
        .with_request_id(m.request_id));
    }
    let media_type = a
        .media_type
        .clone()
        .filter(|v| v.starts_with("image/"))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "vision_image_required",
                "visual extraction requires an image",
            )
            .with_request_id(m.request_id.clone())
        })?;
    let schema=defs.iter().map(|d|serde_json::json!({"definition_id":d.id,"key":d.key,"label":d.label,"value_type":d.value_type,"unit":d.unit,"categories":d.categories})).collect::<Vec<_>>();
    let prompt = format!(
        "Extract only clearly visible cells mapped to these definitions: {}. Return strict JSON {{\"items\":[{{\"definition_id\":\"uuid\",\"subject_type\":\"experiment|animal|sample|artifact\",\"subject_id\":\"uuid\",\"value\":{{\"type\":\"number|text|boolean|date|category|json\",\"value\":...}},\"confidence\":0.0,\"source_label\":\"visible label\"}}]}}. Do not invent values. Experiment subject id: {}.",
        serde_json::to_string(&schema)
            .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?,
        q.experiment_id
    );
    let ResolvedAiProvider {
        provider,
        api_key,
        runtime,
        ..
    } = state
        .ai_providers
        .resolve_vision(p.user_id)
        .await
        .map_err(|error| provider_resolve_error(error, &m))?;
    let provider_id = provider.provider_id().to_owned();
    let model = provider.model().to_owned();
    let credentials = match api_key.as_ref() {
        Some(s) => ProviderCredentials::bearer(s.as_str())
            .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?,
        None => ProviderCredentials::none(),
    };
    let mut request = CompletionRequest::new(vec![ChatMessage::user_with_images(
        prompt,
        vec![VisionImageInput {
            media_type,
            data_base64: STANDARD.encode(bytes),
        }],
    )]);
    request.temperature = Some(runtime.temperature);
    request.max_output_tokens = Some(runtime.max_output_tokens);
    let estimated_input_tokens = estimate_completion_input_tokens(&request);
    if estimated_input_tokens > u64::from(runtime.max_input_tokens) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "context_exceeded",
            "the image and extraction prompt exceed the configured input token budget",
        )
        .with_details(serde_json::json!({
            "estimatedInputTokens": estimated_input_tokens,
            "inputTokenCountIsEstimate": true,
            "maxInputTokens": runtime.max_input_tokens,
            "currentInputTruncated": false,
        }))
        .with_request_id(m.request_id));
    }
    let response = provider
        .complete(request, credentials)
        .await
        .map_err(|error| provider_api_error(error, &m))?;
    let raw = response
        .content
        .ok_or_else(|| invalid_vision("provider returned no structured result", &m))?;
    let wire: WireExtraction = serde_json::from_str(strip_json_fence(&raw))
        .map_err(|_| invalid_vision("provider result is not valid extraction JSON", &m))?;
    if wire.items.is_empty() || wire.items.len() > 500 {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "vision_result_empty",
            "no reviewable values were extracted",
        )
        .with_request_id(m.request_id));
    }
    let map: HashMap<Uuid, &ObservationDefinition> = defs.iter().map(|d| (d.id, d)).collect();
    let now = Utc::now();
    let mut items = Vec::new();
    for src in wire.items {
        let def = map
            .get(&src.definition_id)
            .ok_or_else(|| invalid_vision("unknown observation definition", &m))?;
        def.validate_value(&src.value)
            .map_err(|_| invalid_vision("value type mismatch", &m))?;
        if src.subject_type == ObservationSubjectType::Experiment
            && src.subject_id != q.experiment_id
        {
            return Err(invalid_vision("invalid experiment subject", &m));
        }
        let observation = Observation::new(
            p.lab_id,
            q.project_id,
            q.experiment_id,
            q.experiment_event_id,
            src.definition_id,
            src.subject_type,
            src.subject_id,
            now,
        )
        .map_err(|_| invalid_vision("invalid observation", &m))?;
        let mut value = ObservationValueRecord::new(observation.id, 1, src.value, now, now)
            .map_err(|_| invalid_vision("invalid value", &m))?;
        value.recorded_by = Some(p.user_id);
        value.notes = Some("AI visual extraction; pending human approval".into());
        items.push(AiExtractionItem {
            observation,
            value,
            confidence: src.confidence,
            selected: true,
            source_label: src.source_label,
        });
    }
    let draft = AiExtractionDraft {
        id: Uuid::new_v4(),
        lab_id: p.lab_id,
        user_id: p.user_id,
        project_id: q.project_id,
        experiment_id: q.experiment_id,
        experiment_event_id: q.experiment_event_id,
        private_image_id: image.id,
        attachment_id: a.id,
        image_sha256: a.sha256,
        provider: provider_id,
        model,
        tool_run_id: None,
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
#[serde(deny_unknown_fields)]
struct ApproveInput {
    expected_revision: i64,
    selected_indexes: Vec<usize>,
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
    scope::project_with_permission(
        &state,
        &p,
        &m,
        d.project_id,
        Permission::WriteMeasurementDraft,
    )
    .await?;
    let v = store(
        state.store.apply_ai_extraction_draft(
            id,
            q.expected_revision,
            &q.selected_indexes,
            &p.audit_context(&m),
        ),
        &m,
    )
    .await?;
    Ok(item(v, &m))
}

async fn ai_input_copy(
    state: &AppState,
    p: &AuthPrincipal,
    m: &RequestMetadata,
    a: &Attachment,
    project: Uuid,
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
    let prepared = if a.media_type.as_deref() == Some("image/jpeg") {
        strip_jpeg_exif(&original)
    } else {
        original
    };
    if prepared.len() > MAX_AI_INPUT_BYTES {
        return Ok(prepared);
    }
    let id = Uuid::new_v4();
    let object = files
        .write_bytes(id, &prepared)
        .await
        .map_err(|_| ApiError::internal().with_request_id(m.request_id.clone()))?;
    let d = AttachmentDerivative {
        id,
        lab_id: p.lab_id,
        project_id: Some(project),
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
fn strip_jpeg_exif(bytes: &[u8]) -> Vec<u8> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return bytes.to_vec();
    }
    let mut out = bytes[..2].to_vec();
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            out.extend_from_slice(&bytes[i..]);
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xda || marker == 0xd9 {
            out.extend_from_slice(&bytes[i..]);
            break;
        }
        let n = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if n < 2 || i + n + 2 > bytes.len() {
            return bytes.to_vec();
        }
        if marker != 0xe1 {
            out.extend_from_slice(&bytes[i..i + n + 2])
        }
        i += n + 2
    }
    out
}
fn strip_json_fence(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix("```json")
        .or_else(|| v.strip_prefix("```"))
        .and_then(|v| v.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(v)
}
fn invalid_vision(msg: &'static str, m: &RequestMetadata) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "vision_response_invalid", msg)
        .with_request_id(m.request_id.clone())
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
    #[test]
    fn strips_exif() {
        let b = [
            &[0xff, 0xd8][..],
            &[0xff, 0xe1, 0, 6, b'E', b'x', b'i', b'f'][..],
            &[0xff, 0xda, 0, 2, 1, 2][..],
        ]
        .concat();
        assert!(!strip_jpeg_exif(&b).windows(4).any(|x| x == b"Exif"))
    }
}
