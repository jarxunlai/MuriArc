use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    Animal, AnimalEvent, AnimalEventKind, Approval, ApprovalDecision, GenotypingRecord, Job,
    JobKind, Measurement, Pedigree, RecordStatus, ToolRun, ToolRunStatus, WriteSource,
};

pub const MAX_IMPORT_ENTITIES: usize = 50_000;
pub const AI_SOURCE_IMPORT_JOB_BINDING_KEY: &str = "_muriarc_ai_source_binding";
pub const AI_SOURCE_IMPORT_IDEMPOTENCY_PREFIX: &str = "ai-source-import:";

/// Returns true only for the private import Jobs created from an AI
/// conversation source. The idempotency prefix is a defense-in-depth fallback
/// for historical or damaged Job snapshots that no longer contain the binding.
pub fn is_ai_source_import_job(job: &Job) -> bool {
    job.kind == JobKind::Import
        && (job
            .result
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|value| value.contains_key(AI_SOURCE_IMPORT_JOB_BINDING_KEY))
            || job
                .idempotency_key
                .starts_with(AI_SOURCE_IMPORT_IDEMPOTENCY_PREFIX))
}

/// Reads the preview digest from current camelCase Job DTO snapshots while
/// retaining compatibility with older snake_case rows.
pub fn import_job_preview_hash(result: Option<&Value>) -> Option<&str> {
    result
        .and_then(Value::as_object)
        .and_then(|value| {
            value
                .get("previewHash")
                .or_else(|| value.get("preview_hash"))
        })
        .and_then(Value::as_str)
}

/// Fully resolved, reviewable input for one atomic import confirmation.
///
/// All display identifiers, cages, parents, loci and alleles must already be
/// resolved to UUIDs before this plan reaches a Store adapter. Adapters still
/// revalidate every relationship inside the confirmation transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPlan {
    pub commit_id: Uuid,
    pub lab_id: Uuid,
    pub idempotency_key: String,
    /// Lower- or upper-case hexadecimal SHA-256 of the confirmed preview.
    pub preview_hash: String,
    pub animals: Vec<Animal>,
    pub animal_events: Vec<AnimalEvent>,
    pub genotyping_records: Vec<GenotypingRecord>,
    pub pedigrees: Vec<Pedigree>,
    pub measurements: Vec<Measurement>,
}

impl ImportPlan {
    pub fn empty(
        lab_id: Uuid,
        idempotency_key: impl Into<String>,
        preview_hash: impl Into<String>,
    ) -> Self {
        Self {
            commit_id: Uuid::new_v4(),
            lab_id,
            idempotency_key: idempotency_key.into(),
            preview_hash: preview_hash.into(),
            animals: Vec::new(),
            animal_events: Vec::new(),
            genotyping_records: Vec::new(),
            pedigrees: Vec::new(),
            measurements: Vec::new(),
        }
    }

    pub fn entity_counts(&self) -> ImportEntityCounts {
        ImportEntityCounts {
            animals: self.animals.len(),
            animal_events: self.animal_events.len(),
            // Keep the receipt field name stable for existing API clients and
            // import_commits rows; it now counts Genetics v2 records.
            genotypes: self.genotyping_records.len(),
            pedigrees: self.pedigrees.len(),
            measurements: self.measurements.len(),
        }
    }

    /// Returns the formal scope accepted by an ordinary source-derived import.
    ///
    /// Animal Registry imports are lab-wide (`None`). Measurement imports are
    /// bound to exactly one project. A source archive cannot accompany a mixed
    /// animal/measurement plan.
    pub fn source_archive_project_id(&self) -> Result<Option<Uuid>, ImportPlanError> {
        if self.measurements.is_empty() {
            return Ok(None);
        }
        if !self.animals.is_empty()
            || !self.animal_events.is_empty()
            || !self.genotyping_records.is_empty()
            || !self.pedigrees.is_empty()
        {
            return Err(ImportPlanError::InvalidSourceArchive);
        }
        let project_ids = self
            .measurements
            .iter()
            .map(|measurement| measurement.project_id)
            .collect::<BTreeSet<_>>();
        if project_ids.len() != 1 {
            return Err(ImportPlanError::InvalidSourceArchive);
        }
        Ok(project_ids.into_iter().next())
    }

