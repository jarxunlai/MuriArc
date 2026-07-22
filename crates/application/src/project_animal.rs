use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use muriarc_core::{
    AuditContext, MuriArcStore, ProjectAnimalAssignment, ProjectAnimalAssignmentRemoval,
};
use uuid::Uuid;

use crate::validation::ensure_max_bytes;
use crate::{ApplicationError, ApplicationResult};

pub const MAX_PROJECT_ANIMAL_BATCH: usize = 100;
pub const MAX_PROJECT_ANIMAL_REASON_BYTES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignAnimalsToProjectCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub animal_ids: Vec<Uuid>,
    pub assigned_by: Option<Uuid>,
    pub reason: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn assign_animals_to_project(
    store: &dyn MuriArcStore,
    command: AssignAnimalsToProjectCommand,
    audit: &AuditContext,
) -> ApplicationResult<Vec<ProjectAnimalAssignment>> {
    validate_batch_size(command.animal_ids.len())?;
    if command
        .animal_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        != command.animal_ids.len()
    {
        return Err(ApplicationError::Validation(
            "project animal selection must not contain duplicates".to_owned(),
        ));
    }
    let reason = command
        .reason
        .map(|reason| reason.trim().to_owned())
        .filter(|reason| !reason.is_empty());
    if let Some(reason) = &reason {
        ensure_max_bytes(
            "project_animal_assignment.reason",
            reason,
            MAX_PROJECT_ANIMAL_REASON_BYTES,
        )?;
    }
    let assignments = command
        .animal_ids
        .into_iter()
        .map(|animal_id| {
            ProjectAnimalAssignment::new(
                command.lab_id,
                command.project_id,
                animal_id,
                command.assigned_by,
                reason.clone(),
                command.now,
            )
        })
        .collect::<Vec<_>>();
    Ok(store.assign_animals_to_project(&assignments, audit).await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveAnimalsFromProjectCommand {
    pub removals: Vec<ProjectAnimalAssignmentRemoval>,
    pub now: DateTime<Utc>,
}

pub async fn remove_animals_from_project(
    store: &dyn MuriArcStore,
    command: RemoveAnimalsFromProjectCommand,
    audit: &AuditContext,
) -> ApplicationResult<Vec<ProjectAnimalAssignment>> {
    validate_batch_size(command.removals.len())?;
    if command
        .removals
        .iter()
        .map(|removal| removal.assignment_id)
        .collect::<BTreeSet<_>>()
        .len()
        != command.removals.len()
    {
        return Err(ApplicationError::Validation(
            "project animal removal must not contain duplicates".to_owned(),
        ));
    }
    if command
        .removals
        .iter()
        .any(|removal| removal.expected_revision < 1)
    {
        return Err(ApplicationError::Validation(
            "project animal removal revision must be positive".to_owned(),
        ));
    }
    Ok(store
        .remove_animals_from_project(&command.removals, command.now, audit)
        .await?)
}

fn validate_batch_size(size: usize) -> ApplicationResult<()> {
    if size == 0 {
        return Err(ApplicationError::Validation(
            "at least one project animal must be selected".to_owned(),
        ));
    }
    if size > MAX_PROJECT_ANIMAL_BATCH {
        return Err(ApplicationError::Validation(format!(
            "project animal batch cannot contain more than {MAX_PROJECT_ANIMAL_BATCH} animals"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_duplicate_and_oversized_batches() {
        assert!(validate_batch_size(0).is_err());
        assert!(validate_batch_size(MAX_PROJECT_ANIMAL_BATCH + 1).is_err());
        assert!(validate_batch_size(MAX_PROJECT_ANIMAL_BATCH).is_ok());
    }
}
