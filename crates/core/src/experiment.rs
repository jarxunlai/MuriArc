use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, RecordMeta, require_non_empty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldValueType {
    Number,
    Text,
    Boolean,
    Date,
    Category,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateField {
    pub key: String,
    pub label: String,
    pub value_type: FieldValueType,
    pub unit: Option<String>,
    pub required: bool,
    pub categories: Vec<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub display_order: i32,
    pub ai_writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    Draft,
    Published,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentTemplateVersion {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub template_key: String,
    pub version: i32,
    pub name: String,
    pub description: Option<String>,
    pub status: TemplateStatus,
    pub fields: Vec<TemplateField>,
    pub published_at: Option<DateTime<Utc>>,
    pub published_by: Option<Uuid>,
    pub meta: RecordMeta,
}

impl ExperimentTemplateVersion {
    pub fn draft(
        lab_id: Uuid,
        template_key: impl Into<String>,
        version: i32,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let template_key = template_key.into();
        let name = name.into();
        require_non_empty("template.template_key", &template_key)?;
        require_non_empty("template.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            template_key,
            version,
            name,
            description: None,
            status: TemplateStatus::Draft,
            fields: Vec::new(),
            published_at: None,
            published_by: None,
            meta: RecordMeta::new(now),
        })
    }

    pub fn replace_fields(
        &mut self,
        fields: Vec<TemplateField>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if self.status != TemplateStatus::Draft {
            return Err(DomainError::PublishedTemplateImmutable);
        }
        self.fields = fields;
        self.meta.touch(now);
        Ok(())
    }

    pub fn publish(&mut self, user_id: Uuid, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.status != TemplateStatus::Draft {
            return Err(DomainError::TemplateNotDraft);
        }
        self.status = TemplateStatus::Published;
        self.published_at = Some(now);
        self.published_by = Some(user_id);
        self.meta.touch(now);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Draft,
    Active,
    Completed,
    Cancelled,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Experiment {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub template_version_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub status: ExperimentStatus,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

impl Experiment {
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("experiment.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            template_version_id: None,
            name,
            description: None,
            status: ExperimentStatus::Draft,
            starts_at: None,
            ends_at: None,
            meta: RecordMeta::new(now),
        })
    }

    /// Closes an open experiment. The Store applies this transition together
    /// with enrolled participation exits and animal events in one transaction.
    pub fn close(
        &mut self,
        target: ExperimentStatus,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if !matches!(
            target,
            ExperimentStatus::Completed | ExperimentStatus::Cancelled
        ) {
            return Err(DomainError::InvalidExperimentTransition);
        }
        if !matches!(
            self.status,
            ExperimentStatus::Draft | ExperimentStatus::Active
        ) {
            return Err(DomainError::ExperimentNotOpen);
        }
        self.status = target;
        self.ends_at = Some(now);
        self.meta.touch(now);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cohort {
    pub id: Uuid,
    pub experiment_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub meta: RecordMeta,
}

impl Cohort {
    pub fn new(
        experiment_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("cohort.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            experiment_id,
            name: name.trim().to_owned(),
            description: None,
            meta: RecordMeta::new(now),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipationStatus {
    Enrolled,
    Completed,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypeSnapshotEntry {
    pub genotyping_record_id: Uuid,
    pub genotype_definition_id: Uuid,
    pub state: crate::GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participation {
    pub id: Uuid,
    pub experiment_id: Uuid,
    pub animal_id: Uuid,
    pub cohort_id: Option<Uuid>,
    pub status: ParticipationStatus,
    pub enrolled_at: DateTime<Utc>,
    pub exited_at: Option<DateTime<Utc>>,
    pub genotype_snapshot: Vec<GenotypeSnapshotEntry>,
    pub meta: RecordMeta,
}

impl Participation {
    pub fn enroll(experiment_id: Uuid, animal_id: Uuid, now: DateTime<Utc>) -> Self {
        Self {
            id: Uuid::new_v4(),
            experiment_id,
            animal_id,
            cohort_id: None,
            status: ParticipationStatus::Enrolled,
            enrolled_at: now,
            exited_at: None,
            genotype_snapshot: Vec::new(),
            meta: RecordMeta::new(now),
        }
    }

    pub fn close(
        &mut self,
        target: ParticipationStatus,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if !matches!(
            target,
            ParticipationStatus::Completed | ParticipationStatus::Withdrawn
        ) {
            return Err(DomainError::InvalidParticipationTransition);
        }
        if self.status != ParticipationStatus::Enrolled {
            return Err(DomainError::ParticipationNotEnrolled);
        }
        self.status = target;
        self.exited_at = Some(now);
        self.meta.touch(now);
        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn only_open_experiments_can_be_closed() {
        let now = Utc::now();
        let mut experiment = Experiment::new(Uuid::new_v4(), Uuid::new_v4(), "study", now).unwrap();
        experiment.close(ExperimentStatus::Completed, now).unwrap();
        assert_eq!(experiment.status, ExperimentStatus::Completed);
        assert_eq!(experiment.ends_at, Some(now));
        assert!(matches!(
            experiment.close(ExperimentStatus::Cancelled, now),
            Err(DomainError::ExperimentNotOpen)
        ));
    }

    #[test]
    fn participation_exit_is_single_use() {
        let now = Utc::now();
        let mut participation = Participation::enroll(Uuid::new_v4(), Uuid::new_v4(), now);
        participation
            .close(ParticipationStatus::Withdrawn, now)
            .unwrap();
        assert_eq!(participation.status, ParticipationStatus::Withdrawn);
        assert_eq!(participation.exited_at, Some(now));
        assert!(matches!(
            participation.close(ParticipationStatus::Completed, now),
            Err(DomainError::ParticipationNotEnrolled)
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureStatus {
    Planned,
    Completed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Procedure {
    pub id: Uuid,
    pub experiment_id: Uuid,
    pub animal_id: Option<Uuid>,
    pub name: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub performed_at: Option<DateTime<Utc>>,
    pub status: ProcedureStatus,
    pub details: serde_json::Value,
    pub meta: RecordMeta,
}

impl Procedure {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        experiment_id: Uuid,
        animal_id: Option<Uuid>,
        name: impl Into<String>,
        status: ProcedureStatus,
        scheduled_at: Option<DateTime<Utc>>,
        performed_at: Option<DateTime<Utc>>,
        details: serde_json::Value,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("procedure.name", &name)?;
        if !details.is_object() {
            return Err(DomainError::InvalidProcedureDetails);
        }
        match (status, performed_at) {
            (ProcedureStatus::Completed, Some(_))
            | (
                ProcedureStatus::Planned | ProcedureStatus::Skipped | ProcedureStatus::Cancelled,
                None,
            ) => {}
            _ => return Err(DomainError::InvalidProcedureState),
        }
        Ok(Self {
            id: Uuid::new_v4(),
            experiment_id,
            animal_id,
            name: name.trim().to_owned(),
            scheduled_at,
            performed_at,
            status,
            details,
            meta: RecordMeta::new(now),
        })
    }
}
