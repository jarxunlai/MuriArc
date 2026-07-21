use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateCohortCommand, CreateParticipationCommand, CreateProcedureCommand,
    CreateTemplateVersionCommand, PublishTemplateVersionCommand, TransitionParticipationCommand,
    create_cohort as create_cohort_use_case, create_participation as create_participation_use_case,
    create_procedure as create_procedure_use_case, create_template_version,
    publish_template_version, transition_participation as transition_participation_use_case,
};
use muriarc_core::{
    Cohort, ExperimentTemplateVersion, FieldValueType, Participation, ParticipationFilter,
    ParticipationStatus, Permission, Procedure, ProcedureStatus, TemplateField,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, collection,
    ensure_lab, item, scope, store,
    validation::{collection_limit, required_text, truncate, validation},
};

const MAX_TEMPLATE_KEY_BYTES: usize = 128;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/experiment-template-versions",
            get(list_templates).post(create_template),
        )
        .route("/experiment-template-versions/{id}", get(get_template))
        .route(
            "/experiment-template-versions/{id}/publish",
            axum::routing::post(publish_template),
        )
        .route("/cohorts", get(list_cohorts).post(create_cohort))
        .route(
            "/participations",
            get(list_participations).post(create_participation),
        )
        .route(
            "/participations/{id}/complete",
            axum::routing::post(complete_participation),
        )
        .route(
            "/participations/{id}/withdraw",
            axum::routing::post(withdraw_participation),
        )
        .route("/procedures", get(list_procedures).post(create_procedure))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateAccessQuery {
    project_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateListQuery {
    project_id: Option<Uuid>,
    template_key: Option<String>,
    limit: Option<usize>,
}

async fn list_templates(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<TemplateListQuery>,
) -> Result<Json<CollectionResponse<ExperimentTemplateVersion>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadExperiment,
    )
    .await?;
    let template_key = query
        .template_key
        .map(|key| validated_template_key(key, &metadata))
        .transpose()?;
    let mut templates = store(
        state
            .store
            .list_template_versions(principal.lab_id, template_key.as_deref()),
        &metadata,
    )
    .await?;
    truncate(&mut templates, collection_limit(query.limit, &metadata)?);
    Ok(collection(templates, &metadata))
}

