use std::{io, path::Path, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use futures_util::TryStreamExt;
use muriarc_core::{ImportCommitResult, Job, JobKind, JobStatus, Permission, RecordMeta};
use muriarc_data::{
    AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata, DataError, DataFiles,
    ExportFormat, ImportKind, ImportRemapJobResult, artifact_metadata, build_lab_snapshot,
    export_animals_scoped,
};
use muriarc_importer::{AnimalExportFilter, FieldMapping, MeasurementFieldMapping};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::io::{ReaderStream, StreamReader};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, ItemResponse, authorize, ensure_lab, item, job_error, scope,
};

const MAX_DATA_JSON_BYTES: usize = 1024 * 1024;

pub(super) fn router() -> Router<AppState> {
    let upload = Router::new()
        .route("/data/imports", post(preview_import))
        // The handler streams the raw body into DataFiles, whose independent
        // 32 MiB limit is enforced while bytes are written. Disabling Axum's
        // default 2 MiB extractor limit avoids buffering or premature cutoff.
        .layer(DefaultBodyLimit::disable());
    let json_and_download = Router::new()
        .route("/data/imports/{id}/remap", post(remap_import))
        .route("/data/imports/{id}/confirm", post(confirm_import))
        .route("/data/imports/{id}/cancel", post(cancel_import))
        .route("/data/exports", post(create_export))
        .route("/data/snapshots", post(create_snapshot))
        .route("/data/artifacts/{id}", get(download_artifact))
        .layer(DefaultBodyLimit::max(MAX_DATA_JSON_BYTES));
    upload.merge(json_and_download)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewQuery {
    file_name: String,
    idempotency_key: String,
    #[serde(default)]
    import_kind: ImportKind,
    experiment_id: Option<Uuid>,
}

async fn preview_import(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<PreviewQuery>,
    body: Body,
) -> Result<(StatusCode, Json<ItemResponse<AnimalImportPreviewResponse>>), ApiError> {
    validate_idempotency_key(&query.idempotency_key, &metadata)?;
    let (project_id, experiment_id) = match query.import_kind {
        ImportKind::Animal => {
            if query.experiment_id.is_some() {
                return Err(validation(
                    "animal imports must not specify experiment_id",
                    &metadata,
                ));
            }
            authorize(&principal, Permission::ImportData, None, &metadata)?;
            (None, None)
        }
        ImportKind::Measurement => {
            let experiment_id = query.experiment_id.ok_or_else(|| {
                validation("measurement imports require experiment_id", &metadata)
            })?;
            let experiment = state
                .store
                .get_experiment(experiment_id)
                .await
                .map_err(ApiError::from_store)
                .map_err(|error| error.with_request_id(metadata.request_id.clone()))?;
            ensure_lab(experiment.lab_id, &principal, &metadata)?;
            authorize(
                &principal,
                Permission::ImportData,
                Some(experiment.project_id),
                &metadata,
            )?;
            (Some(experiment.project_id), Some(experiment_id))
        }
    };
    let files = data_files(&state, &metadata)?;
    let stream = body
        .into_data_stream()
        .map_err(|error| io::Error::other(error.to_string()));
    let reader = StreamReader::new(stream);

    let requested = Job {
        id: Uuid::new_v4(),
        lab_id: principal.lab_id,
        project_id,
        created_by: principal.user_id,
        kind: JobKind::Import,
        status: JobStatus::Parsing,
        idempotency_key: query.idempotency_key,
        progress_current: 0,
        progress_total: Some(3),
        result: None,
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(Utc::now()),
    };
    let audit = principal.audit_context(&metadata);
    let outcome = state
        .jobs
        .create(requested, audit.clone())
        .await
        .map_err(|error| job_error(error, &metadata))?;
    let mut job = outcome.job;
    ensure_owned_job(&job, &principal, JobKind::Import, &metadata)?;
    if job.project_id != project_id {
        return Err(conflict(
            "idempotency key belongs to a different import scope",
            &metadata,
        ));
    }
    if !outcome.created {
        files
            .write_upload(job.id, &query.file_name, reader)
            .await
            .map_err(|error| data_error(error, &metadata))?;
        if job.status != JobStatus::AwaitingConfirmation {
            return Err(conflict(
                "existing import job is not awaiting confirmation",
                &metadata,
            ));
        }
        let preview = match query.import_kind {
            ImportKind::Animal => (&files
                .read_pending_import(job.id)
                .await
                .map_err(|error| data_error(error, &metadata))?)
                .into(),
            ImportKind::Measurement => {
                let pending = files
                    .read_pending_measurement_import(job.id)
                    .await
                    .map_err(|error| data_error(error, &metadata))?;
                if Some(pending.experiment_id) != experiment_id {
                    return Err(conflict(
                        "idempotency key belongs to a different experiment",
                        &metadata,
                    ));
                }
                (&pending).into()
            }
        };
        return Ok((StatusCode::OK, item(preview, &metadata)));
    }

    let operation = async {
        files.write_upload(job.id, &query.file_name, reader).await?;
        match query.import_kind {
            ImportKind::Animal => Ok::<_, DataError>(
                (&files
                    .preview_animal_import(&job, state.store.as_ref())
                    .await?)
                    .into(),
            ),
            ImportKind::Measurement => Ok((&files
                .preview_measurement_import(
                    &job,
                    experiment_id.expect("measurement import has an experiment"),
                    state.store.as_ref(),
                )
                .await?)
                .into()),
        }
    }
    .await;
    match operation {
        Ok(preview) => {
            transition_job(
                &state,
                &mut job,
                JobTransition {
                    status: JobStatus::AwaitingConfirmation,
                    progress_current: 2,
                    result: Some(
                        serde_json::to_value(&preview)
                            .map_err(|error| data_error(DataError::Json(error), &metadata))?,
                    ),
                    error_report: None,
                },
                &audit,
                &metadata,
            )
            .await?;
            Ok((StatusCode::CREATED, item(preview, &metadata)))
        }
        Err(error) => {
            let _ = transition_job(
                &state,
                &mut job,
                JobTransition {
                    status: JobStatus::Failed,
                    progress_current: 0,
                    result: None,
                    error_report: Some(json!({ "code": data_error_code(&error) })),
                },
                &audit,
                &metadata,
            )
            .await;
            Err(data_error(error, &metadata))
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemapRequest {
    mapping: FieldMapping,
    idempotency_key: String,
}

async fn remap_import(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RemapRequest>,
) -> Result<(StatusCode, Json<ItemResponse<AnimalImportPreviewResponse>>), ApiError> {
    validate_idempotency_key(&payload.idempotency_key, &metadata)?;
    let files = data_files(&state, &metadata)?;
    let mut previous = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_owned_job(&previous, &principal, JobKind::Import, &metadata)?;
    authorize(
        &principal,
        Permission::ImportData,
        previous.project_id,
        &metadata,
    )?;

    if let Some(existing) = state
        .jobs
        .list(principal.lab_id)
        .await
        .map_err(|error| job_error(error, &metadata))?
        .into_iter()
        .find(|job| {
            job.created_by == principal.user_id && job.idempotency_key == payload.idempotency_key
        })
    {
        let preview = exact_remap_replay(&existing, &previous, &payload.mapping, &metadata)?;
        return Ok((StatusCode::OK, item(preview, &metadata)));
    }

    if previous.status != JobStatus::AwaitingConfirmation || previous.cancellation_requested {
        return Err(conflict(
            "import job is not awaiting a replacement preview",
            &metadata,
        ));
    }

    let experiment_id = if previous.project_id.is_some() {
        let pending = files
            .read_pending_measurement_import(previous.id)
            .await
            .map_err(|error| data_error(error, &metadata))?;
        if pending.job_id != previous.id
            || pending.lab_id != previous.lab_id
            || pending.created_by != previous.created_by
            || Some(pending.project_id) != previous.project_id
        {
            return Err(ApiError::not_found("data object was not found")
                .with_request_id(metadata.request_id.clone()));
        }
        Some(pending.experiment_id)
    } else {
        let pending = files
            .read_pending_import(previous.id)
            .await
            .map_err(|error| data_error(error, &metadata))?;
        if pending.job_id != previous.id
            || pending.lab_id != previous.lab_id
            || pending.created_by != previous.created_by
            || pending.project_id != previous.project_id
        {
            return Err(ApiError::not_found("data object was not found")
                .with_request_id(metadata.request_id.clone()));
        }
        None
    };

    let requested = Job {
        id: Uuid::new_v4(),
        lab_id: previous.lab_id,
        project_id: previous.project_id,
        created_by: previous.created_by,
        kind: JobKind::Import,
        status: JobStatus::Parsing,
        idempotency_key: payload.idempotency_key,
        progress_current: 0,
        progress_total: Some(3),
        result: None,
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(Utc::now()),
    };
    let audit = principal.audit_context(&metadata);
    let outcome = state
        .jobs
        .create(requested, audit.clone())
        .await
        .map_err(|error| job_error(error, &metadata))?;
    let mut replacement = outcome.job;
    ensure_owned_job(&replacement, &principal, JobKind::Import, &metadata)?;
    if !outcome.created {
        let preview = exact_remap_replay(&replacement, &previous, &payload.mapping, &metadata)?;
        return Ok((StatusCode::OK, item(preview, &metadata)));
    }

    let canonical_mapping = payload.mapping;
    let operation = async {
        files.copy_upload(previous.id, replacement.id).await?;
        match experiment_id {
            Some(experiment_id) => Ok::<_, DataError>(
                (&files
                    .preview_measurement_import_with_mapping(
                        &replacement,
                        experiment_id,
                        state.store.as_ref(),
                        Some(MeasurementFieldMapping {
                            columns: canonical_mapping.columns.clone(),
                        }),
                    )
                    .await?)
                    .into(),
            ),
            None => Ok((&files
                .preview_animal_import_with_mapping(
                    &replacement,
                    state.store.as_ref(),
                    Some(canonical_mapping.clone()),
                )
                .await?)
                .into()),
        }
    }
    .await;

    let preview: AnimalImportPreviewResponse = match operation {
        Ok(preview) => preview,
        Err(error) => {
            discard_import_replacement(
                &state,
                files.as_ref(),
                replacement.id,
                JobStatus::Failed,
                data_error_code(&error),
                &audit,
                &metadata,
            )
            .await;
            return Err(data_error(error, &metadata));
        }
    };

    let remap_result = ImportRemapJobResult {
        source_job_id: previous.id,
        mapping: canonical_mapping,
        preview: preview.clone(),
    };
    let result_value = match serde_json::to_value(&remap_result) {
        Ok(value) => value,
        Err(error) => {
            discard_import_replacement(
                &state,
                files.as_ref(),
                replacement.id,
                JobStatus::Failed,
                "serialization_failed",
                &audit,
                &metadata,
            )
            .await;
            return Err(data_error(DataError::Json(error), &metadata));
        }
    };
    if let Err(error) = transition_job(
        &state,
        &mut replacement,
        JobTransition {
            status: JobStatus::AwaitingConfirmation,
            progress_current: 2,
            result: Some(result_value),
            error_report: None,
        },
        &audit,
        &metadata,
    )
    .await
    {
        discard_import_replacement(
            &state,
            files.as_ref(),
            replacement.id,
            JobStatus::Failed,
            "replacement_transition_failed",
            &audit,
            &metadata,
        )
        .await;
        return Err(error);
    }

    previous.cancellation_requested = true;
    let previous_progress = previous.progress_current;
    let previous_result = previous.result.clone();
    if let Err(error) = transition_job(
        &state,
        &mut previous,
        JobTransition {
            status: JobStatus::Cancelled,
            progress_current: previous_progress,
            result: previous_result,
            error_report: None,
        },
        &audit,
        &metadata,
    )
    .await
    {
        discard_import_replacement(
            &state,
            files.as_ref(),
            replacement.id,
            JobStatus::Cancelled,
            "replacement_aborted",
            &audit,
            &metadata,
        )
        .await;
        return Err(error);
    }

    if let Err(error) = files.clear_pending_import(previous.id).await {
        tracing::warn!(job_id = %previous.id, error = %error, "cancelled import pending cleanup failed");
    }
    if let Err(error) = files.clear_upload(previous.id).await {
        tracing::warn!(job_id = %previous.id, error = %error, "cancelled import upload cleanup failed");
    }
    Ok((StatusCode::CREATED, item(preview, &metadata)))
}

fn exact_remap_replay(
    replacement: &Job,
    source: &Job,
    mapping: &FieldMapping,
    metadata: &RequestMetadata,
) -> Result<AnimalImportPreviewResponse, ApiError> {
    if replacement.lab_id != source.lab_id
        || replacement.created_by != source.created_by
        || replacement.project_id != source.project_id
        || replacement.kind != JobKind::Import
        || replacement.status != JobStatus::AwaitingConfirmation
        || replacement.cancellation_requested
    {
        return Err(conflict(
            "idempotency key belongs to a different remap request",
            metadata,
        ));
    }
    let result = replacement
        .result
        .clone()
        .and_then(|value| serde_json::from_value::<ImportRemapJobResult>(value).ok())
        .ok_or_else(|| conflict("remap replay state is unavailable", metadata))?;
    if result.source_job_id != source.id || result.mapping != *mapping {
        return Err(conflict(
            "idempotency key belongs to a different remap request",
            metadata,
        ));
    }
    Ok(result.preview)
}

async fn discard_import_replacement(
    state: &AppState,
    files: &DataFiles,
    job_id: Uuid,
    status: JobStatus,
    code: &'static str,
    audit: &muriarc_core::AuditContext,
    metadata: &RequestMetadata,
) {
    if let Ok(mut job) = state.jobs.get(job_id).await {
        job.cancellation_requested = true;
        let progress = job.progress_current;
        let result = job.result.clone();
        let _ = transition_job(
            state,
            &mut job,
            JobTransition {
                status,
                progress_current: progress,
                result,
                error_report: Some(json!({ "code": code })),
            },
            audit,
            metadata,
        )
        .await;
    }
    let _ = files.clear_pending_import(job_id).await;
    let _ = files.clear_upload(job_id).await;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmRequest {
    preview_hash: String,
}

async fn confirm_import(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ConfirmRequest>,
) -> Result<Json<ItemResponse<ImportReceiptView>>, ApiError> {
    let files = data_files(&state, &metadata)?;
    let mut job = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_owned_job(&job, &principal, JobKind::Import, &metadata)?;
    authorize(
        &principal,
        Permission::ImportData,
        job.project_id,
        &metadata,
    )?;
    if job.status == JobStatus::Completed {
        let result = job
            .result
            .clone()
            .ok_or_else(|| conflict("completed import has no receipt", &metadata))
            .and_then(|value| {
                serde_json::from_value::<ImportCommitResult>(value)
                    .map_err(|_| conflict("completed import receipt is unavailable", &metadata))
            })?;
        if !result
            .preview_hash
            .eq_ignore_ascii_case(payload.preview_hash.trim())
        {
            return Err(conflict(
                "confirmed preview hash does not match the completed import",
                &metadata,
            ));
        }
        let mut view = ImportReceiptView::from_result(job.id, result);
        view.replayed = true;
        return Ok(item(view, &metadata));
    }
    if job.status != JobStatus::AwaitingConfirmation || job.cancellation_requested {
        return Err(conflict(
            "import job is not awaiting confirmation",
            &metadata,
        ));
    }
    let audit = principal.audit_context(&metadata);
    let receipt = if job.project_id.is_some() {
        files
            .confirm_measurement_import(
                &job,
                &payload.preview_hash,
                state.store.as_ref(),
                &audit,
                Utc::now(),
            )
            .await
    } else {
        files
            .confirm_animal_import(
                &job,
                &payload.preview_hash,
                state.store.as_ref(),
                &audit,
                Utc::now(),
            )
            .await
    }
    .map_err(|error| data_error(error, &metadata))?;
    transition_job(
        &state,
        &mut job,
        JobTransition {
            status: JobStatus::Completed,
            progress_current: 3,
            result: Some(
                serde_json::to_value(&receipt)
                    .map_err(|error| data_error(DataError::Json(error), &metadata))?,
            ),
            error_report: None,
        },
        &audit,
        &metadata,
    )
    .await?;
    let _ = files.clear_pending_import(job.id).await;
    let _ = files.clear_upload(job.id).await;
    Ok(item(
        ImportReceiptView::from_result(job.id, receipt),
        &metadata,
    ))
}

async fn cancel_import(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<()>>, ApiError> {
    let files = data_files(&state, &metadata)?;
    let mut job = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_owned_job(&job, &principal, JobKind::Import, &metadata)?;
    authorize(
        &principal,
        Permission::ImportData,
        job.project_id,
        &metadata,
    )?;
    if matches!(job.status, JobStatus::Completed | JobStatus::Writing) {
        return Err(conflict("import job can no longer be cancelled", &metadata));
    }
    if job.status != JobStatus::Cancelled {
        job.cancellation_requested = true;
        let audit = principal.audit_context(&metadata);
        let progress = job.progress_current;
        let result = job.result.clone();
        transition_job(
            &state,
            &mut job,
            JobTransition {
                status: JobStatus::Cancelled,
                progress_current: progress,
                result,
                error_report: None,
            },
            &audit,
            &metadata,
        )
        .await?;
    }
    files
        .clear_pending_import(job.id)
        .await
        .map_err(|error| data_error(error, &metadata))?;
    files
        .clear_upload(job.id)
        .await
        .map_err(|error| data_error(error, &metadata))?;
    Ok(item((), &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportRequest {
    format: ExportFormat,
    idempotency_key: String,
    project_id: Option<Uuid>,
}

async fn create_export(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<ExportRequest>,
) -> Result<(StatusCode, Json<ItemResponse<DataArtifactView>>), ApiError> {
    if let Some(project_id) = payload.project_id {
        scope::project_with_permission(
            &state,
            &principal,
            &metadata,
            project_id,
            Permission::ExportData,
        )
        .await?;
    } else {
        authorize(&principal, Permission::ExportData, None, &metadata)?;
    }
    validate_idempotency_key(&payload.idempotency_key, &metadata)?;
    create_artifact_job(
        state,
        principal,
        metadata,
        JobKind::Export,
        payload.project_id,
        payload.idempotency_key,
        move |job, state, _| {
            Box::pin(async move {
                let bytes = export_animals_scoped(
                    state.store.as_ref(),
                    job.lab_id,
                    job.project_id,
                    payload.format,
                    &AnimalExportFilter::default(),
                )
                .await?;
                let metadata = artifact_metadata(
                    job.id,
                    ArtifactKind::Export,
                    format!(
                        "muriarc-animals-{}.{}",
                        job.meta.created_at.format("%Y%m%d-%H%M%S"),
                        payload.format.extension()
                    ),
                    payload.format.media_type().to_owned(),
                    &bytes,
                    job.meta.created_at,
                )?;
                Ok((metadata, bytes))
            })
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRequest {
    idempotency_key: String,
}

async fn create_snapshot(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SnapshotRequest>,
) -> Result<(StatusCode, Json<ItemResponse<DataArtifactView>>), ApiError> {
    authorize(&principal, Permission::ExportData, None, &metadata)?;
    validate_idempotency_key(&payload.idempotency_key, &metadata)?;
    create_artifact_job(
        state,
        principal,
        metadata,
        JobKind::Snapshot,
        None,
        payload.idempotency_key,
        |job, state, attachment_root| {
            Box::pin(async move {
                let files = state.data_files.as_ref().expect("data state checked");
                let bytes = build_lab_snapshot(
                    state.store.as_ref(),
                    attachment_root,
                    job.id,
                    files.instance_id().await?,
                    job.lab_id,
                    Some(job.created_by),
                    job.meta.created_at,
                )
                .await?;
                let metadata = artifact_metadata(
                    job.id,
                    ArtifactKind::Snapshot,
                    format!(
                        "muriarc-snapshot-{}.muriarc.zip",
                        job.meta.created_at.format("%Y%m%d-%H%M%S")
                    ),
                    "application/vnd.muriarc.snapshot+zip".to_owned(),
                    &bytes,
                    job.meta.created_at,
                )?;
                Ok((metadata, bytes))
            })
        },
    )
    .await
}

type ArtifactFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<(ArtifactMetadata, Vec<u8>), DataError>>
            + Send
            + 'a,
    >,
>;

async fn create_artifact_job<F>(
    state: AppState,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    kind: JobKind,
    project_id: Option<Uuid>,
    idempotency_key: String,
    build: F,
) -> Result<(StatusCode, Json<ItemResponse<DataArtifactView>>), ApiError>
where
    F: for<'a> FnOnce(Job, &'a AppState, &'a Path) -> ArtifactFuture<'a>,
{
    let files = data_files(&state, &metadata)?;
    let attachment_root = attachment_root(&state, &metadata)?;
    let requested = Job {
        id: Uuid::new_v4(),
        lab_id: principal.lab_id,
        project_id,
        created_by: principal.user_id,
        kind,
        status: JobStatus::Writing,
        idempotency_key,
        progress_current: 0,
        progress_total: Some(1),
        result: None,
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(Utc::now()),
    };
    let audit = principal.audit_context(&metadata);
    let outcome = state
        .jobs
        .create(requested, audit.clone())
        .await
        .map_err(|error| job_error(error, &metadata))?;
    let mut job = outcome.job;
    ensure_owned_job(&job, &principal, kind, &metadata)?;
    if job.project_id != project_id {
        return Err(conflict(
            "idempotency key belongs to a different project scope",
            &metadata,
        ));
    }
    if !outcome.created {
        if job.status != JobStatus::Completed {
            return Err(conflict(
                "existing artifact job is not completed",
                &metadata,
            ));
        }
        let artifact = files
            .artifact_metadata(job.id)
            .await
            .map_err(|error| data_error(error, &metadata))?;
        return Ok((
            StatusCode::OK,
            item(DataArtifactView::from(artifact), &metadata),
        ));
    }
    let operation = async {
        let (artifact, bytes) = build(job.clone(), &state, attachment_root.as_ref()).await?;
        files.write_artifact(&artifact, &bytes).await?;
        Ok::<_, DataError>(artifact)
    }
    .await;
    match operation {
        Ok(artifact) => {
            transition_job(
                &state,
                &mut job,
                JobTransition {
                    status: JobStatus::Completed,
                    progress_current: 1,
                    result: Some(
                        serde_json::to_value(&artifact)
                            .map_err(|error| data_error(DataError::Json(error), &metadata))?,
                    ),
                    error_report: None,
                },
                &audit,
                &metadata,
            )
            .await?;
            Ok((
                StatusCode::CREATED,
                item(DataArtifactView::from(artifact), &metadata),
            ))
        }
        Err(error) => {
            let _ = transition_job(
                &state,
                &mut job,
                JobTransition {
                    status: JobStatus::Failed,
                    progress_current: 0,
                    result: None,
                    error_report: Some(json!({ "code": "artifact_failed" })),
                },
                &audit,
                &metadata,
            )
            .await;
            Err(data_error(error, &metadata))
        }
    }
}

async fn download_artifact(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    let files = data_files(&state, &metadata)?;
    let job = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_owned_job(&job, &principal, job.kind, &metadata)?;
    if !matches!(job.kind, JobKind::Export | JobKind::Snapshot)
        || job.status != JobStatus::Completed
    {
        return Err(
            ApiError::not_found("artifact was not found").with_request_id(metadata.request_id)
        );
    }
    authorize(
        &principal,
        Permission::ExportData,
        job.project_id,
        &metadata,
    )?;
    let opened = files
        .open_artifact(id)
        .await
        .map_err(|error| data_error(error, &metadata))?;
    let mut response = Body::from_stream(ReaderStream::new(opened.file)).into_response();
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&opened.metadata.media_type)
            .map_err(|_| data_error(DataError::CorruptState("artifact media type"), &metadata))?,
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&opened.metadata.size_bytes.to_string())
            .expect("u64 is a valid header value"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            opened.metadata.file_name
        ))
        .map_err(|_| data_error(DataError::CorruptState("artifact file name"), &metadata))?,
    );
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", opened.metadata.sha256))
            .expect("SHA-256 is a valid header value"),
    );
    Ok(response)
}

struct JobTransition {
    status: JobStatus,
    progress_current: i64,
    result: Option<Value>,
    error_report: Option<Value>,
}

async fn transition_job(
    state: &AppState,
    job: &mut Job,
    transition: JobTransition,
    audit: &muriarc_core::AuditContext,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    let expected_revision = job.meta.revision;
    job.status = transition.status;
    job.progress_current = transition.progress_current;
    job.result = transition.result;
    job.error_report = transition.error_report;
    job.meta.touch(Utc::now());
    state
        .jobs
        .update(job.clone(), expected_revision, audit.clone())
        .await
        .map_err(|error| job_error(error, metadata))
}

fn data_files<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a Arc<DataFiles>, ApiError> {
    state.data_files.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_transport_disabled",
            "shared data storage is not configured",
        )
        .with_request_id(metadata.request_id.clone())
    })
}

fn attachment_root<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a Arc<std::path::PathBuf>, ApiError> {
    state.attachment_root.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_transport_disabled",
            "shared attachment storage is not configured",
        )
        .with_request_id(metadata.request_id.clone())
    })
}

fn ensure_owned_job(
    job: &Job,
    principal: &AuthPrincipal,
    kind: JobKind,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if job.lab_id == principal.lab_id && job.created_by == principal.user_id && job.kind == kind {
        Ok(())
    } else {
        Err(ApiError::not_found("job was not found").with_request_id(metadata.request_id.clone()))
    }
}

fn validate_idempotency_key(value: &str, metadata: &RequestMetadata) -> Result<(), ApiError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 128
        || value.chars().any(char::is_control)
    {
        Err(validation(
            "idempotency_key must be 1-128 non-control characters without outer whitespace",
            metadata,
        ))
    } else {
        Ok(())
    }
}

