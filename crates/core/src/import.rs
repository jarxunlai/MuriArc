use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    Animal, AnimalEvent, AnimalEventKind, GenotypingRecord, Measurement, Pedigree, RecordStatus,
};

pub const MAX_IMPORT_ENTITIES: usize = 50_000;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCommitOptions {
    /// Snapshot of the owning Job cancellation flag immediately before the
    /// adapter starts its transaction.
    pub cancellation_requested: bool,
    /// Owning import Job, recorded in entity provenance during confirmation.
    pub job_id: Option<Uuid>,
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
