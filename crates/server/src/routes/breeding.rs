use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::CreateAnimalIdentifierScope;
use muriarc_application::{
    CreateAnimalDraftInput, CreateBreedingLineCommand, CreateBreedingPairCommand,
    CreateColonyCommand, CreateLitterCommand, CreateMatingEventCommand, RegisterAnimalDraftCommand,
    breeding_prediction, create_breeding_line, create_breeding_pair, create_colony, create_litter,
    create_mating_event, register_animal_draft, retire_breeding_pair,
};
use muriarc_core::{
    AnimalDraft, BreedingLine, BreedingPair, Colony, Litter, LocusPrediction, MatingEvent,
    Permission, Sex,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/breeding-lines",
            get(list_breeding_lines).post(create_line),
        )
        .route("/breeding-lines/{id}", get(get_breeding_line))
        .route("/colonies", get(list_colonies).post(create_colony_route))
        .route("/colonies/{id}", get(get_colony))
        .route(
            "/breeding-pairs",
            get(list_breeding_pairs).post(create_pair),
        )
        .route("/breeding-pairs/{id}", get(get_breeding_pair))
        .route(
            "/breeding-pairs/{id}/retire",
            axum::routing::post(retire_pair),
        )
        .route(
            "/mating-events",
            get(list_mating_events).post(create_mating),
        )
        .route("/mating-events/{id}", get(get_mating_event))
        .route("/litters", get(list_litters).post(create_litter_route))
        .route("/litters/{id}", get(get_litter))
        .route("/animal-drafts", get(list_animal_drafts))
        .route("/animal-drafts/{id}", get(get_animal_draft))
        .route(
            "/animal-drafts/{id}/register",
            axum::routing::post(register_draft),
        )
        .route("/breeding-predictions", axum::routing::post(predict))
}

fn require_read(principal: &AuthPrincipal, metadata: &RequestMetadata) -> Result<(), ApiError> {
    authorize(principal, Permission::ReadAnimal, None, metadata)
}

