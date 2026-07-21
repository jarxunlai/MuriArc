use chrono::{DateTime, Utc};
use muriarc_core::{Animal, AnimalTransfer, AuditContext, Cage, CageKind, MuriArcStore};
use uuid::Uuid;

use crate::ApplicationResult;
use crate::validation::{ensure_max_bytes, normalized_optional, normalized_required};

pub const MAX_CAGE_DISPLAY_ID_CHARS: usize = 64;
pub const MAX_CAGE_SECTION_CHARS: usize = 128;
pub const MAX_CAGE_LOCATION_CHARS: usize = 128;
pub const MAX_TRANSFER_NOTES_BYTES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCageCommand {
    pub lab_id: Uuid,
    pub section: String,
    pub display_id: String,
    pub location: Option<String>,
    pub kind: CageKind,
    pub capacity: i32,
    pub sort_order: i32,
    pub now: DateTime<Utc>,
}

pub async fn create_cage(
    store: &dyn MuriArcStore,
    command: CreateCageCommand,
    audit: &AuditContext,
) -> ApplicationResult<Cage> {
    let section = normalized_required("cage.section", command.section, MAX_CAGE_SECTION_CHARS)?;
    let display_id = normalized_required(
        "cage.display_id",
        command.display_id,
        MAX_CAGE_DISPLAY_ID_CHARS,
    )?;
    let location = normalized_optional("cage.location", command.location, MAX_CAGE_LOCATION_CHARS)?;

    let mut cage = Cage::new(command.lab_id, section, display_id, command.now)?;
    cage.location = location;
    cage.kind = command.kind;
    cage.set_capacity(command.capacity)?;
    cage.sort_order = command.sort_order;
    store.create_cage(&cage, audit).await?;
    Ok(cage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferAnimalsCommand {
    pub lab_id: Uuid,
    pub animal_ids: Vec<Uuid>,
    pub target_cage_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
}

pub async fn transfer_animals(
    store: &dyn MuriArcStore,
    command: TransferAnimalsCommand,
    audit: &AuditContext,
) -> ApplicationResult<Vec<Animal>> {
    let notes = command
        .notes
        .map(|notes| notes.trim().to_owned())
        .filter(|notes| !notes.is_empty());
    if let Some(notes) = &notes {
        ensure_max_bytes("transfer.notes", notes, MAX_TRANSFER_NOTES_BYTES)?;
    }

    let mut transfer = AnimalTransfer::new(
        command.lab_id,
        command.animal_ids,
        command.target_cage_id,
        command.occurred_at,
        command.recorded_at,
    )?;
    transfer.recorded_by = command.recorded_by;
    transfer.notes = notes;
    Ok(store.transfer_animals(&transfer, audit).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApplicationError;

    #[test]
    fn transfer_notes_use_a_bounded_utf8_payload() {
        let notes = "鼠".repeat(MAX_TRANSFER_NOTES_BYTES / 3 + 1);
        let error =
            ensure_max_bytes("transfer.notes", &notes, MAX_TRANSFER_NOTES_BYTES).unwrap_err();
        assert!(matches!(
            error,
            ApplicationError::TooManyBytes {
                field: "transfer.notes",
                max: MAX_TRANSFER_NOTES_BYTES
            }
        ));
    }
}
