use muriarc_core::{Animal, AnimalFilter, Experiment, Permission, Project};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{authorize, ensure_lab, store};

pub(super) async fn project_with_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    project_id: Uuid,
    permission: Permission,
) -> Result<Project, ApiError> {
    authorize(principal, permission, Some(project_id), metadata)?;
    let project = store(state.store.get_project(project_id), metadata).await?;
    ensure_lab(project.lab_id, principal, metadata)?;
    Ok(project)
}

pub(super) async fn optional_project_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    project_id: Option<Uuid>,
    permission: Permission,
) -> Result<(), ApiError> {
    if let Some(project_id) = project_id {
        project_with_permission(state, principal, metadata, project_id, permission).await?;
    } else {
        authorize(principal, permission, None, metadata)?;
    }
    Ok(())
}

pub(super) async fn experiment_with_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    experiment_id: Uuid,
    permission: Permission,
) -> Result<Experiment, ApiError> {
    let experiment = store(state.store.get_experiment(experiment_id), metadata).await?;
    ensure_lab(experiment.lab_id, principal, metadata)?;
    project_with_permission(
        state,
        principal,
        metadata,
        experiment.project_id,
        permission,
    )
    .await?;
    Ok(experiment)
}

pub(super) async fn animal_with_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    animal_id: Uuid,
    project_id: Option<Uuid>,
    permission: Permission,
) -> Result<Animal, ApiError> {
    optional_project_permission(state, principal, metadata, project_id, permission).await?;
    let animal = store(state.store.get_animal(animal_id), metadata).await?;
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
        .any(|candidate| candidate.id == animal_id);
        if !visible {
            return Err(
                ApiError::not_found(format!("animal {animal_id} was not found"))
                    .with_request_id(metadata.request_id.clone()),
            );
        }
    }

    Ok(animal)
}
