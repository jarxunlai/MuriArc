use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateExperimentEventCommand, CreateObservationDefinitionCommand, RecordObservationCommand,
    ReviseObservationValueCommand, create_experiment_event, create_observation_definition,
    record_observation, revise_observation_value,
};
use muriarc_core::{
    ExperimentEvent, Observation, ObservationDefinition, ObservationFilter, ObservationPolicy,
    ObservationSubjectType, ObservationValueData, ObservationValueRecord, ObservationValueType,
    Permission,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, collection,
    ensure_lab, item, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/experiment-events",
            get(list_experiment_events).post(create_event),
        )
        .route("/experiment-events/{id}", get(get_experiment_event))
        .route(
            "/observation-definitions",
            get(list_observation_definitions).post(create_definition),
        )
        .route(
            "/observation-definitions/{id}",
            get(get_observation_definition),
        )
        .route(
            "/observations",
            get(list_observations).post(create_observation),
        )
        .route("/observations/{id}", get(get_observation))
        .route("/observations/{id}/values", get(list_observation_values))
        .route(
            "/observations/{id}/revisions",
            axum::routing::post(revise_observation),
        )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentResourceQuery {
    experiment_id: Uuid,
    limit: Option<usize>,
}

async fn list_experiment_events(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ExperimentResourceQuery>,
) -> Result<Json<CollectionResponse<ExperimentEvent>>, ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        query.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    let mut events = store(
        state.store.list_experiment_events(query.experiment_id),
        &metadata,
    )
    .await?;
    truncate(&mut events, collection_limit(query.limit, &metadata)?);
    Ok(collection(events, &metadata))
}

async fn get_experiment_event(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<ExperimentEvent>>, ApiError> {
    let event = store(state.store.get_experiment_event(id), &metadata).await?;
    ensure_lab(event.lab_id, &principal, &metadata)?;
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        event.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    Ok(item(event, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateEventRequest {
    experiment_id: Uuid,
    event_key: String,
    label: String,
    occurred_at: Option<DateTime<Utc>>,
    #[serde(default = "empty_object")]
    details: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

async fn create_event(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateEventRequest>,
) -> Result<(StatusCode, Json<ItemResponse<ExperimentEvent>>), ApiError> {
    let experiment = scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let event = application(
        create_experiment_event(
            state.store.as_ref(),
            CreateExperimentEventCommand {
                lab_id: principal.lab_id,
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                event_key: payload.event_key,
                label: payload.label,
                occurred_at: payload.occurred_at.unwrap_or(now),
                details: payload.details,
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(event, &metadata)))
}

async fn list_observation_definitions(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ExperimentResourceQuery>,
) -> Result<Json<CollectionResponse<ObservationDefinition>>, ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        query.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    let mut definitions = store(
        state
            .store
            .list_observation_definitions(query.experiment_id),
        &metadata,
    )
    .await?;
    truncate(&mut definitions, collection_limit(query.limit, &metadata)?);
    Ok(collection(definitions, &metadata))
}

async fn get_observation_definition(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<ObservationDefinition>>, ApiError> {
    let definition = store(state.store.get_observation_definition(id), &metadata).await?;
    ensure_lab(definition.lab_id, &principal, &metadata)?;
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        definition.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    Ok(item(definition, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDefinitionRequest {
    experiment_id: Uuid,
    key: String,
    label: String,
    value_type: ObservationValueType,
    unit: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    policy: ObservationPolicy,
}

async fn create_definition(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateDefinitionRequest>,
) -> Result<(StatusCode, Json<ItemResponse<ObservationDefinition>>), ApiError> {
    let experiment = scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let audit = principal.audit_context(&metadata);
    let definition = application(
        create_observation_definition(
            state.store.as_ref(),
            CreateObservationDefinitionCommand {
                lab_id: principal.lab_id,
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                key: payload.key,
                label: payload.label,
                value_type: payload.value_type,
                unit: payload.unit,
                categories: payload.categories,
                policy: payload.policy,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(definition, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationListQuery {
    experiment_id: Uuid,
    experiment_event_id: Option<Uuid>,
    subject_type: Option<ObservationSubjectType>,
    subject_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_observations(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ObservationListQuery>,
) -> Result<Json<CollectionResponse<Observation>>, ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        query.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    let mut observations = store(
        state.store.list_observations(&ObservationFilter {
            experiment_id: query.experiment_id,
            experiment_event_id: query.experiment_event_id,
            subject_type: query.subject_type,
            subject_id: query.subject_id,
        }),
        &metadata,
    )
    .await?;
    truncate(&mut observations, collection_limit(query.limit, &metadata)?);
    Ok(collection(observations, &metadata))
}

async fn get_observation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Observation>>, ApiError> {
    let observation = store(state.store.get_observation(id), &metadata).await?;
    ensure_lab(observation.lab_id, &principal, &metadata)?;
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        observation.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    Ok(item(observation, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateObservationRequest {
    experiment_id: Uuid,
    experiment_event_id: Uuid,
    definition_id: Uuid,
    subject_type: ObservationSubjectType,
    subject_id: Uuid,
    #[serde(default = "empty_object")]
    context: Value,
    value: ObservationValueData,
    recorded_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
struct RecordedObservationResponse {
    observation: Observation,
    value: ObservationValueRecord,
}

async fn create_observation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateObservationRequest>,
) -> Result<(StatusCode, Json<ItemResponse<RecordedObservationResponse>>), ApiError> {
    let experiment = scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let recorded = application(
        record_observation(
            state.store.as_ref(),
            RecordObservationCommand {
                lab_id: principal.lab_id,
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                experiment_event_id: payload.experiment_event_id,
                definition_id: payload.definition_id,
                subject_type: payload.subject_type,
                subject_id: payload.subject_id,
                context: payload.context,
                value: payload.value,
                recorded_at: payload.recorded_at.unwrap_or(now),
                recorded_by: Some(principal.user_id),
                notes: payload.notes,
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        item(
            RecordedObservationResponse {
                observation: recorded.observation,
                value: recorded.value,
            },
            &metadata,
        ),
    ))
}

async fn list_observation_values(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<CollectionResponse<ObservationValueRecord>>, ApiError> {
    let observation = store(state.store.get_observation(id), &metadata).await?;
    ensure_lab(observation.lab_id, &principal, &metadata)?;
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        observation.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    let values = store(state.store.list_observation_values(id), &metadata).await?;
    Ok(collection(values, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviseObservationRequest {
    expected_revision: i64,
    value: ObservationValueData,
    recorded_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

async fn revise_observation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ReviseObservationRequest>,
) -> Result<Json<ItemResponse<RecordedObservationResponse>>, ApiError> {
    let before = store(state.store.get_observation(id), &metadata).await?;
    ensure_lab(before.lab_id, &principal, &metadata)?;
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        before.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let revised = application(
        revise_observation_value(
            state.store.as_ref(),
            ReviseObservationValueCommand {
                observation_id: id,
                expected_revision: payload.expected_revision,
                value: payload.value,
                recorded_at: payload.recorded_at.unwrap_or(now),
                recorded_by: Some(principal.user_id),
                notes: payload.notes,
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(
        RecordedObservationResponse {
            observation: revised.observation,
            value: revised.value,
        },
        &metadata,
    ))
}
