use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_application::{
    CreateExperimentCommand, TransitionExperimentCommand, create_experiment, transition_experiment,
};
use muriarc_core::{Experiment, ExperimentFilter, ExperimentStatus, Permission};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, scope, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/experiments", get(list).post(create))
        .route("/experiments/{id}", get(get_one))
        .route("/experiments/{id}/complete", axum::routing::post(complete))
        .route("/experiments/{id}/cancel", axum::routing::post(cancel))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    project_id: Uuid,
    status: Option<ExperimentStatus>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<Experiment>>, ApiError> {
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadExperiment,
    )
    .await?;
    let experiments = store(
        state.store.list_experiments(&ExperimentFilter {
            project_id: query.project_id,
            status: query.status,
        }),
        &metadata,
    )
    .await?;
    Ok(collection(experiments, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Experiment>>, ApiError> {
    let experiment = store(state.store.get_experiment(id), &metadata).await?;
    ensure_lab(experiment.lab_id, &principal, &metadata)?;
    authorize(
        &principal,
        Permission::ReadExperiment,
        Some(experiment.project_id),
        &metadata,
    )?;
    Ok(item(experiment, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    project_id: Uuid,
    name: String,
    description: Option<String>,
    template_version_id: Option<Uuid>,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Experiment>>), ApiError> {
    authorize(
        &principal,
        Permission::WriteExperiment,
        Some(payload.project_id),
        &metadata,
    )?;
    let audit = principal.audit_context(&metadata);
    let experiment = application(
        create_experiment(
            state.store.as_ref(),
            CreateExperimentCommand {
                lab_id: principal.lab_id,
                project_id: payload.project_id,
                template_version_id: payload.template_version_id,
                name: payload.name,
                description: payload.description,
                starts_at: None,
                now: chrono::Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(experiment, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionRequest {
    expected_revision: i64,
}

async fn complete(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<TransitionRequest>,
) -> Result<Json<ItemResponse<Experiment>>, ApiError> {
    transition(
        &state,
        &principal,
        &metadata,
        id,
        ExperimentStatus::Completed,
        payload.expected_revision,
    )
    .await
}

async fn cancel(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<TransitionRequest>,
) -> Result<Json<ItemResponse<Experiment>>, ApiError> {
    transition(
        &state,
        &principal,
        &metadata,
        id,
        ExperimentStatus::Cancelled,
        payload.expected_revision,
    )
    .await
}

async fn transition(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
    target: ExperimentStatus,
    expected_revision: i64,
) -> Result<Json<ItemResponse<Experiment>>, ApiError> {
    let experiment = store(state.store.get_experiment(id), metadata).await?;
    ensure_lab(experiment.lab_id, principal, metadata)?;
    authorize(
        principal,
        Permission::WriteExperiment,
        Some(experiment.project_id),
        metadata,
    )?;
    let audit = principal.audit_context(metadata);
    let transitioned = application(
        transition_experiment(
            state.store.as_ref(),
            TransitionExperimentCommand {
                id,
                target,
                expected_revision,
                occurred_at: chrono::Utc::now(),
            },
            &audit,
        ),
        metadata,
    )
    .await?;
    Ok(item(transitioned, metadata))
}
