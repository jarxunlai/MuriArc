use chrono::{DateTime, Utc};
use muriarc_core::{AuditContext, MuriArcStore, Project};
use uuid::Uuid;

use crate::ApplicationResult;
use crate::validation::{normalized_optional, normalized_required};

pub const MAX_PROJECT_NAME_CHARS: usize = 256;
pub const MAX_PROJECT_DESCRIPTION_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectCommand {
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_project(
    store: &dyn MuriArcStore,
    command: CreateProjectCommand,
    audit: &AuditContext,
) -> ApplicationResult<Project> {
    let name = normalized_required("project.name", command.name, MAX_PROJECT_NAME_CHARS)?;
    let mut project = Project::new(command.lab_id, name, command.now)?;
    project.description = normalized_optional(
        "project.description",
        command.description,
        MAX_PROJECT_DESCRIPTION_CHARS,
    )?;
    store.create_project(&project, audit).await?;
    Ok(project)
}
