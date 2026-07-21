use chrono::{DateTime, NaiveDate, Utc};
use muriarc_core::{Animal, AuditContext, IdentifierScope, MuriArcStore, Sex};
use uuid::Uuid;

use crate::ApplicationResult;
use crate::validation::{normalized_optional, normalized_required};

pub const MAX_ANIMAL_DISPLAY_ID_CHARS: usize = 64;
pub const MAX_ANIMAL_STRAIN_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateAnimalIdentifierScope {
    Lab,
    Project(Uuid),
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
    pub now: DateTime<Utc>,
}

pub async fn create_animal(
    store: &dyn MuriArcStore,
    command: CreateAnimalCommand,
    audit: &AuditContext,
) -> ApplicationResult<Animal> {
    let animal = prepare_animal(command)?;
    store.create_animal(&animal, audit).await?;
    Ok(animal)
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