fn validation(message: impl Into<String>, metadata: &RequestMetadata) -> ApiError {
    ApiError::validation(message).with_request_id(metadata.request_id.clone())
}

fn conflict(message: impl Into<String>, metadata: &RequestMetadata) -> ApiError {
    ApiError::conflict(message).with_request_id(metadata.request_id.clone())
}

fn data_error(error: DataError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        DataError::NotFound => ApiError::not_found("data object was not found"),
        DataError::UploadTooLarge(limit) => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            format!("upload exceeds the {limit}-byte limit"),
        ),
        DataError::Conflict(message) => ApiError::conflict(message),
        DataError::ScopeMismatch => ApiError::not_found("data object was not found"),
        DataError::PreviewHasErrors => ApiError::validation("preview contains blocking errors"),
        DataError::Plan(issues) => ApiError::validation("import plan contains blocking issues")
            .with_details(json!({ "issues": issues })),
        DataError::InvalidFileName
        | DataError::EmptyUpload
        | DataError::UnsupportedUpload(_)
        | DataError::ArtifactTooLarge(_)
        | DataError::Directory(_)
        | DataError::Attachment(_) => ApiError::validation(error.to_string()),
        DataError::Store(error) => ApiError::from_store(error),
        DataError::ChecksumMismatch(_)
        | DataError::CorruptState(_)
        | DataError::Import(_)
        | DataError::Snapshot(_)
        | DataError::Json(_)
        | DataError::Io(_) => {
            tracing::error!(error = %error, "server data transport failed");
            ApiError::internal()
        }
    };
    error.with_request_id(metadata.request_id.clone())
}

