use std::fmt::Write as _;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{delete, get, post},
};
use muriarc_core::{Attachment, Permission, RecordMeta};
use muriarc_data::{AttachmentInspectionError, inspect_attachment};
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse,
    attachment_files::{
        AttachmentFileError, open_verified, remove_installed_object, write_object_with_limit,
    },
    collection, ensure_lab, item, scope, store,
    validation::{collection_limit, optional_text, required_text, truncate, validation},
};

const MAX_FILE_NAME_BYTES: usize = 255;
const MAX_MEDIA_TYPE_BYTES: usize = 127;
const MAX_ATTACHMENT_VERSION: i32 = 1_000_000;

pub(super) fn router() -> Router<AppState> {
    let upload = Router::new()
        .route("/attachments/upload", post(upload))
        // The request is streamed and bounded by `write_object`; this is
        // intentionally independent from the 1 MiB JSON extractor limit.
        .layer(DefaultBodyLimit::disable());
    let reads = Router::new()
        .route("/attachments", get(list))
        .route("/attachments/{id}/content", get(download))
        .route("/attachments/{id}/preview", get(preview))
        .route("/attachments/{id}", delete(delete_attachment))
        .layer(DefaultBodyLimit::max(1024 * 1024));
    upload.merge(reads)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AttachmentTarget {
    Project,
    Animal,
    GenotypingBatch,
    Experiment,
    Measurement,
    Sample,
}

impl AttachmentTarget {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Animal => "animal",
            Self::GenotypingBatch => "genotyping_batch",
            Self::Experiment => "experiment",
            Self::Measurement => "measurement",
            Self::Sample => "sample",
        }
    }

    fn from_stored(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "animal" => Some(Self::Animal),
            "genotyping_batch" => Some(Self::GenotypingBatch),
            "experiment" => Some(Self::Experiment),
            "measurement" => Some(Self::Measurement),
            "sample" => Some(Self::Sample),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
struct AttachmentMetadata {
    id: Uuid,
    lab_id: Uuid,
    project_id: Option<Uuid>,
    entity_type: String,
    entity_id: Uuid,
    file_name: String,
    media_type: Option<String>,
    size_bytes: i64,
    sha256: String,
    version: i32,
    content_href: String,
    preview_supported: bool,
    preview_href: Option<String>,
    preview_reason: Option<String>,
    meta: RecordMeta,
}

impl From<Attachment> for AttachmentMetadata {
    fn from(attachment: Attachment) -> Self {
        let id = attachment.id;
        Self {
            id,
            lab_id: attachment.lab_id,
            project_id: attachment.project_id,
            entity_type: attachment.entity_type,
            entity_id: attachment.entity_id,
            file_name: attachment.file_name,
            media_type: attachment.media_type.clone(),
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256,
            version: attachment.version,
            content_href: format!("/api/v1/attachments/{id}/content"),
            preview_supported: preview_media_type(attachment.media_type.as_deref()),
            preview_href: preview_media_type(attachment.media_type.as_deref())
                .then(|| format!("/api/v1/attachments/{id}/preview")),
            preview_reason: (!preview_media_type(attachment.media_type.as_deref()))
                .then(|| "该科研文件可安全保存和下载，但当前格式不支持在线预览".to_owned()),
            meta: attachment.meta,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    entity_type: AttachmentTarget,
    entity_id: Uuid,
    project_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<AttachmentMetadata>>, ApiError> {
    let effective_project = authorize_target(
        &state,
        &principal,
        &metadata,
        query.entity_type,
        query.entity_id,
        query.project_id,
        Permission::ReadAttachment,
    )
    .await?;
    let mut attachments = store(
        state.store.list_attachments(
            principal.lab_id,
            query.entity_type.as_str(),
            query.entity_id,
        ),
        &metadata,
    )
    .await?;
    if let Some(project_id) = effective_project {
        attachments.retain(|attachment| attachment.project_id == Some(project_id));
    }
    truncate(&mut attachments, collection_limit(query.limit, &metadata)?);
    Ok(collection(
        attachments
            .into_iter()
            .map(AttachmentMetadata::from)
            .collect(),
        &metadata,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    entity_type: AttachmentTarget,
    entity_id: Uuid,
    project_id: Option<Uuid>,
    file_name: String,
    media_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteAttachmentRequest {
    expected_revision: i64,
    reason: Option<String>,
}

async fn upload(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<ItemResponse<AttachmentMetadata>>), ApiError> {
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|size| size > state.deployment_security.attachment_max_bytes())
    {
        return Err(payload_too_large(
            &metadata,
            state.deployment_security.attachment_max_bytes(),
        ));
    }
    let effective_project = authorize_target(
        &state,
        &principal,
        &metadata,
        query.entity_type,
        query.entity_id,
        query.project_id,
        Permission::WriteAttachment,
    )
    .await?;
    let file_name = validated_file_name(query.file_name, &metadata)?;
    let header_media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value != "application/octet-stream")
        .map(str::to_owned);
    let declared_media_type =
        validated_media_type(query.media_type.or(header_media_type), &metadata)?;
    let existing = store(
        state.store.list_attachments(
            principal.lab_id,
            query.entity_type.as_str(),
            query.entity_id,
        ),
        &metadata,
    )
    .await?;
    let version = existing
        .iter()
        .filter(|attachment| attachment.file_name == file_name)
        .map(|attachment| attachment.version)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|version| *version <= MAX_ATTACHMENT_VERSION)
        .ok_or_else(|| validation("attachment version limit was reached", &metadata))?;

    let root = attachment_root(&state, &metadata)?;
    let id = Uuid::new_v4();
    let object = write_object_with_limit(
        root.as_ref(),
        id,
        body,
        state.deployment_security.attachment_max_bytes(),
    )
    .await
    .map_err(|error| {
        upload_file_error(
            error,
            &metadata,
            state.deployment_security.attachment_max_bytes(),
        )
    })?;
    let inspection = match inspect_attachment(
        &object.absolute_path,
        &file_name,
        declared_media_type.as_deref(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            if let Err(cleanup_error) = remove_installed_object(root.as_ref(), &object).await {
                tracing::error!(
                    request_id = %metadata.request_id,
                    attachment_id = %id,
                    error = %cleanup_error,
                    "failed to clean a rejected attachment object"
                );
            }
            return Err(inspection_error(error, &metadata));
        }
    };
    let attachment = Attachment {
        id,
        lab_id: principal.lab_id,
        project_id: effective_project,
        entity_type: query.entity_type.as_str().to_owned(),
        entity_id: query.entity_id,
        file_name,
        media_type: inspection.media_type,
        relative_path: object.relative_path.clone(),
        size_bytes: object.size_bytes,
        sha256: object.sha256.clone(),
        version,
        meta: RecordMeta::new(chrono::Utc::now()),
    };
    if let Err(error) = state
        .store
        .create_attachment(&attachment, &principal.audit_context(&metadata))
        .await
    {
        if let Err(cleanup_error) = remove_installed_object(root.as_ref(), &object).await {
            tracing::error!(
                request_id = %metadata.request_id,
                attachment_id = %id,
                error = %cleanup_error,
                "failed to clean an attachment object after metadata rollback"
            );
        }
        return Err(ApiError::from_store(error).with_request_id(metadata.request_id));
    }
    Ok((
        StatusCode::CREATED,
        item(AttachmentMetadata::from(attachment), &metadata),
    ))
}

async fn download(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    let attachment = store(state.store.get_attachment(id), &metadata).await?;
    ensure_lab(attachment.lab_id, &principal, &metadata)?;
    let target = AttachmentTarget::from_stored(&attachment.entity_type).ok_or_else(|| {
        ApiError::not_found("attachment target was not found")
            .with_request_id(metadata.request_id.clone())
    })?;
    let effective_project = authorize_target(
        &state,
        &principal,
        &metadata,
        target,
        attachment.entity_id,
        attachment.project_id,
        Permission::ReadAttachment,
    )
    .await?;
    if effective_project != attachment.project_id {
        return Err(
            ApiError::not_found("attachment was not found").with_request_id(metadata.request_id)
        );
    }

    let root = attachment_root(&state, &metadata)?;
    let object = open_verified(root.as_ref(), &attachment)
        .await
        .map_err(|error| download_file_error(error, id, &metadata))?;
    let stream = ReaderStream::new(object.file);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&object.size_bytes.to_string())
            .map_err(|_| ApiError::internal().with_request_id(metadata.request_id.clone()))?,
    );
    headers.insert(
        header::CONTENT_TYPE,
        attachment
            .media_type
            .as_deref()
            .and_then(|value| HeaderValue::from_str(value).ok())
            .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(&attachment.file_name, &metadata)?,
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

async fn preview(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    let attachment = store(state.store.get_attachment(id), &metadata).await?;
    if !preview_media_type(attachment.media_type.as_deref()) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "preview_unavailable",
            "this attachment format is stored safely but does not support inline preview",
        )
        .with_request_id(metadata.request_id));
    }
    let file_name = attachment.file_name.clone();
    let mut response = download(State(state), principal, metadata.clone(), ApiPath(id)).await?;
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        inline_content_disposition(&file_name, &metadata)?,
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'",
        ),
    );
    Ok(response)
}

