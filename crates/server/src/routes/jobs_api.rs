use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_core::{Job, JobKind, JobStatus, Permission, RecordMeta};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, CollectionResponse, ItemResponse, authorize, collection, ensure_lab, item,
    job_error,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list).post(create))
        .route("/jobs/{id}", get(get_one))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    project_id: Option<Uuid>,
    kind: JobKind,
    idempotency_key: String,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Job>>), ApiError> {
    authorize(
        &principal,
        job_permission(payload.kind),
        payload.project_id,
        &metadata,
    )?;
    validate_idempotency_key(&payload.idempotency_key, &metadata)?;

    let job = Job {
        id: Uuid::new_v4(),
        lab_id: principal.lab_id,
        project_id: payload.project_id,
        created_by: principal.user_id,
        kind: payload.kind,
        status: JobStatus::Queued,
        idempotency_key: payload.idempotency_key,
        progress_current: 0,
        progress_total: None,
        result: None,
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(chrono::Utc::now()),
    };
    let outcome = state
        .jobs
        .create(job, principal.audit_context(&metadata))
        .await
        .map_err(|error| job_error(error, &metadata))?;
    let status = if outcome.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, item(outcome.job, &metadata)))
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<Job>>, ApiError> {
    let mut jobs = state
        .jobs
        .list(principal.lab_id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    jobs.retain(|job| {
        job.created_by == principal.user_id
            || principal.can(Permission::ReadExperiment, job.project_id)
    });
    Ok(collection(jobs, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Job>>, ApiError> {
    let job = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_lab(job.lab_id, &principal, &metadata)?;
    if job.created_by != principal.user_id
        && !principal.can(Permission::ReadExperiment, job.project_id)
    {
        return Err(ApiError::forbidden().with_request_id(metadata.request_id));
    }
    Ok(item(job, &metadata))
}

fn job_permission(kind: JobKind) -> Permission {
    match kind {
        JobKind::Import | JobKind::BulkOperation => Permission::ImportData,
        JobKind::Export | JobKind::Snapshot => Permission::ExportData,
    }
}

fn validate_idempotency_key(key: &str, metadata: &RequestMetadata) -> Result<(), ApiError> {
    if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) || key.trim() != key {
        Err(ApiError::validation(
            "idempotency_key must be 1-128 non-control characters without outer whitespace",
        )
        .with_request_id(metadata.request_id.clone()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_validation_rejects_outer_whitespace() {
        let metadata = RequestMetadata {
            request_id: "test".into(),
            reason: None,
        };
        assert!(validate_idempotency_key(" retry ", &metadata).is_err());
        assert!(validate_idempotency_key("retry-1", &metadata).is_ok());
    }

    #[test]
    fn job_permissions_are_never_admin_permissions() {
        assert_eq!(job_permission(JobKind::Import), Permission::ImportData);
        assert_eq!(job_permission(JobKind::Export), Permission::ExportData);
        assert_eq!(
            job_permission(JobKind::BulkOperation),
            Permission::ImportData
        );
    }
}
