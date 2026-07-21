use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use muriarc_application::{
    AssignAnimalsToProjectCommand, CreateProjectCommand, RemoveAnimalsFromProjectCommand,
    assign_animals_to_project, create_project, remove_animals_from_project,
};
use muriarc_core::{
    Permission, Project, ProjectAnimalAssignment, ProjectAnimalAssignmentFilter,
    ProjectAnimalAssignmentRemoval,
};
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
        .route(
            "/projects/{id}/animal-assignments",
            get(list_animal_assignments)
                .post(assign_animals)
                .delete(remove_animals),
        )
}

async fn list_animal_assignments(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<CollectionResponse<ProjectAnimalAssignment>>, ApiError> {
    authorize(
        &principal,
        Permission::ReadAnimal,
        Some(project_id),
        &metadata,
    )?;
    let project = store(state.store.get_project(project_id), &metadata).await?;
    ensure_lab(project.lab_id, &principal, &metadata)?;
    let assignments = store(
        state
            .store
            .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                lab_id: principal.lab_id,
                project_id: Some(project_id),
                animal_id: None,
            }),
        &metadata,
    )
    .await?;
    Ok(collection(assignments, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignAnimalsRequest {
    animal_ids: Vec<Uuid>,
    reason: Option<String>,
}

async fn assign_animals(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<AssignAnimalsRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Vec<ProjectAnimalAssignment>>>), ApiError> {
    authorize(
        &principal,
        Permission::ManageProjectAnimals,
        Some(project_id),
        &metadata,
    )?;
    let project = store(state.store.get_project(project_id), &metadata).await?;
    ensure_lab(project.lab_id, &principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let assignments = application(
        assign_animals_to_project(
            state.store.as_ref(),
            AssignAnimalsToProjectCommand {
                lab_id: principal.lab_id,
                project_id,
                animal_ids: payload.animal_ids,
                assigned_by: Some(principal.user_id),
                reason: payload.reason,
                now: chrono::Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(assignments, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveAnimalsRequest {
    assignments: Vec<RemoveAssignmentRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveAssignmentRequest {
    assignment_id: Uuid,
    expected_revision: i64,
}

async fn remove_animals(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RemoveAnimalsRequest>,
) -> Result<Json<ItemResponse<Vec<ProjectAnimalAssignment>>>, ApiError> {
    authorize(
        &principal,
        Permission::ManageProjectAnimals,
        Some(project_id),
        &metadata,
    )?;
    let project = store(state.store.get_project(project_id), &metadata).await?;
    ensure_lab(project.lab_id, &principal, &metadata)?;
    let active = store(
        state
            .store
            .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                lab_id: principal.lab_id,
                project_id: Some(project_id),
                animal_id: None,
            }),
        &metadata,
    )
    .await?;
    let active_ids = active
        .into_iter()
        .map(|assignment| assignment.id)
        .collect::<std::collections::BTreeSet<_>>();
    if payload
        .assignments
        .iter()
        .any(|assignment| !active_ids.contains(&assignment.assignment_id))
    {
        return Err(
            ApiError::not_found("project animal assignment was not found")
                .with_request_id(metadata.request_id),
        );
    }
    let audit = principal.audit_context(&metadata);
    let removed = application(
        remove_animals_from_project(
            state.store.as_ref(),
            RemoveAnimalsFromProjectCommand {
                removals: payload
                    .assignments
                    .into_iter()
                    .map(|assignment| ProjectAnimalAssignmentRemoval {
                        assignment_id: assignment.assignment_id,
                        expected_revision: assignment.expected_revision,
                    })
                    .collect(),
                now: chrono::Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(removed, &metadata))
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
