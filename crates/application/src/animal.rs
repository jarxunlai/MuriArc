use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use muriarc_core::{
    Animal, AuditContext, GenotypingRecord, GenotypingState, IdentifierScope, MuriArcStore, Sex,
};
use uuid::Uuid;

use crate::genetics::{MAX_GENOTYPING_METHOD_BYTES, MAX_GENOTYPING_NOTES_BYTES};
use crate::validation::{normalized_optional, normalized_optional_bytes, normalized_required};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_ANIMAL_DISPLAY_ID_CHARS: usize = 64;
pub const MAX_ANIMAL_STRAIN_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateAnimalIdentifierScope {
    Lab,
    Project(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialGenotypingRecordInput {
    pub genotype_definition_id: Uuid,
    pub state: GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAnimalCommand {
    pub lab_id: Uuid,
    pub identifier_scope: CreateAnimalIdentifierScope,
    pub display_id: String,
    pub sex: Sex,
    pub strain: Option<String>,
    pub birth_date: Option<NaiveDate>,
    pub legacy_id: Option<String>,
    pub initial_cage_id: Option<Uuid>,
    pub initial_genotyping_records: Vec<InitialGenotypingRecordInput>,
    pub now: DateTime<Utc>,
}

pub async fn create_animal(
    store: &dyn MuriArcStore,
    command: CreateAnimalCommand,
    audit: &AuditContext,
) -> ApplicationResult<Animal> {
    let project_id = match command.identifier_scope {
        CreateAnimalIdentifierScope::Lab => None,
        CreateAnimalIdentifierScope::Project(project_id) => Some(project_id),
    };
    let record_inputs = command.initial_genotyping_records.clone();
    let animal = prepare_animal(command)?;
    let mut definition_ids = BTreeSet::new();
    let mut records = Vec::with_capacity(record_inputs.len());
    for input in record_inputs {
        if !definition_ids.insert(input.genotype_definition_id) {
            return Err(ApplicationError::Validation(
                "initial genotyping records contain a duplicate genotype definition".to_owned(),
            ));
        }
        let mut record = GenotypingRecord::new(
            animal.lab_id,
            animal.id,
            input.genotype_definition_id,
            input.state,
            input.assessed_at,
            animal.meta.created_at,
        )?;
        record.project_id = project_id;
        record.method = normalized_optional_bytes(
            "genotyping_record.method",
            input.method,
            MAX_GENOTYPING_METHOD_BYTES,
        )?;
        record.notes = normalized_optional_bytes(
            "genotyping_record.notes",
            input.notes,
            MAX_GENOTYPING_NOTES_BYTES,
        )?;
        record.validate()?;
        records.push(record);
    }
    store
        .create_animal_with_genotyping_records(&animal, &records, audit)
        .await?;
    Ok(store.get_animal(animal.id).await?)
}

pub(crate) fn prepare_animal(command: CreateAnimalCommand) -> ApplicationResult<Animal> {
    let display_id = normalized_required(
        "animal.display_id",
        command.display_id,
        MAX_ANIMAL_DISPLAY_ID_CHARS,
    )?;
    let strain = normalized_optional("animal.strain", command.strain, MAX_ANIMAL_STRAIN_CHARS)?;
    let legacy_id = command
        .legacy_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let mut animal = Animal::new_mouse(command.lab_id, display_id, command.sex, command.now)?;
    animal.identifier_scope = match command.identifier_scope {
        CreateAnimalIdentifierScope::Lab => IdentifierScope::Lab,
        CreateAnimalIdentifierScope::Project(project_id) => IdentifierScope::Project { project_id },
    };
    animal.strain = strain;
    animal.birth_date = command.birth_date;
    animal.legacy_id = legacy_id;
    animal.current_cage_id = command.initial_cage_id;
    Ok(animal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApplicationError;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-19T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn command(display_id: impl Into<String>) -> CreateAnimalCommand {
        CreateAnimalCommand {
            lab_id: Uuid::new_v4(),
            identifier_scope: CreateAnimalIdentifierScope::Lab,
            display_id: display_id.into(),
            sex: Sex::Female,
            strain: None,
            birth_date: None,
            legacy_id: None,
            initial_cage_id: None,
            initial_genotyping_records: Vec::new(),
            now: fixed_now(),
        }
    }

    #[test]
    fn preparation_rejects_overlong_shared_fields() {
        let error =
            prepare_animal(command("x".repeat(MAX_ANIMAL_DISPLAY_ID_CHARS + 1))).unwrap_err();
        assert!(matches!(
            error,
            ApplicationError::TooLong {
                field: "animal.display_id",
                max: MAX_ANIMAL_DISPLAY_ID_CHARS
            }
        ));

        let mut command = command("M-001");
        command.strain = Some("x".repeat(MAX_ANIMAL_STRAIN_CHARS + 1));
        let error = prepare_animal(command).unwrap_err();
        assert!(matches!(
            error,
            ApplicationError::TooLong {
                field: "animal.strain",
                max: MAX_ANIMAL_STRAIN_CHARS
            }
        ));
    }
}
