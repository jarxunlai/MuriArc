use std::{io::Cursor, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use muriarc_application::{
    CancelGenotypingBatchCommand, CommitGenotypingBatchCommand, CreateGenotypingBatchCommand,
    SetGenotypingBatchPreviewCommand, cancel_genotyping_batch, commit_genotyping_batch,
    create_genotyping_batch, set_genotyping_batch_preview,
};
use muriarc_core::{
    AnimalFilter, Attachment, GenotypingBatch, GenotypingBatchFilter, GenotypingBatchReceipt,
    GenotypingBatchStatus, GenotypingRecord, Permission,
};
use muriarc_data::AttachmentFiles;
use muriarc_importer::{
    AnimalDirectory, GenotypingFieldMapping, GenotypingImportPreview, genotyping_template_csv,
    preview_genotyping, read_csv, read_xlsx,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, collection,
    ensure_lab, item, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/genotyping-batches", get(list_batches).post(create_batch))
        .route("/genotyping-batches/template.csv", get(download_template))
        .route("/genotyping-batches/{id}", get(get_batch))
        .route("/genotyping-records/{id}/batch", get(get_record_batch))
        .route("/genotyping-batches/{id}/records", get(list_batch_records))
        .route("/genotyping-batches/{id}/preview", post(preview_batch))
        .route("/genotyping-batches/{id}/commit", post(commit_batch))
        .route("/genotyping-batches/{id}/cancel", post(cancel_batch))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchListQuery {
    project_id: Option<Uuid>,
    status: Option<GenotypingBatchStatus>,
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessQuery {
    project_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBatchRequest {
    project_id: Option<Uuid>,
    batch_number: String,
    genotype_definition_id: Uuid,
    assessed_at: DateTime<Utc>,
    method: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBatchRequest {
    project_id: Option<Uuid>,
    expected_revision: i64,
    source_attachment_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitBatchRequest {
    project_id: Option<Uuid>,
    expected_revision: i64,
    preview_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelBatchRequest {
    project_id: Option<Uuid>,
    expected_revision: i64,
    reason: String,
}

#[derive(Debug, Serialize)]
struct BatchPreviewResponse {
    batch: GenotypingBatch,
    preview: GenotypingImportPreview,
}

#[derive(Debug, Serialize)]
struct BatchDetailResponse {
    batch: GenotypingBatch,
    records: Vec<GenotypingRecord>,
}

async fn list_batches(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<BatchListQuery>,
) -> Result<Json<CollectionResponse<GenotypingBatch>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut batches = store(
        state.store.list_genotyping_batches(&GenotypingBatchFilter {
            lab_id: principal.lab_id,
            project_id: query.project_id,
            status: query.status,
        }),
        &metadata,
    )
    .await?;
    truncate(&mut batches, collection_limit(query.limit, &metadata)?);
    Ok(collection(batches, &metadata))
}

async fn create_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateBatchRequest>,
) -> Result<(StatusCode, Json<ItemResponse<GenotypingBatch>>), ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let audit = principal.audit_context(&metadata);
    let batch = application(
        create_genotyping_batch(
            state.store.as_ref(),
            CreateGenotypingBatchCommand {
                lab_id: principal.lab_id,
                project_id: payload.project_id,
                batch_number: payload.batch_number,
                genotype_definition_id: payload.genotype_definition_id,
                assessed_at: payload.assessed_at,
                method: payload.method,
                notes: payload.notes,
                created_by: Some(principal.user_id),
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(batch, &metadata)))
}

async fn get_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<BatchDetailResponse>>, ApiError> {
    let batch = visible_batch(
        &state,
        &principal,
        &metadata,
        id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let records = store(state.store.list_genotyping_batch_records(id), &metadata).await?;
    Ok(item(BatchDetailResponse { batch, records }, &metadata))
}

async fn get_record_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Option<GenotypingBatch>>>, ApiError> {
    let record = store(state.store.get_genotyping_record(id), &metadata).await?;
    ensure_lab(record.lab_id, &principal, &metadata)?;
    if query.project_id.is_some() && query.project_id != record.project_id {
        return Err(ApiError::not_found("genotyping record was not found")
            .with_request_id(metadata.request_id));
    }
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        record.animal_id,
        record.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let batch = store(state.store.find_genotyping_batch_for_record(id), &metadata).await?;
    if let Some(batch) = batch.as_ref() {
        ensure_lab(batch.lab_id, &principal, &metadata)?;
        if batch.project_id != record.project_id {
            return Err(ApiError::internal().with_request_id(metadata.request_id));
        }
    }
    Ok(item(batch, &metadata))
}

async fn list_batch_records(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<CollectionResponse<GenotypingRecord>>, ApiError> {
    visible_batch(
        &state,
        &principal,
        &metadata,
        id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let records = store(state.store.list_genotyping_batch_records(id), &metadata).await?;
    Ok(collection(records, &metadata))
}

async fn preview_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<PreviewBatchRequest>,
) -> Result<Json<ItemResponse<BatchPreviewResponse>>, ApiError> {
    let mut batch = visible_batch(
        &state,
        &principal,
        &metadata,
        id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    if batch.status != GenotypingBatchStatus::Draft {
        return Err(ApiError::conflict("genotyping batch is no longer a draft")
            .with_request_id(metadata.request_id));
    }
    let source = source_attachment(
        &state,
        &principal,
        &metadata,
        &batch,
        payload.source_attachment_id,
    )
    .await?;
    let preview = parse_preview(&state, &metadata, &batch, &source).await?;
    if preview.can_confirm() {
        let row_count = i64::try_from(preview.accepted_rows.len()).map_err(|_| {
            ApiError::validation("genotyping preview has too many rows")
                .with_request_id(metadata.request_id.clone())
        })?;
        batch = application(
            set_genotyping_batch_preview(
                state.store.as_ref(),
                SetGenotypingBatchPreviewCommand {
                    batch_id: batch.id,
                    expected_revision: payload.expected_revision,
                    source_attachment_id: source.id,
                    preview_hash: preview.preview_hash.clone(),
                    row_count,
                    now: Utc::now(),
                },
                &principal.audit_context(&metadata),
            ),
            &metadata,
        )
        .await?;
    }
    Ok(item(BatchPreviewResponse { batch, preview }, &metadata))
}

async fn commit_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<CommitBatchRequest>,
) -> Result<Json<ItemResponse<GenotypingBatchReceipt>>, ApiError> {
    let batch = visible_batch(
        &state,
        &principal,
        &metadata,
        id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let source_id = batch.source_attachment_id.ok_or_else(|| {
        ApiError::conflict("genotyping batch has no confirmed preview")
            .with_request_id(metadata.request_id.clone())
    })?;
    let source = source_attachment(&state, &principal, &metadata, &batch, source_id).await?;
    let preview = parse_preview(&state, &metadata, &batch, &source).await?;
    if !preview.can_confirm()
        || preview.preview_hash != payload.preview_hash
        || batch.preview_hash.as_deref() != Some(payload.preview_hash.as_str())
    {
        return Err(ApiError::conflict(
            "genotyping source changed or no longer matches the confirmed preview",
        )
        .with_request_id(metadata.request_id));
    }
    let rows = preview
        .accepted_rows
        .into_iter()
        .map(|row| muriarc_core::GenotypingBatchRecordInput {
            animal_id: row.animal_id,
            state: row.state,
            notes: row.notes,
        })
        .collect();
    let receipt = application(
        commit_genotyping_batch(
            state.store.as_ref(),
            CommitGenotypingBatchCommand {
                batch_id: batch.id,
                expected_revision: payload.expected_revision,
                preview_hash: payload.preview_hash,
                rows,
                now: Utc::now(),
            },
            &principal.audit_context(&metadata),
        ),
        &metadata,
    )
    .await?;
    Ok(item(receipt, &metadata))
}

async fn cancel_batch(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<CancelBatchRequest>,
) -> Result<Json<ItemResponse<GenotypingBatch>>, ApiError> {
    visible_batch(
        &state,
        &principal,
        &metadata,
        id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let batch = application(
        cancel_genotyping_batch(
            state.store.as_ref(),
            CancelGenotypingBatchCommand {
                batch_id: id,
                expected_revision: payload.expected_revision,
                reason: payload.reason,
                now: Utc::now(),
            },
            &principal.audit_context(&metadata),
        ),
        &metadata,
    )
    .await?;
    Ok(item(batch, &metadata))
}

async fn visible_batch(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
    requested_project_id: Option<Uuid>,
    permission: Permission,
) -> Result<GenotypingBatch, ApiError> {
    let batch = store(state.store.get_genotyping_batch(id), metadata).await?;
    ensure_lab(batch.lab_id, principal, metadata)?;
    if requested_project_id != batch.project_id {
        return Err(ApiError::not_found("genotyping batch was not found")
            .with_request_id(metadata.request_id.clone()));
    }
    scope::optional_project_permission(state, principal, metadata, batch.project_id, permission)
        .await?;
    Ok(batch)
}

async fn source_attachment(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    batch: &GenotypingBatch,
    attachment_id: Uuid,
) -> Result<Attachment, ApiError> {
    let attachment = store(state.store.get_attachment(attachment_id), metadata).await?;
    if attachment.lab_id != principal.lab_id
        || attachment.project_id != batch.project_id
        || attachment.entity_type != "genotyping_batch"
        || attachment.entity_id != batch.id
        || attachment.meta.deleted_at.is_some()
        || attachment
            .media_type
            .as_deref()
            .is_some_and(|value| value.starts_with("image/"))
    {
        return Err(
            ApiError::not_found("genotyping source attachment was not found")
                .with_request_id(metadata.request_id.clone()),
        );
    }
    Ok(attachment)
}

async fn parse_preview(
    state: &AppState,
    metadata: &RequestMetadata,
    batch: &GenotypingBatch,
    source: &Attachment,
) -> Result<GenotypingImportPreview, ApiError> {
    let root = attachment_root(state, metadata)?;
    let bytes = AttachmentFiles::new(root.as_ref())
        .read_verified_bytes(source)
        .await
        .map_err(|_| {
            ApiError::conflict("genotyping source attachment failed integrity verification")
                .with_request_id(metadata.request_id.clone())
        })?;
    let extension = source
        .file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let table = match extension.as_str() {
        "csv" => read_csv(Cursor::new(bytes)),
        "xlsx" => read_xlsx(Cursor::new(bytes)),
        _ => {
            return Err(
                ApiError::validation("genotyping result table must be a CSV or XLSX file")
                    .with_request_id(metadata.request_id.clone()),
            );
        }
    }
    .map_err(|error| {
        ApiError::validation(format!(
            "genotyping result table could not be parsed: {error}"
        ))
        .with_request_id(metadata.request_id.clone())
    })?;
    let animals = store(
        state.store.list_animals(&AnimalFilter {
            lab_id: batch.lab_id,
            project_id: batch.project_id,
            ..AnimalFilter::default()
        }),
        metadata,
    )
    .await?;
    let directory = AnimalDirectory::from_entries(
        animals
            .into_iter()
            .map(|animal| (animal.display_id, animal.id)),
    )
    .map_err(|_| ApiError::internal().with_request_id(metadata.request_id.clone()))?;
    let mapping = GenotypingFieldMapping::infer(&table.headers);
    Ok(preview_genotyping(&table, &mapping, &directory))
}

fn attachment_root<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a Arc<std::path::PathBuf>, ApiError> {
    state.attachment_root.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "attachment_transport_disabled",
            "shared attachment storage is not configured",
        )
        .with_request_id(metadata.request_id.clone())
    })
}

async fn download_template() -> Response {
    let mut response = Body::from(genotyping_template_csv()).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_static(
            "attachment; filename=\"muriarc-genotyping-batch-template.csv\"",
        ),
    );
    response
}
