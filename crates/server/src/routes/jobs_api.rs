use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_ai::SOURCE_IMPORT_JOB_BINDING_KEY;
use muriarc_application::JobReadView;
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
) -> Result<(StatusCode, Json<ItemResponse<JobReadView>>), ApiError> {
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
    Ok((status, item(JobReadView::from(outcome.job), &metadata)))
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<JobReadView>>, ApiError> {
    let mut jobs = state
        .jobs
        .list(principal.lab_id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    jobs.retain(|job| can_read_job(&principal, job));
    Ok(collection(
        jobs.into_iter().map(JobReadView::from).collect(),
        &metadata,
    ))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<JobReadView>>, ApiError> {
    let job = state
        .jobs
        .get(id)
        .await
        .map_err(|error| job_error(error, &metadata))?;
    ensure_lab(job.lab_id, &principal, &metadata)?;
    if is_source_import_job(&job) && job.created_by != principal.user_id {
        return Err(ApiError::not_found("job was not found").with_request_id(metadata.request_id));
    }
    if !can_read_job(&principal, &job) {
        return Err(ApiError::forbidden().with_request_id(metadata.request_id));
    }
    Ok(item(JobReadView::from(job), &metadata))
}

fn can_read_job(principal: &AuthPrincipal, job: &Job) -> bool {
    if is_source_import_job(job) {
        job.created_by == principal.user_id
    } else {
        job.created_by == principal.user_id
            || principal.can(Permission::ReadExperiment, job.project_id)
    }
}

fn is_source_import_job(job: &Job) -> bool {
    job.idempotency_key.starts_with("ai-source-import:")
        || job
            .result
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .is_some_and(|result| result.contains_key(SOURCE_IMPORT_JOB_BINDING_KEY))
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
    use muriarc_core::{LabRole, ProjectRole};
    use serde_json::json;

    use super::*;

    fn job(
        owner_id: Uuid,
        lab_id: Uuid,
        project_id: Option<Uuid>,
        idempotency_key: &str,
        result: Option<serde_json::Value>,
    ) -> Job {
        Job {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            created_by: owner_id,
            kind: JobKind::Import,
            status: JobStatus::AwaitingConfirmation,
            idempotency_key: idempotency_key.to_owned(),
            progress_current: 2,
            progress_total: Some(3),
            result,
            error_report: Some(json!({"private_error": "must-not-leak"})),
            cancellation_requested: false,
            meta: RecordMeta::new(chrono::Utc::now()),
        }
    }

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

    #[test]
    fn public_job_projection_omits_private_payloads_and_idempotency() {
        let binding_id = Uuid::new_v4();
        let value = serde_json::to_value(JobReadView::from(job(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            "ai-source-import:private-key",
            Some(json!({
                (SOURCE_IMPORT_JOB_BINDING_KEY): {
                    "attachment_id": binding_id,
                    "conversation_id": Uuid::new_v4(),
                },
                "preview_rows": [{"display_id": "PRIVATE-ANIMAL"}],
            })),
        )))
        .unwrap();

        let object = value.as_object().unwrap();
        for forbidden in [
            "idempotency_key",
            "result",
            "error_report",
            "created_by",
            "lab_id",
            "meta",
        ] {
            assert!(!object.contains_key(forbidden), "{forbidden} leaked");
        }
        let serialized = value.to_string();
        for forbidden in [
            SOURCE_IMPORT_JOB_BINDING_KEY,
            "attachment_id",
            "preview_rows",
            "PRIVATE-ANIMAL",
            "private_error",
        ] {
            assert!(!serialized.contains(forbidden), "{forbidden} leaked");
        }
        assert!(!serialized.contains(&binding_id.to_string()));
        assert_eq!(value["result_available"], true);
        assert_eq!(value["error_report_available"], true);
    }

    #[test]
    fn source_import_jobs_are_owner_only_even_for_project_and_lab_readers() {
        let lab_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let owner_id = Uuid::new_v4();
        let viewer = AuthPrincipal::human(Uuid::new_v4(), "Viewer", lab_id, [])
            .with_project_role(project_id, ProjectRole::Viewer);
        let animal_manager = AuthPrincipal::human(
            Uuid::new_v4(),
            "Animal manager",
            lab_id,
            [LabRole::AnimalManager],
        );
        let owner = AuthPrincipal::human(owner_id, "Owner", lab_id, []);
        let binding = Some(json!({
            (SOURCE_IMPORT_JOB_BINDING_KEY): {
                "attachment_id": Uuid::new_v4(),
            },
        }));
        let project_source = job(
            owner_id,
            lab_id,
            Some(project_id),
            "ai-source-import:project-source",
            binding.clone(),
        );
        let lab_source = job(
            owner_id,
            lab_id,
            None,
            "ai-source-import:lab-source",
            binding,
        );
        let project_job = job(
            owner_id,
            lab_id,
            Some(project_id),
            "ordinary-project-import",
            None,
        );
        let lab_job = job(owner_id, lab_id, None, "ordinary-lab-import", None);

        assert!(can_read_job(&owner, &project_source));
        assert!(can_read_job(&owner, &lab_source));
        assert!(!can_read_job(&viewer, &project_source));
        assert!(!can_read_job(&animal_manager, &project_source));
        assert!(!can_read_job(&animal_manager, &lab_source));
        assert!(can_read_job(&viewer, &project_job));
        assert!(can_read_job(&animal_manager, &project_job));
        assert!(can_read_job(&animal_manager, &lab_job));
    }
}