fn require_write(principal: &AuthPrincipal, metadata: &RequestMetadata) -> Result<(), ApiError> {
    authorize(principal, Permission::ManageBreeding, None, metadata)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitQuery {
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColonyListQuery {
    breeding_line_id: Option<Uuid>,
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairListQuery {
    colony_id: Option<Uuid>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairResourceListQuery {
    breeding_pair_id: Uuid,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LitterDraftListQuery {
    litter_id: Uuid,
    limit: Option<usize>,
}

async fn list_breeding_lines(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<LimitQuery>,
) -> Result<Json<CollectionResponse<BreedingLine>>, ApiError> {
    require_read(&principal, &metadata)?;
    let mut lines = store(state.store.list_breeding_lines(principal.lab_id), &metadata).await?;
    truncate(&mut lines, collection_limit(query.limit, &metadata)?);
    Ok(collection(lines, &metadata))
}

async fn get_breeding_line(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<BreedingLine>>, ApiError> {
    require_read(&principal, &metadata)?;
    let line = store(state.store.get_breeding_line(id), &metadata).await?;
    ensure_lab(line.lab_id, &principal, &metadata)?;
    Ok(item(line, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLineRequest {
    name: String,
    description: Option<String>,
    genotype_definition_ids: Vec<Uuid>,
}

async fn create_line(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateLineRequest>,
) -> Result<(StatusCode, Json<ItemResponse<BreedingLine>>), ApiError> {
    require_write(&principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let line = application(
        create_breeding_line(
            state.store.as_ref(),
            CreateBreedingLineCommand {
                lab_id: principal.lab_id,
                name: payload.name,
                description: payload.description,
                genotype_definition_ids: payload.genotype_definition_ids,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(line, &metadata)))
}

async fn list_colonies(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ColonyListQuery>,
) -> Result<Json<CollectionResponse<Colony>>, ApiError> {
    require_read(&principal, &metadata)?;
    let mut colonies = store(
        state
            .store
            .list_colonies(principal.lab_id, query.breeding_line_id),
        &metadata,
    )
    .await?;
    truncate(&mut colonies, collection_limit(query.limit, &metadata)?);
    Ok(collection(colonies, &metadata))
}

async fn get_colony(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Colony>>, ApiError> {
    require_read(&principal, &metadata)?;
    let colony = store(state.store.get_colony(id), &metadata).await?;
    ensure_lab(colony.lab_id, &principal, &metadata)?;
    Ok(item(colony, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateColonyRequest {
    breeding_line_id: Uuid,
    name: String,
    description: Option<String>,
}

async fn create_colony_route(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateColonyRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Colony>>), ApiError> {
    require_write(&principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let colony = application(
        create_colony(
            state.store.as_ref(),
            CreateColonyCommand {
                lab_id: principal.lab_id,
                breeding_line_id: payload.breeding_line_id,
                name: payload.name,
                description: payload.description,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(colony, &metadata)))
}

async fn list_breeding_pairs(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<PairListQuery>,
) -> Result<Json<CollectionResponse<BreedingPair>>, ApiError> {
    require_read(&principal, &metadata)?;
    let mut pairs = store(
        state
            .store
            .list_breeding_pairs(principal.lab_id, query.colony_id),
        &metadata,
    )
    .await?;
    truncate(&mut pairs, collection_limit(query.limit, &metadata)?);
    Ok(collection(pairs, &metadata))
}

async fn get_breeding_pair(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<BreedingPair>>, ApiError> {
    require_read(&principal, &metadata)?;
    let pair = store(state.store.get_breeding_pair(id), &metadata).await?;
    ensure_lab(pair.lab_id, &principal, &metadata)?;
    Ok(item(pair, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePairRequest {
    project_id: Option<Uuid>,
    colony_id: Uuid,
    name: String,
    male_animal_id: Uuid,
    female_animal_ids: Vec<Uuid>,
    started_at: Option<DateTime<Utc>>,
}

async fn create_pair(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreatePairRequest>,
) -> Result<(StatusCode, Json<ItemResponse<BreedingPair>>), ApiError> {
    require_write(&principal, &metadata)?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        payload.male_animal_id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    for animal_id in &payload.female_animal_ids {
        scope::animal_with_permission(
            &state,
            &principal,
            &metadata,
            *animal_id,
            payload.project_id,
            Permission::ManageBreeding,
        )
        .await?;
    }
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let pair = application(
        create_breeding_pair(
            state.store.as_ref(),
            CreateBreedingPairCommand {
                lab_id: principal.lab_id,
                colony_id: payload.colony_id,
                name: payload.name,
                male_animal_id: payload.male_animal_id,
                female_animal_ids: payload.female_animal_ids,
                started_at: payload.started_at.unwrap_or(now),
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(pair, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetirePairRequest {
    expected_revision: i64,
    ended_at: Option<DateTime<Utc>>,
}

async fn retire_pair(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RetirePairRequest>,
) -> Result<Json<ItemResponse<BreedingPair>>, ApiError> {
    require_write(&principal, &metadata)?;
    let pair = store(state.store.get_breeding_pair(id), &metadata).await?;
    ensure_lab(pair.lab_id, &principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let pair = application(
        retire_breeding_pair(
            state.store.as_ref(),
            id,
            payload.expected_revision,
            payload.ended_at.unwrap_or_else(Utc::now),
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(pair, &metadata))
}

async fn list_mating_events(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<PairResourceListQuery>,
) -> Result<Json<CollectionResponse<MatingEvent>>, ApiError> {
    require_read(&principal, &metadata)?;
    let pair = store(
        state.store.get_breeding_pair(query.breeding_pair_id),
        &metadata,
    )
    .await?;
    ensure_lab(pair.lab_id, &principal, &metadata)?;
    let mut events = store(
        state.store.list_mating_events(query.breeding_pair_id),
        &metadata,
    )
    .await?;
    truncate(&mut events, collection_limit(query.limit, &metadata)?);
    Ok(collection(events, &metadata))
}

async fn get_mating_event(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<MatingEvent>>, ApiError> {
    require_read(&principal, &metadata)?;
    let event = store(state.store.get_mating_event(id), &metadata).await?;
    ensure_lab(event.lab_id, &principal, &metadata)?;
    Ok(item(event, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateMatingRequest {
    project_id: Option<Uuid>,
    breeding_pair_id: Uuid,
    male_animal_id: Uuid,
    female_animal_id: Uuid,
    occurred_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

async fn create_mating(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateMatingRequest>,
) -> Result<(StatusCode, Json<ItemResponse<MatingEvent>>), ApiError> {
    require_write(&principal, &metadata)?;
    for animal_id in [payload.male_animal_id, payload.female_animal_id] {
        scope::animal_with_permission(
            &state,
            &principal,
            &metadata,
            animal_id,
            payload.project_id,
            Permission::ManageBreeding,
        )
        .await?;
    }
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let event = application(
        create_mating_event(
            state.store.as_ref(),
            CreateMatingEventCommand {
                lab_id: principal.lab_id,
                breeding_pair_id: payload.breeding_pair_id,
                male_animal_id: payload.male_animal_id,
                female_animal_id: payload.female_animal_id,
                occurred_at: payload.occurred_at.unwrap_or(now),
                notes: payload.notes,
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(event, &metadata)))
}

async fn list_litters(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<PairResourceListQuery>,
) -> Result<Json<CollectionResponse<Litter>>, ApiError> {
    require_read(&principal, &metadata)?;
    let pair = store(
        state.store.get_breeding_pair(query.breeding_pair_id),
        &metadata,
    )
    .await?;
    ensure_lab(pair.lab_id, &principal, &metadata)?;
    let mut litters = store(state.store.list_litters(query.breeding_pair_id), &metadata).await?;
    truncate(&mut litters, collection_limit(query.limit, &metadata)?);
    Ok(collection(litters, &metadata))
}

async fn get_litter(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Litter>>, ApiError> {
    require_read(&principal, &metadata)?;
    let litter = store(state.store.get_litter(id), &metadata).await?;
    ensure_lab(litter.lab_id, &principal, &metadata)?;
    Ok(item(litter, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    temporary_label: String,
    sex: Sex,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLitterRequest {
    mating_event_id: Uuid,
    born_on: NaiveDate,
    size_total: i32,
    drafts: Vec<DraftRequest>,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreatedLitterResponse {
    litter: Litter,
    drafts: Vec<AnimalDraft>,
}

async fn create_litter_route(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateLitterRequest>,
) -> Result<(StatusCode, Json<ItemResponse<CreatedLitterResponse>>), ApiError> {
    require_write(&principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let created = application(
        create_litter(
            state.store.as_ref(),
            CreateLitterCommand {
                lab_id: principal.lab_id,
                mating_event_id: payload.mating_event_id,
                born_on: payload.born_on,
                size_total: payload.size_total,
                drafts: payload
                    .drafts
                    .into_iter()
                    .map(|draft| CreateAnimalDraftInput {
                        temporary_label: draft.temporary_label,
                        sex: draft.sex,
                    })
                    .collect(),
                notes: payload.notes,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        item(
            CreatedLitterResponse {
                litter: created.litter,
                drafts: created.drafts,
            },
            &metadata,
        ),
    ))
}

async fn list_animal_drafts(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<LitterDraftListQuery>,
) -> Result<Json<CollectionResponse<AnimalDraft>>, ApiError> {
    require_read(&principal, &metadata)?;
    let litter = store(state.store.get_litter(query.litter_id), &metadata).await?;
    ensure_lab(litter.lab_id, &principal, &metadata)?;
    let mut drafts = store(state.store.list_animal_drafts(query.litter_id), &metadata).await?;
    truncate(&mut drafts, collection_limit(query.limit, &metadata)?);
    Ok(collection(drafts, &metadata))
}

async fn get_animal_draft(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AnimalDraft>>, ApiError> {
    require_read(&principal, &metadata)?;
    let draft = store(state.store.get_animal_draft(id), &metadata).await?;
    ensure_lab(draft.lab_id, &principal, &metadata)?;
    Ok(item(draft, &metadata))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IdentifierScopeRequest {
    Lab,
    Project,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterDraftRequest {
    expected_revision: i64,
    identifier_scope: IdentifierScopeRequest,
    project_id: Option<Uuid>,
    display_id: String,
    strain: Option<String>,
    initial_cage_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct RegisteredDraftResponse {
    draft: AnimalDraft,
    animal: muriarc_core::Animal,
}

async fn register_draft(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RegisterDraftRequest>,
) -> Result<Json<ItemResponse<RegisteredDraftResponse>>, ApiError> {
    require_write(&principal, &metadata)?;
    authorize(
        &principal,
        Permission::WriteAnimal,
        payload.project_id,
        &metadata,
    )?;
    let identifier_scope = match payload.identifier_scope {
        IdentifierScopeRequest::Lab => CreateAnimalIdentifierScope::Lab,
        IdentifierScopeRequest::Project => {
            CreateAnimalIdentifierScope::Project(payload.project_id.ok_or_else(|| {
                ApiError::validation("project_id is required for project identifier scope")
                    .with_request_id(metadata.request_id.clone())
            })?)
        }
    };
    if let Some(project_id) = payload.project_id {
        scope::project_with_permission(
            &state,
            &principal,
            &metadata,
            project_id,
            Permission::WriteAnimal,
        )
        .await?;
    }
    let draft = store(state.store.get_animal_draft(id), &metadata).await?;
    ensure_lab(draft.lab_id, &principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let registered = application(
        register_animal_draft(
            state.store.as_ref(),
            RegisterAnimalDraftCommand {
                lab_id: principal.lab_id,
                draft_id: id,
                expected_revision: payload.expected_revision,
                identifier_scope,
                display_id: payload.display_id,
                strain: payload.strain,
                initial_cage_id: payload.initial_cage_id,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(
        RegisteredDraftResponse {
            draft: registered.draft,
            animal: registered.animal,
        },
        &metadata,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredictionRequest {
    male_genotype_definition_id: Uuid,
    female_genotype_definition_id: Uuid,
}

async fn predict(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<PredictionRequest>,
) -> Result<Json<ItemResponse<Vec<LocusPrediction>>>, ApiError> {
    require_read(&principal, &metadata)?;
    let male = store(
        state
            .store
            .get_genotype_definition(payload.male_genotype_definition_id),
        &metadata,
    )
    .await?;
    let female = store(
        state
            .store
            .get_genotype_definition(payload.female_genotype_definition_id),
        &metadata,
    )
    .await?;
    ensure_lab(male.lab_id, &principal, &metadata)?;
    ensure_lab(female.lab_id, &principal, &metadata)?;
    let prediction = application(
        breeding_prediction(
            state.store.as_ref(),
            payload.male_genotype_definition_id,
            payload.female_genotype_definition_id,
        ),
        &metadata,
    )
    .await?;
    Ok(item(prediction, &metadata))
}