fn data_error_code(error: &DataError) -> &'static str {
    match error {
        DataError::UploadTooLarge(_) => "upload_too_large",
        DataError::Conflict(_) => "conflict",
        DataError::PreviewHasErrors | DataError::Plan(_) => "preview_blocked",
        DataError::InvalidFileName
        | DataError::EmptyUpload
        | DataError::UnsupportedUpload(_)
        | DataError::Directory(_)
        | DataError::Attachment(_) => "validation_error",
        _ => "data_failed",
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportReceiptView {
    job_id: String,
    preview_hash: String,
    committed_at: String,
    replayed: bool,
    counts: ImportCountsView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportCountsView {
    animals: usize,
    animal_events: usize,
    genotypes: usize,
    pedigrees: usize,
    measurements: usize,
}

impl ImportReceiptView {
    fn from_result(job_id: Uuid, result: ImportCommitResult) -> Self {
        Self {
            job_id: job_id.to_string(),
            preview_hash: result.preview_hash,
            committed_at: result.committed_at.to_rfc3339(),
            replayed: result.replayed,
            counts: ImportCountsView {
                animals: result.counts.animals,
                animal_events: result.counts.animal_events,
                genotypes: result.counts.genotypes,
                pedigrees: result.counts.pedigrees,
                measurements: result.counts.measurements,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataArtifactView {
    job_id: String,
    kind: &'static str,
    file_name: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
    download_url: String,
}

impl From<ArtifactMetadata> for DataArtifactView {
    fn from(metadata: ArtifactMetadata) -> Self {
        Self {
            job_id: metadata.job_id.to_string(),
            kind: match metadata.kind {
                ArtifactKind::Export => "export",
                ArtifactKind::Snapshot => "snapshot",
            },
            file_name: metadata.file_name,
            media_type: metadata.media_type,
            size_bytes: metadata.size_bytes,
            sha256: metadata.sha256,
            download_url: format!("/data/artifacts/{}", metadata.job_id),
        }
    }
}
