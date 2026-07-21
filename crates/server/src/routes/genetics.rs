use std::collections::HashSet;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateAlleleCommand, CreateGeneLocusCommand, CreateGenotypeCommand,
    CreateGenotypeComponentInput, CreateGenotypeDefinitionCommand, CreateGenotypingRecordCommand,
    CreatePedigreeCommand, create_allele as create_allele_use_case, create_gene_locus,
    create_genotype as create_genotype_use_case, create_genotype_definition,
    create_genotyping_record, create_pedigree as create_pedigree_use_case,
};
use muriarc_core::{
    Allele, AnimalFilter, GeneLocus, Genotype, GenotypeComponentMode, GenotypeDefinition,
    GenotypingRecord, GenotypingState, ParentType, Pedigree, Permission,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, collection,
    ensure_lab, item, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/gene-loci", get(list_loci).post(create_locus))
        .route("/gene-loci/{id}", get(get_locus))
        .route("/alleles", get(list_alleles).post(create_allele))
        .route("/alleles/{id}", get(get_allele))
        .route("/genotypes", get(list_genotypes).post(create_genotype))
        .route("/genotypes/{id}", get(get_genotype))
        .route(
            "/genotype-definitions",
            get(list_genotype_definitions).post(create_genotype_definition_route),
        )
        .route("/genotype-definitions/{id}", get(get_genotype_definition))
        .route(
            "/genotyping-records",
            get(list_genotyping_records).post(create_genotyping_record_route),
        )
        .route("/genotyping-records/{id}", get(get_genotyping_record))
        .route("/pedigrees", get(list_pedigrees).post(create_pedigree))
        .route("/pedigrees/{id}", get(get_pedigree))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessQuery {
    project_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocusListQuery {
    project_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_loci(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<LocusListQuery>,
) -> Result<Json<CollectionResponse<GeneLocus>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut loci = store(state.store.list_gene_loci(principal.lab_id), &metadata).await?;
    truncate(&mut loci, collection_limit(query.limit, &metadata)?);
    Ok(collection(loci, &metadata))
}

async fn get_locus(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<GeneLocus>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let locus = visible_locus(&state, &principal, &metadata, id).await?;
    Ok(item(locus, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLocusRequest {
    project_id: Option<Uuid>,
    symbol: String,
    description: Option<String>,
}

async fn create_locus(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateLocusRequest>,
) -> Result<(StatusCode, Json<ItemResponse<GeneLocus>>), ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let audit = principal.audit_context(&metadata);
    let locus = application(
        create_gene_locus(
            state.store.as_ref(),
            CreateGeneLocusCommand {
                lab_id: principal.lab_id,
                symbol: payload.symbol,
                description: payload.description,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(locus, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlleleListQuery {
    locus_id: Uuid,
    project_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_alleles(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AlleleListQuery>,
) -> Result<Json<CollectionResponse<Allele>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    visible_locus(&state, &principal, &metadata, query.locus_id).await?;
    let mut alleles = store(state.store.list_alleles(query.locus_id), &metadata).await?;
    truncate(&mut alleles, collection_limit(query.limit, &metadata)?);
    Ok(collection(alleles, &metadata))
}

async fn get_allele(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Allele>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let allele = store(state.store.get_allele(id), &metadata).await?;
    visible_locus(&state, &principal, &metadata, allele.locus_id).await?;
    Ok(item(allele, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAlleleRequest {
    project_id: Option<Uuid>,
    locus_id: Uuid,
    symbol: String,
    description: Option<String>,
    #[serde(default)]
    is_wild_type: bool,
}

async fn create_allele(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateAlleleRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Allele>>), ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    visible_locus(&state, &principal, &metadata, payload.locus_id).await?;
    let audit = principal.audit_context(&metadata);
    let allele = application(
        create_allele_use_case(
            state.store.as_ref(),
            CreateAlleleCommand {
                locus_id: payload.locus_id,
                symbol: payload.symbol,
                description: payload.description,
                is_wild_type: payload.is_wild_type,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(allele, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimalResourceListQuery {
    animal_id: Uuid,
    project_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list_genotypes(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AnimalResourceListQuery>,
) -> Result<Json<CollectionResponse<Genotype>>, ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        query.animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut genotypes = store(state.store.list_genotypes(query.animal_id), &metadata).await?;
    truncate(&mut genotypes, collection_limit(query.limit, &metadata)?);
    Ok(collection(genotypes, &metadata))
}

async fn get_genotype(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Genotype>>, ApiError> {
    let genotype = store(state.store.get_genotype(id), &metadata).await?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        genotype.animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    visible_locus(&state, &principal, &metadata, genotype.locus_id).await?;
    Ok(item(genotype, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateGenotypeRequest {
    project_id: Option<Uuid>,
    animal_id: Uuid,
    locus_id: Uuid,
    allele_1_id: Option<Uuid>,
    allele_2_id: Option<Uuid>,
    assessed_at: Option<DateTime<Utc>>,
}

async fn create_genotype(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateGenotypeRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Genotype>>), ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        payload.animal_id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    visible_locus(&state, &principal, &metadata, payload.locus_id).await?;
    let audit = principal.audit_context(&metadata);
    let genotype = application(
        create_genotype_use_case(
            state.store.as_ref(),
            CreateGenotypeCommand {
                animal_id: payload.animal_id,
                locus_id: payload.locus_id,
                allele_1_id: payload.allele_1_id,
                allele_2_id: payload.allele_2_id,
                assessed_at: payload.assessed_at,
                project_id: payload.project_id,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(genotype, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenotypeDefinitionComponentRequest {
    locus_id: Uuid,
    allele_1_id: Uuid,
    allele_2_id: Option<Uuid>,
    mode: GenotypeComponentMode,
    display_order: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateGenotypeDefinitionRequest {
    project_id: Option<Uuid>,
    name: String,
    description: Option<String>,
    components: Vec<GenotypeDefinitionComponentRequest>,
}

async fn list_genotype_definitions(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<LocusListQuery>,
) -> Result<Json<CollectionResponse<GenotypeDefinition>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut definitions = store(
        state.store.list_genotype_definitions(principal.lab_id),
        &metadata,
    )
    .await?;
    truncate(&mut definitions, collection_limit(query.limit, &metadata)?);
    Ok(collection(definitions, &metadata))
}

async fn get_genotype_definition(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<GenotypeDefinition>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let definition = store(state.store.get_genotype_definition(id), &metadata).await?;
    ensure_lab(definition.lab_id, &principal, &metadata)?;
    Ok(item(definition, &metadata))
}

async fn create_genotype_definition_route(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateGenotypeDefinitionRequest>,
) -> Result<(StatusCode, Json<ItemResponse<GenotypeDefinition>>), ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let audit = principal.audit_context(&metadata);
    let definition = application(
        create_genotype_definition(
            state.store.as_ref(),
            CreateGenotypeDefinitionCommand {
                lab_id: principal.lab_id,
                name: payload.name,
                description: payload.description,
                components: payload
                    .components
                    .into_iter()
                    .map(|component| CreateGenotypeComponentInput {
                        locus_id: component.locus_id,
                        allele_1_id: component.allele_1_id,
                        allele_2_id: component.allele_2_id,
                        mode: component.mode,
                        display_order: component.display_order,
                    })
                    .collect(),
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(definition, &metadata)))
}

async fn list_genotyping_records(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AnimalResourceListQuery>,
) -> Result<Json<CollectionResponse<GenotypingRecord>>, ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        query.animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut records = store(
        state.store.list_genotyping_records(query.animal_id),
        &metadata,
    )
    .await?;
    truncate(&mut records, collection_limit(query.limit, &metadata)?);
    Ok(collection(records, &metadata))
}

async fn get_genotyping_record(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<GenotypingRecord>>, ApiError> {
    let record = store(state.store.get_genotyping_record(id), &metadata).await?;
    ensure_lab(record.lab_id, &principal, &metadata)?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        record.animal_id,
        query.project_id.or(record.project_id),
        Permission::ReadAnimal,
    )
    .await?;
    Ok(item(record, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateGenotypingRecordRequest {
    project_id: Option<Uuid>,
    animal_id: Uuid,
    genotype_definition_id: Uuid,
    state: GenotypingState,
    assessed_at: Option<DateTime<Utc>>,
    method: Option<String>,
    notes: Option<String>,
}

async fn create_genotyping_record_route(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateGenotypingRecordRequest>,
) -> Result<(StatusCode, Json<ItemResponse<GenotypingRecord>>), ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        payload.animal_id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    let definition = store(
        state
            .store
            .get_genotype_definition(payload.genotype_definition_id),
        &metadata,
    )
    .await?;
    ensure_lab(definition.lab_id, &principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let record = application(
        create_genotyping_record(
            state.store.as_ref(),
            CreateGenotypingRecordCommand {
                lab_id: principal.lab_id,
                project_id: payload.project_id,
                animal_id: payload.animal_id,
                genotype_definition_id: payload.genotype_definition_id,
                state: payload.state,
                assessed_at: payload.assessed_at,
                method: payload.method,
                notes: payload.notes,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(record, &metadata)))
}

async fn list_pedigrees(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AnimalResourceListQuery>,
) -> Result<Json<CollectionResponse<Pedigree>>, ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        query.animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let mut pedigrees = store(state.store.list_pedigrees(query.animal_id), &metadata).await?;
    if let Some(project_id) = query.project_id {
        let visible_ids: HashSet<_> = store(
            state.store.list_animals(&AnimalFilter {
                lab_id: principal.lab_id,
                project_id: Some(project_id),
                cage_id: None,
                status: None,
                query: None,
            }),
            &metadata,
        )
        .await?
        .into_iter()
        .map(|animal| animal.id)
        .collect();
        pedigrees.retain(|pedigree| visible_ids.contains(&pedigree.parent_id));
    }
    truncate(&mut pedigrees, collection_limit(query.limit, &metadata)?);
    Ok(collection(pedigrees, &metadata))
}

async fn get_pedigree(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Pedigree>>, ApiError> {
    let pedigree = store(state.store.get_pedigree(id), &metadata).await?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        pedigree.animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        pedigree.parent_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    Ok(item(pedigree, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePedigreeRequest {
    project_id: Option<Uuid>,
    animal_id: Uuid,
    parent_id: Uuid,
    parent_type: ParentType,
}

async fn create_pedigree(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreatePedigreeRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Pedigree>>), ApiError> {
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        payload.animal_id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;
    scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        payload.parent_id,
        payload.project_id,
        Permission::ManageBreeding,
    )
    .await?;

    let audit = principal.audit_context(&metadata);
    let pedigree = application(
        create_pedigree_use_case(
            state.store.as_ref(),
            CreatePedigreeCommand {
                animal_id: payload.animal_id,
                parent_id: payload.parent_id,
                parent_type: payload.parent_type,
                now: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(pedigree, &metadata)))
}

async fn visible_locus(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    id: Uuid,
) -> Result<GeneLocus, ApiError> {
    let locus = store(state.store.get_gene_locus(id), metadata).await?;
    ensure_lab(locus.lab_id, principal, metadata)?;
    Ok(locus)
}
