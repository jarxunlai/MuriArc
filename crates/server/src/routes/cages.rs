use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_application::{CreateCageCommand, create_cage};
use muriarc_core::{Cage, CageKind, Permission};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/cages", get(list).post(create))
        .route("/cages/{id}", get(get_one))
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<CollectionResponse<Cage>>, ApiError> {
    authorize(
        &principal,
        Permission::ReadAnimal,
        query.project_id,
        &metadata,
    )?;
    let cages = match query.project_id {
        Some(project_id) => {
            let project = store(state.store.get_project(project_id), &metadata).await?;
            ensure_lab(project.lab_id, &principal, &metadata)?;
            store(
                state
                    .store
                    .list_cages_for_project(principal.lab_id, project_id),
                &metadata,
            )
            .await?
        }
        None => store(state.store.list_cages(principal.lab_id), &metadata).await?,
    };
    Ok(collection(cages, &metadata))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessQuery {
    project_id: Option<Uuid>,
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<AccessQuery>,
) -> Result<Json<ItemResponse<Cage>>, ApiError> {
    authorize(
        &principal,
        Permission::ReadAnimal,
        query.project_id,
        &metadata,
    )?;
    let cage = store(state.store.get_cage(id), &metadata).await?;
    ensure_lab(cage.lab_id, &principal, &metadata)?;
    if let Some(project_id) = query.project_id {
        let visible = store(
            state
                .store
                .list_cages_for_project(principal.lab_id, project_id),
            &metadata,
        )
        .await?
        .into_iter()
        .any(|candidate| candidate.id == cage.id);
        if !visible {
            return Err(ApiError::not_found(format!("cage {id} was not found"))
                .with_request_id(metadata.request_id));
        }
    }
    Ok(item(cage, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    section: String,
    display_id: String,
    location: Option<String>,
    kind: Option<CageKind>,
    capacity: Option<i32>,
    sort_order: Option<i32>,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Cage>>), ApiError> {
    authorize(&principal, Permission::ManageCage, None, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let cage = application(
        create_cage(
            state.store.as_ref(),
            CreateCageCommand {
                lab_id: principal.lab_id,
                section: payload.section,
                display_id: payload.display_id,
                location: payload.location,
                kind: payload.kind.unwrap_or(CageKind::Standard),
                capacity: payload.capacity.unwrap_or(5),
                sort_order: payload.sort_order.unwrap_or_default(),
                now: chrono::Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(cage, &metadata)))
}