async fn delete_attachment(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<DeleteAttachmentRequest>,
) -> Result<Json<ItemResponse<AttachmentMetadata>>, ApiError> {
    let attachment = store(state.store.get_attachment(id), &metadata).await?;
    ensure_lab(attachment.lab_id, &principal, &metadata)?;
    let target = AttachmentTarget::from_stored(&attachment.entity_type).ok_or_else(|| {
        ApiError::not_found("attachment target was not found")
            .with_request_id(metadata.request_id.clone())
    })?;
    let effective_project = authorize_target(
        &state,
        &principal,
        &metadata,
        target,
        attachment.entity_id,
        attachment.project_id,
        Permission::WriteAttachment,
    )
    .await?;
    if effective_project != attachment.project_id {
        return Err(
            ApiError::not_found("attachment was not found").with_request_id(metadata.request_id)
        );
    }
    let mut audit = principal.audit_context(&metadata);
    if payload.reason.is_some() {
        audit.reason = optional_text(payload.reason, "reason", 1024, &metadata)?;
    }
    let deleted = store(
        state.store.soft_delete_attachment(
            id,
            payload.expected_revision,
            chrono::Utc::now(),
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(AttachmentMetadata::from(deleted), &metadata))
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

async fn authorize_target(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    entity_type: AttachmentTarget,
    entity_id: Uuid,
    requested_project_id: Option<Uuid>,
    permission: Permission,
) -> Result<Option<Uuid>, ApiError> {
    match entity_type {
        AttachmentTarget::Project => {
            if requested_project_id.is_some_and(|project_id| project_id != entity_id) {
                return Err(ApiError::not_found("attachment target was not found")
                    .with_request_id(metadata.request_id.clone()));
            }
            scope::project_with_permission(state, principal, metadata, entity_id, permission)
                .await?;
            Ok(Some(entity_id))
        }
        AttachmentTarget::Animal => {
            scope::animal_with_permission(
                state,
                principal,
                metadata,
                entity_id,
                requested_project_id,
                permission,
            )
            .await?;
            Ok(requested_project_id)
        }
        AttachmentTarget::GenotypingBatch => {
            let batch = store(state.store.get_genotyping_batch(entity_id), metadata).await?;
            ensure_lab(batch.lab_id, principal, metadata)?;
            if requested_project_id != batch.project_id {
                return Err(ApiError::not_found("attachment target was not found")
                    .with_request_id(metadata.request_id.clone()));
            }
            let required_permission = if permission == Permission::WriteAttachment {
                Permission::ManageBreeding
            } else {
                permission
            };
            scope::optional_project_permission(
                state,
                principal,
                metadata,
                batch.project_id,
                required_permission,
            )
            .await?;
            if permission == Permission::WriteAttachment
                && batch.status != muriarc_core::GenotypingBatchStatus::Draft
            {
                return Err(ApiError::conflict(
                    "attachments can only be changed while the genotyping batch is a draft",
                )
                .with_request_id(metadata.request_id.clone()));
            }
            Ok(batch.project_id)
        }
        AttachmentTarget::Experiment => {
            let experiment = scope::experiment_with_permission(
                state, principal, metadata, entity_id, permission,
            )
            .await?;
            ensure_requested_project(requested_project_id, experiment.project_id, metadata)?;
            Ok(Some(experiment.project_id))
        }
        AttachmentTarget::Measurement => {
            let measurement = store(state.store.get_measurement(entity_id), metadata).await?;
            ensure_lab(measurement.lab_id, principal, metadata)?;
            ensure_requested_project(requested_project_id, measurement.project_id, metadata)?;
            scope::project_with_permission(
                state,
                principal,
                metadata,
                measurement.project_id,
                permission,
            )
            .await?;
            Ok(Some(measurement.project_id))
        }
        AttachmentTarget::Sample => {
            let sample = store(state.store.get_sample(entity_id), metadata).await?;
            ensure_lab(sample.lab_id, principal, metadata)?;
            ensure_requested_project(requested_project_id, sample.project_id, metadata)?;
            scope::project_with_permission(
                state,
                principal,
                metadata,
                sample.project_id,
                permission,
            )
            .await?;
            Ok(Some(sample.project_id))
        }
    }
}

fn ensure_requested_project(
    requested: Option<Uuid>,
    actual: Uuid,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if requested.is_none_or(|project_id| project_id == actual) {
        Ok(())
    } else {
        Err(ApiError::not_found("attachment target was not found")
            .with_request_id(metadata.request_id.clone()))
    }
}

fn validated_file_name(value: String, metadata: &RequestMetadata) -> Result<String, ApiError> {
    let value = required_text(value, "file_name", MAX_FILE_NAME_BYTES, metadata)?;
    if matches!(value.as_str(), "." | "..")
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(validation(
            "file_name must be a plain name without path separators or control characters",
            metadata,
        ));
    }
    Ok(value)
}

fn validated_media_type(
    value: Option<String>,
    metadata: &RequestMetadata,
) -> Result<Option<String>, ApiError> {
    let value = optional_text(value, "media_type", MAX_MEDIA_TYPE_BYTES, metadata)?;
    if value
        .as_ref()
        .is_some_and(|value| !value.is_ascii() || value.chars().any(char::is_control))
    {
        return Err(validation(
            "media_type must contain printable ASCII characters only",
            metadata,
        ));
    }
    Ok(value)
}

fn attachment_root<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a std::sync::Arc<std::path::PathBuf>, ApiError> {
    state.attachment_root.as_ref().ok_or_else(|| {
        tracing::error!(request_id = %metadata.request_id, "attachment storage is not configured");
        ApiError::internal().with_request_id(metadata.request_id.clone())
    })
}

fn upload_file_error(
    error: AttachmentFileError,
    metadata: &RequestMetadata,
    max_bytes: u64,
) -> ApiError {
    if matches!(error, AttachmentFileError::TooLarge) {
        return payload_too_large(metadata, max_bytes);
    }
    if matches!(error, AttachmentFileError::AlreadyExists) {
        return ApiError::conflict("attachment object already exists")
            .with_request_id(metadata.request_id.clone());
    }
    tracing::error!(request_id = %metadata.request_id, error = %error, "attachment upload failed");
    ApiError::internal().with_request_id(metadata.request_id.clone())
}

fn download_file_error(
    error: AttachmentFileError,
    attachment_id: Uuid,
    metadata: &RequestMetadata,
) -> ApiError {
    tracing::error!(
        request_id = %metadata.request_id,
        %attachment_id,
        error = %error,
        "attachment download failed closed"
    );
    match error {
        AttachmentFileError::Missing
        | AttachmentFileError::Integrity
        | AttachmentFileError::UnsafePath
        | AttachmentFileError::TooLarge => {
            ApiError::conflict("attachment content is unavailable or failed integrity verification")
                .with_request_id(metadata.request_id.clone())
        }
        _ => ApiError::internal().with_request_id(metadata.request_id.clone()),
    }
}

fn inspection_error(error: AttachmentInspectionError, metadata: &RequestMetadata) -> ApiError {
    match error {
        AttachmentInspectionError::ExecutableContent => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsafe_attachment",
            "executable or script content is not accepted",
        ),
        AttachmentInspectionError::SignatureMismatch => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "attachment_signature_mismatch",
            "the file extension, declared media type and content signature do not agree",
        ),
        AttachmentInspectionError::ResourceLimit => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "attachment_resource_limit",
            "the image or document exceeds safe preview resource limits",
        ),
        AttachmentInspectionError::Io => {
            tracing::error!(
                request_id = %metadata.request_id,
                "attachment content inspection failed"
            );
            return ApiError::internal().with_request_id(metadata.request_id.clone());
        }
    }
    .with_request_id(metadata.request_id.clone())
}

