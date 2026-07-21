use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::{
    CreateAnimalCommand, CreateAnimalIdentifierScope, InitialGenotypingRecordInput,
    TransferAnimalsCommand, create_animal, transfer_animals,
};
use muriarc_core::{
    Animal, AnimalEvent, AnimalFilter, AnimalStatus, GenotypingState, Permission, Sex,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, scope, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/animals", get(list).post(create))
        .route("/animals/{id}", get(get_one))
        .route("/animals/{id}/events", get(list_events))
        .route("/animals/transfer", axum::routing::post(transfer))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    project_id: Option<Uuid>,
    cage_id: Option<Uuid>,
    status: Option<AnimalStatus>,
    q: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessQuery {
    project_id: Option<Uuid>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<Animal>>, ApiError> {
    authorize(
        &principal,
        Permission::ReadAnimal,
        query.project_id,
        &metadata,
    )?;
    let animals = store(
        state.store.list_animals(&AnimalFilter {
            lab_id: principal.lab_id,
            project_id: query.project_id,
            cage_id: query.cage_id,
            status: query.status,
            query: normalized_query(query.q, &metadata)?,
        }),
        &metadata,
    )
    .await?;
    Ok(collection(animals, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Animal>>, ApiError> {
    let animal = visible_animal(&state, &principal, &metadata, id, query.project_id).await?;
    Ok(item(animal, &metadata))
}

async fn list_events(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<CollectionResponse<AnimalEvent>>, ApiError> {
    visible_animal(&state, &principal, &metadata, id, query.project_id).await?;
    let mut events = store(state.store.list_animal_events(id), &metadata).await?;
    if let Some(project_id) = query.project_id {
        let can_read_unscoped = principal.is_lab_operator();
        events.retain(|event| {
            event.project_id == Some(project_id)
                || (can_read_unscoped && event.project_id.is_none())
        });
    }
    Ok(collection(events, &metadata))
}

async fn visible_animal(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Animal, ApiError> {
    authorize(principal, Permission::ReadAnimal, project_id, metadata)?;
    let animal = store(state.store.get_animal(id), metadata).await?;
    ensure_lab(animal.lab_id, principal, metadata)?;

    if let Some(project_id) = project_id {
        let visible = store(
            state.store.list_animals(&AnimalFilter {
                lab_id: principal.lab_id,
                project_id: Some(project_id),
                cage_id: None,
                status: None,
                query: None,
            }),
            metadata,
        )
        .await?
        .into_iter()
        .any(|candidate| candidate.id == id);
        if !visible {
            return Err(ApiError::not_found(format!("animal {id} was not found"))
                .with_request_id(metadata.request_id.clone()));
        }
    }
    Ok(animal)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferRequest {
    animal_ids: Vec<Uuid>,
    target_cage_id: Uuid,
    occurred_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

async fn transfer(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<TransferRequest>,
) -> Result<Json<ItemResponse<Vec<Animal>>>, ApiError> {
    authorize(&principal, Permission::ManageCage, None, &metadata)?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let animals = application(
        transfer_animals(
            state.store.as_ref(),
            TransferAnimalsCommand {
                lab_id: principal.lab_id,
                animal_ids: payload.animal_ids,
                target_cage_id: payload.target_cage_id,
                occurred_at: payload.occurred_at.unwrap_or(now),
                recorded_at: now,
                recorded_by: Some(principal.user_id),
                notes: payload.notes,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(animals, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    display_id: String,
    sex: Sex,
    project_id: Option<Uuid>,
    strain: Option<String>,
    birth_date: Option<NaiveDate>,
    legacy_id: Option<String>,
    #[serde(default)]
    initial_genotyping_records: Vec<InitialGenotypingRecordRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialGenotypingRecordRequest {
    genotype_definition_id: Uuid,
    #[serde(default = "expected_genotyping_state")]
    state: GenotypingState,
    assessed_at: Option<DateTime<Utc>>,
    method: Option<String>,
    notes: Option<String>,
}

const fn expected_genotyping_state() -> GenotypingState {
    GenotypingState::Expected
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Animal>>), ApiError> {
    let permission = if payload.initial_genotyping_records.iter().any(|record| {
        matches!(
            record.state,
            GenotypingState::Confirmed | GenotypingState::Rejected
        )
    }) {
        Permission::ManageBreeding
    } else {
        Permission::WriteAnimal
    };
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        permission,
    )
    .await?;
    let identifier_scope = payload
        .project_id
        .map(CreateAnimalIdentifierScope::Project)
        .unwrap_or(CreateAnimalIdentifierScope::Lab);
    let audit = principal.audit_context(&metadata);
    let animal = application(
        create_animal(
            state.store.as_ref(),
            CreateAnimalCommand {
                lab_id: principal.lab_id,
                identifier_scope,
                display_id: payload.display_id,
                sex: payload.sex,
                strain: payload.strain,
                birth_date: payload.birth_date,
                legacy_id: payload.legacy_id,
                initial_cage_id: None,
                initial_genotyping_records: payload
                    .initial_genotyping_records
                    .into_iter()
                    .map(|record| InitialGenotypingRecordInput {
                        genotype_definition_id: record.genotype_definition_id,
                        state: record.state,
                        assessed_at: record.assessed_at,
                        method: record.method,
                        notes: record.notes,
                    })
                    .collect(),
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(animal, &metadata)))
}

fn normalized_query(
    query: Option<String>,
    metadata: &RequestMetadata,
) -> Result<Option<String>, ApiError> {
    match query.map(|value| value.trim().to_owned()) {
        Some(value) if value.len() > 256 => {
            Err(ApiError::validation("query must not exceed 256 bytes")
                .with_request_id(metadata.request_id.clone()))
        }
        Some(value) if value.is_empty() => Ok(None),
        value => Ok(value),
    }
}
