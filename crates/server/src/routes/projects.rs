use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_application::{CreateProjectCommand, create_project};
use muriarc_core::{Permission, Project};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, CollectionResponse, ItemResponse, application, authorize, collection,
    ensure_lab, item, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/projects", get(list).post(create))
        .route("/projects/{id}", get(get_one))
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<Project>>, ApiError> {
    let mut projects = store(state.store.list_projects(principal.lab_id), &metadata).await?;
    projects.retain(|project| principal.can(Permission::ReadExperiment, Some(project.id)));

    if projects.is_empty()
        && !principal.is_lab_operator()
        && principal.project_ids().next().is_none()
    {
        return Err(ApiError::forbidden().with_request_id(metadata.request_id));
    }
    Ok(collection(projects, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Project>>, ApiError> {
    authorize(&principal, Permission::ReadExperiment, Some(id), &metadata)?;
    let project = store(state.store.get_project(id), &metadata).await?;
    ensure_lab(project.lab_id, &principal, &metadata)?;
    Ok(item(project, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    name: String,
    description: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Project>>), ApiError> {
    authorize(&principal, Permission::ManageProject, None, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let project = application(
        create_project(
            state.store.as_ref(),
            CreateProjectCommand {
                lab_id: principal.lab_id,
                name: payload.name,
                description: payload.description,
                now: chrono::Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(project, &metadata)))
}