fn payload_too_large(metadata: &RequestMetadata, max_bytes: u64) -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!("attachment must not exceed {max_bytes} bytes"),
    )
    .with_request_id(metadata.request_id.clone())
}

fn inline_content_disposition(
    file_name: &str,
    metadata: &RequestMetadata,
) -> Result<HeaderValue, ApiError> {
    content_disposition_value("inline", file_name, metadata)
}

fn content_disposition(
    file_name: &str,
    metadata: &RequestMetadata,
) -> Result<HeaderValue, ApiError> {
    content_disposition_value("attachment", file_name, metadata)
}

fn content_disposition_value(
    disposition: &str,
    file_name: &str,
    metadata: &RequestMetadata,
) -> Result<HeaderValue, ApiError> {
    let fallback: String = file_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let fallback = if fallback.trim().is_empty() {
        "attachment"
    } else {
        fallback.as_str()
    };
    let mut encoded = String::new();
    for byte in file_name.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                *byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(*byte));
        } else {
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .map_err(|_| ApiError::internal().with_request_id(metadata.request_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> RequestMetadata {
        RequestMetadata {
            request_id: "attachment-unit".to_owned(),
            reason: None,
        }
    }

    #[test]
    fn file_names_never_become_paths_or_response_headers() {
        let metadata = metadata();
        for invalid in ["../x", "a/b", "a\\b", "line\nbreak", ".."] {
            assert!(validated_file_name(invalid.to_owned(), &metadata).is_err());
        }
        let disposition = content_disposition("结果 \"A\".csv", &metadata).unwrap();
        let value = disposition.to_str().unwrap();
        assert!(value.starts_with("attachment; filename=\""));
        assert!(value.contains("filename*=UTF-8''"));
        assert!(!value.contains('\n'));
        assert!(!value.contains('\r'));
    }
}
