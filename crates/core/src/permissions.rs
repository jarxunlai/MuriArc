use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, RecordMeta};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabRole {
    LabAdmin,
    AnimalManager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    ProjectAdmin,
    Editor,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub lab_role: Option<LabRole>,
    pub project_role: Option<ProjectRole>,
    pub meta: RecordMeta,
}

impl Membership {
    pub fn lab(lab_id: Uuid, user_id: Uuid, role: LabRole, now: DateTime<Utc>) -> Self {
        Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id: None,
            user_id,
            lab_role: Some(role),
            project_role: None,
            meta: RecordMeta::new(now),
        }
    }

    pub fn project(
        lab_id: Uuid,
        project_id: Uuid,
        user_id: Uuid,
        role: ProjectRole,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id: Some(project_id),
            user_id,
            lab_role: None,
            project_role: Some(role),
            meta: RecordMeta::new(now),
        }
    }

    pub fn validate_scope(&self) -> Result<(), DomainError> {
        match (self.project_id, self.lab_role, self.project_role) {
            (None, Some(_), None) | (Some(_), None, Some(_)) => Ok(()),
            _ => Err(DomainError::InvalidMembershipScope),
        }
    }

    pub fn change_lab_role(
        &mut self,
        role: LabRole,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if self.project_id.is_some() || self.lab_role.is_none() || self.project_role.is_some() {
            return Err(DomainError::InvalidMembershipScope);
        }
        if self.lab_role != Some(role) {
            self.lab_role = Some(role);
            self.meta.touch(now);
        }
        Ok(())
    }

    pub fn change_project_role(
        &mut self,
        role: ProjectRole,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if self.project_id.is_none() || self.lab_role.is_some() || self.project_role.is_none() {
            return Err(DomainError::InvalidMembershipScope);
        }
        if self.project_role != Some(role) {
            self.project_role = Some(role);
            self.meta.touch(now);
        }
        Ok(())
    }

    pub fn soft_delete(&mut self, now: DateTime<Utc>) {
        if self.meta.deleted_at.is_none() {
            self.meta.soft_delete(now);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ManageLab,
    ManageUsers,
    ManageProject,
    ReadAnimal,
    WriteAnimal,
    ManageCage,
    ManageBreeding,
    ReadExperiment,
    WriteExperiment,
    DraftTemplate,
    PublishTemplate,
    ReadMeasurement,
    WriteMeasurementDraft,
    SignMeasurement,
    ReadSample,
    WriteSample,
    ReadAttachment,
    WriteAttachment,
    ImportData,
    ExportData,
    ReadAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiScope {
    Read,
    WriteDraft,
    Import,
    Export,
    TemplateDraft,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorAccess {
    pub lab_roles: BTreeSet<LabRole>,
    pub project_roles: BTreeSet<ProjectRole>,
    /// `None` denotes a human session. `Some` denotes an AI/external token and
    /// restricts the human user's effective permissions to these scopes.
    pub ai_scopes: Option<BTreeSet<AiScope>>,
}

impl ActorAccess {
    pub fn human(
        lab_roles: impl IntoIterator<Item = LabRole>,
        project_roles: impl IntoIterator<Item = ProjectRole>,
    ) -> Self {
        Self {
            lab_roles: lab_roles.into_iter().collect(),
            project_roles: project_roles.into_iter().collect(),
            ai_scopes: None,
        }
    }

    pub fn with_ai_scopes(mut self, scopes: impl IntoIterator<Item = AiScope>) -> Self {
        self.ai_scopes = Some(scopes.into_iter().collect());
        self
    }

    pub fn allows(&self, permission: Permission) -> bool {
        self.human_allows(permission) && self.scope_allows(permission)
    }

    fn human_allows(&self, permission: Permission) -> bool {
        if self.lab_roles.contains(&LabRole::LabAdmin) {
            return true;
        }

        if self.lab_roles.contains(&LabRole::AnimalManager)
            && matches!(
                permission,
                Permission::ReadAnimal
                    | Permission::WriteAnimal
                    | Permission::ManageCage
                    | Permission::ManageBreeding
                    | Permission::ReadExperiment
                    | Permission::ReadMeasurement
                    | Permission::ReadSample
                    | Permission::ReadAttachment
                    | Permission::ImportData
                    | Permission::ExportData
            )
        {
            return true;
        }

        if self.project_roles.contains(&ProjectRole::ProjectAdmin)
            && !matches!(
                permission,
                Permission::ManageLab | Permission::ManageUsers | Permission::ManageCage
            )
        {
            return true;
        }

        if self.project_roles.contains(&ProjectRole::Editor)
            && matches!(
                permission,
                Permission::ReadAnimal
                    | Permission::ReadExperiment
                    | Permission::WriteExperiment
                    | Permission::DraftTemplate
                    | Permission::ReadMeasurement
                    | Permission::WriteMeasurementDraft
                    | Permission::SignMeasurement
                    | Permission::ReadSample
                    | Permission::WriteSample
                    | Permission::ReadAttachment
                    | Permission::WriteAttachment
                    | Permission::ImportData
                    | Permission::ExportData
            )
        {
            return true;
        }

        self.project_roles.contains(&ProjectRole::Viewer)
            && matches!(
                permission,
                Permission::ReadAnimal
                    | Permission::ReadExperiment
                    | Permission::ReadMeasurement
                    | Permission::ReadSample
                    | Permission::ReadAttachment
                    | Permission::ExportData
            )
    }

    fn scope_allows(&self, permission: Permission) -> bool {
        let Some(scopes) = &self.ai_scopes else {
            return true;
        };

        match permission {
            Permission::ReadAnimal
            | Permission::ReadExperiment
            | Permission::ReadMeasurement
            | Permission::ReadSample
            | Permission::ReadAttachment => scopes.contains(&AiScope::Read),
            Permission::WriteMeasurementDraft | Permission::WriteSample => {
                scopes.contains(&AiScope::WriteDraft)
            }
            Permission::DraftTemplate => scopes.contains(&AiScope::TemplateDraft),
            Permission::ImportData => scopes.contains(&AiScope::Import),
            Permission::ExportData => scopes.contains(&AiScope::Export),
            _ => false,
        }
    }
}