    pub fn validate(&self) -> Result<(), ImportPlanError> {
        if self.commit_id.is_nil() || self.lab_id.is_nil() {
            return Err(ImportPlanError::NilIdentifier);
        }
        if self.idempotency_key.trim().is_empty()
            || self.idempotency_key.chars().count() > 128
            || self.idempotency_key.chars().any(char::is_control)
        {
            return Err(ImportPlanError::InvalidIdempotencyKey);
        }
        if self.preview_hash.len() != 64
            || !self
                .preview_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ImportPlanError::InvalidPreviewHash);
        }
        let counts = self.entity_counts();
        let total = counts.total();
        if total == 0 {
            return Err(ImportPlanError::EmptyPlan);
        }
        if total > MAX_IMPORT_ENTITIES {
            return Err(ImportPlanError::TooManyEntities {
                maximum: MAX_IMPORT_ENTITIES,
            });
        }

        unique_ids("animal", self.animals.iter().map(|animal| animal.id))?;
        unique_ids(
            "animal_event",
            self.animal_events.iter().map(|event| event.id),
        )?;
        unique_ids(
            "genotyping_record",
            self.genotyping_records.iter().map(|record| record.id),
        )?;
        unique_ids(
            "pedigree",
            self.pedigrees.iter().map(|pedigree| pedigree.id),
        )?;
        unique_ids(
            "measurement",
            self.measurements.iter().map(|measurement| measurement.id),
        )?;

        let imported_animals = self
            .animals
            .iter()
            .map(|animal| animal.id)
            .collect::<BTreeSet<_>>();
        if self
            .animals
            .iter()
            .any(|animal| animal.lab_id != self.lab_id)
            || self.animal_events.iter().any(|event| {
                event.lab_id != self.lab_id || !imported_animals.contains(&event.animal_id)
            })
        {
            return Err(ImportPlanError::CrossLabOrUnresolvedAnimal);
        }
        if self.animal_events.iter().any(|event| {
            !matches!(
                &event.kind,
                AnimalEventKind::Registered
                    | AnimalEventKind::Born { .. }
                    | AnimalEventKind::Transferred { .. }
            )
        }) {
            return Err(ImportPlanError::UnsupportedAnimalEvent);
        }
        for animal in &self.animals {
            let events = self
                .animal_events
                .iter()
                .filter(|event| event.animal_id == animal.id)
                .collect::<Vec<_>>();
            let registered = events
                .iter()
                .filter(|event| matches!(&event.kind, AnimalEventKind::Registered))
                .count();
            if registered != 1 {
                return Err(ImportPlanError::InvalidRegisteredEventCount {
                    animal_id: animal.id,
                });
            }

            let born = events
                .iter()
                .filter_map(|event| match &event.kind {
                    AnimalEventKind::Born { birth_date } => Some(*birth_date),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if born.len() > 1
                || match (animal.birth_date, born.first().copied()) {
                    (Some(expected), Some(actual)) => expected != actual,
                    (Some(_), None) | (None, Some(_)) => true,
                    (None, None) => false,
                }
            {
                return Err(ImportPlanError::InvalidBornEvent {
                    animal_id: animal.id,
                });
            }

            let transferred = events
                .iter()
                .filter_map(|event| match &event.kind {
                    AnimalEventKind::Transferred { to_cage_id, .. } => Some(*to_cage_id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if transferred.len() > 1
                || transferred
                    .first()
                    .is_some_and(|to_cage_id| *to_cage_id != animal.current_cage_id)
                || (animal.current_cage_id.is_some() && transferred.is_empty())
            {
                return Err(ImportPlanError::InvalidTransferredEvent {
                    animal_id: animal.id,
                });
            }
        }
        if self.genotyping_records.iter().any(|record| {
            record.lab_id != self.lab_id
                || !imported_animals.contains(&record.animal_id)
                || record.supersedes_record_id.is_some()
                || record.is_voided()
                || record.meta.deleted_at.is_some()
                || record.validate().is_err()
        }) || self
            .pedigrees
            .iter()
            .any(|pedigree| !imported_animals.contains(&pedigree.animal_id))
        {
            return Err(ImportPlanError::CrossLabOrUnresolvedAnimal);
        }

        let mut measurement_keys = BTreeSet::new();
        for measurement in &self.measurements {
            if measurement.lab_id != self.lab_id
                || measurement.status != RecordStatus::Draft
                || measurement.signed_by.is_some()
                || measurement.signed_at.is_some()
                || measurement.label.trim().is_empty()
                || measurement.validate_record().is_err()
            {
                return Err(ImportPlanError::InvalidMeasurement);
            }
            if !measurement_keys.insert((
                measurement.animal_id,
                measurement.key.clone(),
                measurement.measured_at,
            )) {
                return Err(ImportPlanError::DuplicateMeasurement);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSourceArchive {
    pub source_id: Uuid,
    pub expected_revision: i64,
    pub attachment_id: Uuid,
    pub expected_attachment_revision: i64,
    pub conversation_id: Uuid,
    pub project_id: Option<Uuid>,
}

impl ImportSourceArchive {
    pub fn validate(self) -> Result<(), ImportPlanError> {
        if self.source_id.is_nil()
            || self.expected_revision < 1
            || self.attachment_id.is_nil()
            || self.expected_attachment_revision < 1
            || self.conversation_id.is_nil()
        {
            Err(ImportPlanError::InvalidSourceArchive)
        } else {
            Ok(())
        }
    }
}

/// Final AI approval/tool projection that must commit in the same database
/// transaction as the confirmed import, owning Job, and source archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiImportResolution {
    /// Revision of the owning AwaitingConfirmation Job that the human
    /// reviewed. The adapter advances it exactly once in the import
    /// transaction.
    pub expected_job_revision: i64,
    pub tool_run: ToolRun,
    pub expected_tool_run_revision: i64,
    pub approval: Approval,
    pub expected_approval_revision: i64,
}

impl AiImportResolution {
    fn validate(&self) -> Result<(), ImportPlanError> {
        let approval_draft = self.approval.requested_diff.get("draft");
        let tool_draft = self
            .tool_run
            .output
            .as_ref()
            .and_then(|value| value.get("draft"));
        let applied_draft_matches = approval_draft == tool_draft
            && approval_draft.is_some_and(|draft| {
                draft.get("status").and_then(serde_json::Value::as_str) == Some("applied")
                    && draft
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| Uuid::parse_str(value).ok())
                        == Some(self.approval.id)
            });
        if self.expected_job_revision < 1
            || self.tool_run.id.is_nil()
            || self.approval.id.is_nil()
            || self.approval.tool_run_id != self.tool_run.id
            || self.tool_run.conversation_id.is_none()
            || self.tool_run.tool_name != "import_commit_draft"
            || self.expected_tool_run_revision < 1
            || self.expected_approval_revision < 1
            || self.tool_run.meta.revision != self.expected_tool_run_revision + 1
            || self.approval.meta.revision != self.expected_approval_revision + 1
            || self.tool_run.status != ToolRunStatus::Completed
            || self.tool_run.source != WriteSource::Ai
            || self.tool_run.completed_at.is_none()
            || self.tool_run.error.is_some()
            || self.approval.decision != ApprovalDecision::Approved
            || self.approval.decided_by.is_none()
            || self.approval.decided_at.is_none()
            || !applied_draft_matches
        {
            Err(ImportPlanError::InvalidAiResolution)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportCommitOptions {
    /// Snapshot of the owning Job cancellation flag immediately before the
    /// adapter starts its transaction.
    pub cancellation_requested: bool,
    /// Owning import Job, recorded in entity provenance during confirmation.
    pub job_id: Option<Uuid>,
    /// Optional trusted AI conversation source to archive atomically with the
    /// import. This is constructed by an application backend from persisted
    /// source binding metadata, never from model approval arguments.
    pub source_archive: Option<ImportSourceArchive>,
    /// Present only for an AI-confirmed import. Adapters must update this
    /// resolution and the owning Job in the same transaction as domain rows.
    pub ai_resolution: Option<AiImportResolution>,
}

impl ImportCommitOptions {
    pub fn validate_for_plan(&self, plan: &ImportPlan) -> Result<(), ImportPlanError> {
        if self.job_id.is_some_and(|value| value.is_nil()) {
            return Err(ImportPlanError::InvalidSourceArchive);
        }
        if let Some(source_archive) = self.source_archive {
            source_archive.validate()?;
            if self.job_id.is_none()
                || source_archive.project_id != plan.source_archive_project_id()?
            {
                return Err(ImportPlanError::InvalidSourceArchive);
            }
        }
        if let Some(resolution) = &self.ai_resolution {
            resolution.validate()?;
            let draft = resolution
                .approval
                .requested_diff
                .get("draft")
                .and_then(Value::as_object);
            let payload = draft
                .and_then(|draft| draft.get("payload"))
                .and_then(Value::as_object);
            let reviewed_job_id = payload
                .and_then(|payload| payload.get("job_id"))
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok());
            let reviewed_revision = payload
                .and_then(|payload| payload.get("expected_revision"))
                .and_then(Value::as_i64);
            let reviewed_hash = payload
                .and_then(|payload| payload.get("preview_hash"))
                .and_then(Value::as_str);
            if self.job_id.is_none()
                || resolution.tool_run.lab_id != plan.lab_id
                || resolution.tool_run.project_id != plan.source_archive_project_id()?
                || draft
                    .and_then(|draft| draft.get("kind"))
                    .and_then(Value::as_str)
                    != Some("bulk_import")
                || draft
                    .and_then(|draft| draft.get("tool"))
                    .and_then(Value::as_str)
                    != Some("import_commit_draft")
                || reviewed_job_id != self.job_id
                || reviewed_revision != Some(resolution.expected_job_revision)
                || !reviewed_hash
                    .is_some_and(|value| value.eq_ignore_ascii_case(&plan.preview_hash))
            {
                return Err(ImportPlanError::InvalidAiResolution);
            }
            if let Some(source_archive) = self.source_archive
                && resolution.tool_run.conversation_id != Some(source_archive.conversation_id)
            {
                return Err(ImportPlanError::InvalidAiResolution);
            }
        }
        if plan
            .idempotency_key
            .starts_with(AI_SOURCE_IMPORT_IDEMPOTENCY_PREFIX)
            && (self.source_archive.is_none() || self.ai_resolution.is_none())
        {
            return Err(ImportPlanError::InvalidAiResolution);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportEntityCounts {
    pub animals: usize,
    pub animal_events: usize,
    pub genotypes: usize,
    pub pedigrees: usize,
    pub measurements: usize,
}

impl ImportEntityCounts {
    pub const fn total(self) -> usize {
        self.animals + self.animal_events + self.genotypes + self.pedigrees + self.measurements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCommitResult {
    pub commit_id: Uuid,
    pub preview_hash: String,
    pub counts: ImportEntityCounts,
    pub committed_at: DateTime<Utc>,
    /// True only when the same idempotency key and preview hash were already
    /// committed and no entity write was repeated.
    pub replayed: bool,
}

/// Returns the canonical receipt embedded in durable Job and ToolRun records.
///
/// A replay response may set `replayed=true`, but the original durable
/// operation is always compared against the first successful receipt.
pub fn canonical_import_receipt(receipt: &ImportCommitResult) -> ImportCommitResult {
    let mut receipt = receipt.clone();
    receipt.replayed = false;
    receipt
}

/// Builds the exact completed ToolRun projection used by both Store adapters
/// and by application-level completed-import replay validation.
pub fn completed_ai_import_tool_run(
    resolution: &AiImportResolution,
    job_id: Uuid,
    receipt: &ImportCommitResult,
) -> Result<ToolRun, ImportPlanError> {
    let mut tool_run = resolution.tool_run.clone();
    let output = tool_run
        .output
        .as_mut()
        .and_then(Value::as_object_mut)
        .ok_or(ImportPlanError::InvalidAiResolution)?;
    output.insert("job_id".to_owned(), Value::String(job_id.to_string()));
    output.insert(
        "result".to_owned(),
        serde_json::to_value(canonical_import_receipt(receipt))
            .map_err(|_| ImportPlanError::InvalidAiResolution)?,
    );
    Ok(tool_run)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImportPlanError {
    #[error("import identifiers must not be nil UUIDs")]
    NilIdentifier,
    #[error("import idempotency key must contain 1-128 non-control characters")]
    InvalidIdempotencyKey,
    #[error("preview hash must be a 64-character hexadecimal SHA-256")]
    InvalidPreviewHash,
    #[error("import plan must contain at least one entity")]
    EmptyPlan,
    #[error("import plan cannot contain more than {maximum} entities")]
    TooManyEntities { maximum: usize },
    #[error("import plan contains a duplicate {entity} UUID")]
    DuplicateEntityId { entity: &'static str },
    #[error("import plan contains a cross-lab or unresolved animal relationship")]
    CrossLabOrUnresolvedAnimal,
    #[error("import plan contains an animal event outside the supported import vocabulary")]
    UnsupportedAnimalEvent,
    #[error("imported animal {animal_id} must have exactly one Registered event")]
    InvalidRegisteredEventCount { animal_id: Uuid },
    #[error("imported animal {animal_id} has inconsistent Born events")]
    InvalidBornEvent { animal_id: Uuid },
    #[error("imported animal {animal_id} has inconsistent Transferred events")]
    InvalidTransferredEvent { animal_id: Uuid },
    #[error("import plan contains an invalid or signed measurement")]
    InvalidMeasurement,
    #[error("import plan contains a duplicate animal/key/time measurement")]
    DuplicateMeasurement,
    #[error("import source archive binding is invalid")]
    InvalidSourceArchive,
    #[error("AI import approval resolution is invalid")]
    InvalidAiResolution,
}

fn unique_ids(
    entity: &'static str,
    ids: impl IntoIterator<Item = Uuid>,
) -> Result<(), ImportPlanError> {
    let mut unique = BTreeSet::new();
    if ids.into_iter().all(|id| !id.is_nil() && unique.insert(id)) {
        Ok(())
    } else {
        Err(ImportPlanError::DuplicateEntityId { entity })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::{AnimalEvent, MeasurementValue, Sex};

    fn simple_animal_plan() -> (ImportPlan, Animal, DateTime<Utc>) {
        let now = Utc::now();
        let lab_id = Uuid::new_v4();
        let animal = Animal::new_mouse(lab_id, "M1", Sex::Female, now).unwrap();
        let event = AnimalEvent::new(lab_id, animal.id, AnimalEventKind::Registered, now, now);
        let mut plan = ImportPlan::empty(lab_id, "import-1", "a".repeat(64));
        plan.animals.push(animal.clone());
        plan.animal_events.push(event);
        (plan, animal, now)
    }

    #[test]
    fn simple_animal_plan_is_valid() {
        let (plan, _, _) = simple_animal_plan();
        assert_eq!(plan.validate(), Ok(()));
    }

    #[test]
    fn import_job_preview_hash_accepts_current_and_legacy_snapshot_keys() {
        let current = serde_json::json!({"previewHash": "a".repeat(64)});
        let legacy = serde_json::json!({"preview_hash": "b".repeat(64)});
        let current_hash = "a".repeat(64);
        let legacy_hash = "b".repeat(64);
        assert_eq!(
            import_job_preview_hash(Some(&current)),
            Some(current_hash.as_str())
        );
        assert_eq!(
            import_job_preview_hash(Some(&legacy)),
            Some(legacy_hash.as_str())
        );
        assert_eq!(import_job_preview_hash(Some(&serde_json::json!({}))), None);
    }

    #[test]
    fn plan_rejects_duplicate_measurements_before_store_access() {
        let (mut plan, animal, now) = simple_animal_plan();
        let measurement = Measurement::draft(
            plan.lab_id,
            Uuid::new_v4(),
            animal.id,
            "body_weight",
            "Body weight",
            MeasurementValue::Number(20.0),
            now,
            now,
        )
        .unwrap();
        let mut duplicate = measurement.clone();
        duplicate.id = Uuid::new_v4();
        plan.measurements.extend([measurement, duplicate]);

        assert_eq!(plan.validate(), Err(ImportPlanError::DuplicateMeasurement));
    }

    #[test]
    fn born_event_must_be_unique_and_match_the_animal_projection() {
        let (mut plan, _, now) = simple_animal_plan();
        let birth_date = now.date_naive();
        plan.animals[0].birth_date = Some(birth_date);

        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidBornEvent { .. })
        ));

        plan.animal_events.push(AnimalEvent::new(
            plan.lab_id,
            plan.animals[0].id,
            AnimalEventKind::Born {
                birth_date: birth_date - Duration::days(1),
            },
            now,
            now,
        ));
        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidBornEvent { .. })
        ));

        plan.animal_events[1].kind = AnimalEventKind::Born { birth_date };
        assert_eq!(plan.validate(), Ok(()));

        plan.animal_events.push(AnimalEvent::new(
            plan.lab_id,
            plan.animals[0].id,
            AnimalEventKind::Born { birth_date },
            now,
            now,
        ));
        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidBornEvent { .. })
        ));
    }

    #[test]
    fn transfer_event_must_be_unique_and_match_the_current_cage_projection() {
        let (mut plan, _, now) = simple_animal_plan();
        let cage_id = Uuid::new_v4();
        plan.animals[0].current_cage_id = Some(cage_id);

        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidTransferredEvent { .. })
        ));

        plan.animal_events.push(AnimalEvent::new(
            plan.lab_id,
            plan.animals[0].id,
            AnimalEventKind::Transferred {
                from_cage_id: None,
                to_cage_id: Some(Uuid::new_v4()),
            },
            now,
            now,
        ));
        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidTransferredEvent { .. })
        ));

        plan.animal_events[1].kind = AnimalEventKind::Transferred {
            from_cage_id: None,
            to_cage_id: Some(cage_id),
        };
        assert_eq!(plan.validate(), Ok(()));

        plan.animal_events.push(AnimalEvent::new(
            plan.lab_id,
            plan.animals[0].id,
            AnimalEventKind::Transferred {
                from_cage_id: None,
                to_cage_id: Some(cage_id),
            },
            now,
            now,
        ));
        assert!(matches!(
            plan.validate(),
            Err(ImportPlanError::InvalidTransferredEvent { .. })
        ));
    }
}