async fn get_template(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<TemplateAccessQuery>,
) -> Result<Json<ItemResponse<ExperimentTemplateVersion>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadExperiment,
    )
    .await?;
    let template = store(state.store.get_template_version(id), &metadata).await?;
    ensure_lab(template.lab_id, &principal, &metadata)?;
    Ok(item(template, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateFieldRequest {
    key: String,
    label: String,
    value_type: FieldValueType,
    unit: Option<String>,
    required: bool,
    #[serde(default)]
    categories: Vec<String>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    display_order: i32,
    ai_writable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTemplateRequest {
    project_id: Option<Uuid>,
    template_key: String,
    version: i32,
    name: String,
    description: Option<String>,
    #[serde(default)]
    fields: Vec<TemplateFieldRequest>,
}

async fn create_template(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateTemplateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<ExperimentTemplateVersion>>), ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::DraftTemplate,
    )
    .await?;
    let now = Utc::now();
    let fields = payload
        .fields
        .into_iter()
        .map(|field| TemplateField {
            key: field.key,
            label: field.label,
            value_type: field.value_type,
            unit: field.unit,
            required: field.required,
            categories: field.categories,
            minimum: field.minimum,
            maximum: field.maximum,
            display_order: field.display_order,
            ai_writable: field.ai_writable,
        })
        .collect();
    let audit = principal.audit_context(&metadata);
    let template = application(
        create_template_version(
            state.store.as_ref(),
            CreateTemplateVersionCommand {
                lab_id: principal.lab_id,
                template_key: payload.template_key,
                version: payload.version,
                name: payload.name,
                description: payload.description,
                fields,
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(template, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishTemplateRequest {
    project_id: Option<Uuid>,
    expected_revision: i64,
}

async fn publish_template(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<PublishTemplateRequest>,
) -> Result<Json<ItemResponse<ExperimentTemplateVersion>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::PublishTemplate,
    )
    .await?;
    let template = store(state.store.get_template_version(id), &metadata).await?;
    ensure_lab(template.lab_id, &principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let published = application(
        publish_template_version(
            state.store.as_ref(),
            PublishTemplateVersionCommand {
                id,
                expected_revision: payload.expected_revision,
                published_by: principal.user_id,
                published_at: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(published, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CohortListQuery {
    experiment_id: Uuid,
    limit: Option<usize>,
}

async fn list_cohorts(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<CohortListQuery>,
) -> Result<Json<CollectionResponse<Cohort>>, ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        query.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    let mut cohorts = store(state.store.list_cohorts(query.experiment_id), &metadata).await?;
    truncate(&mut cohorts, collection_limit(query.limit, &metadata)?);
    Ok(collection(cohorts, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCohortRequest {
    experiment_id: Uuid,
    name: String,
    description: Option<String>,
}

async fn create_cohort(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateCohortRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Cohort>>), ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let audit = principal.audit_context(&metadata);
    let cohort = application(
        create_cohort_use_case(
            state.store.as_ref(),
            CreateCohortCommand {
                experiment_id: payload.experiment_id,
                name: payload.name,
                description: payload.description,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(cohort, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipationListQuery {
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    animal_id: Option<Uuid>,
    cohort_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_participations(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ParticipationListQuery>,
) -> Result<Json<CollectionResponse<Participation>>, ApiError> {
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadExperiment,
    )
    .await?;

    if let Some(experiment_id) = query.experiment_id {
        let experiment = scope::experiment_with_permission(
            &state,
            &principal,
            &metadata,
            experiment_id,
            Permission::ReadExperiment,
        )
        .await?;
        if experiment.project_id != query.project_id {
            return Err(ApiError::not_found("experiment was not found")
                .with_request_id(metadata.request_id));
        }
    }
    if let Some(animal_id) = query.animal_id {
        scope::animal_with_permission(
            &state,
            &principal,
            &metadata,
            animal_id,
            Some(query.project_id),
            Permission::ReadAnimal,
        )
        .await?;
    }

    let mut participations = store(
        state.store.list_participations(&ParticipationFilter {
            project_id: query.project_id,
            experiment_id: query.experiment_id,
            animal_id: query.animal_id,
            cohort_id: query.cohort_id,
        }),
        &metadata,
    )
    .await?;
    truncate(
        &mut participations,
        collection_limit(query.limit, &metadata)?,
    );
    Ok(collection(participations, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateParticipationRequest {
    experiment_id: Uuid,
    animal_id: Uuid,
    cohort_id: Option<Uuid>,
    enrolled_at: Option<DateTime<Utc>>,
}

async fn create_participation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateParticipationRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Participation>>), ApiError> {
    scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let animal = store(state.store.get_animal(payload.animal_id), &metadata).await?;
    ensure_lab(animal.lab_id, &principal, &metadata)?;

    if let Some(cohort_id) = payload.cohort_id {
        let belongs_to_experiment =
            store(state.store.list_cohorts(payload.experiment_id), &metadata)
                .await?
                .into_iter()
                .any(|cohort| cohort.id == cohort_id);
        if !belongs_to_experiment {
            return Err(
                ApiError::not_found("cohort was not found").with_request_id(metadata.request_id)
            );
        }
    }

    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let participation = application(
        create_participation_use_case(
            state.store.as_ref(),
            CreateParticipationCommand {
                experiment_id: payload.experiment_id,
                animal_id: payload.animal_id,
                cohort_id: payload.cohort_id,
                enrolled_at: payload.enrolled_at.unwrap_or(now),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(participation, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipationTransitionRequest {
    expected_revision: i64,
}

async fn complete_participation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ParticipationTransitionRequest>,
) -> Result<Json<ItemResponse<Participation>>, ApiError> {
    transition_participation(
        &state,
        &principal,
        &metadata,
        id,
        ParticipationStatus::Completed,
        payload.expected_revision,
    )
    .await
}

async fn withdraw_participation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ParticipationTransitionRequest>,
) -> Result<Json<ItemResponse<Participation>>, ApiError> {
    transition_participation(
        &state,
        &principal,
        &metadata,
        id,
        ParticipationStatus::Withdrawn,
        payload.expected_revision,
    )
    .await
}

async fn transition_participation(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
    target: ParticipationStatus,
    expected_revision: i64,
) -> Result<Json<ItemResponse<Participation>>, ApiError> {
    let participation = store(state.store.get_participation(id), metadata).await?;
    scope::experiment_with_permission(
        state,
        principal,
        metadata,
        participation.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    let audit = principal.audit_context(metadata);
    let transitioned = application(
        transition_participation_use_case(
            state.store.as_ref(),
            TransitionParticipationCommand {
                id,
                target,
                expected_revision,
                occurred_at: Utc::now(),
            },
            &audit,
        ),
        metadata,
    )
    .await?;
    Ok(item(transitioned, metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcedureListQuery {
    experiment_id: Uuid,
    animal_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_procedures(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ProcedureListQuery>,
) -> Result<Json<CollectionResponse<Procedure>>, ApiError> {
    let experiment = scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        query.experiment_id,
        Permission::ReadExperiment,
    )
    .await?;
    if let Some(animal_id) = query.animal_id {
        scope::animal_with_permission(
            &state,
            &principal,
            &metadata,
            animal_id,
            Some(experiment.project_id),
            Permission::ReadAnimal,
        )
        .await?;
    }
    let mut procedures = store(
        state
            .store
            .list_procedures(query.experiment_id, query.animal_id),
        &metadata,
    )
    .await?;
    truncate(&mut procedures, collection_limit(query.limit, &metadata)?);
    Ok(collection(procedures, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProcedureRequest {
    experiment_id: Uuid,
    animal_id: Option<Uuid>,
    name: String,
    scheduled_at: Option<DateTime<Utc>>,
    performed_at: Option<DateTime<Utc>>,
    status: Option<ProcedureStatus>,
    details: Option<Value>,
}

async fn create_procedure(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateProcedureRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Procedure>>), ApiError> {
    let experiment = scope::experiment_with_permission(
        &state,
        &principal,
        &metadata,
        payload.experiment_id,
        Permission::WriteExperiment,
    )
    .await?;
    if let Some(animal_id) = payload.animal_id {
        scope::animal_with_permission(
            &state,
            &principal,
            &metadata,
            animal_id,
            Some(experiment.project_id),
            Permission::ReadAnimal,
        )
        .await?;
        let is_participant = !store(
            state.store.list_participations(&ParticipationFilter {
                project_id: experiment.project_id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal_id),
                cohort_id: None,
            }),
            &metadata,
        )
        .await?
        .is_empty();
        if !is_participant {
            return Err(validation(
                "animal must participate in the experiment before an animal-specific procedure is created",
                &metadata,
            ));
        }
    }

    let audit = principal.audit_context(&metadata);
    let procedure = application(
        create_procedure_use_case(
            state.store.as_ref(),
            CreateProcedureCommand {
                experiment_id: payload.experiment_id,
                animal_id: payload.animal_id,
                name: payload.name,
                scheduled_at: payload.scheduled_at,
                performed_at: payload.performed_at,
                status: payload.status.unwrap_or(ProcedureStatus::Planned),
                details: payload.details.unwrap_or_else(|| serde_json::json!({})),
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(procedure, &metadata)))
}

fn validated_template_key(value: String, metadata: &RequestMetadata) -> Result<String, ApiError> {
    let value = required_text(value, "template_key", MAX_TEMPLATE_KEY_BYTES, metadata)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(validation(
            "template_key may contain only ASCII letters, digits, '.', '-' and '_'",
            metadata,
        ));
    }
    Ok(value)
}
