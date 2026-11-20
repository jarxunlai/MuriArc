use chrono::{DateTime, Utc};
use muriarc_core::{
    Animal, AnimalTransfer, AuditContext, Cage, CageKind, CageUpdate, MuriArcStore, StoreError,
};
use uuid::Uuid;

use crate::validation::{ensure_max_bytes, normalized_optional, normalized_required};
use crate::{ApplicationError, ApplicationResult};

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

/// Partial edit of a cage's housing attributes at the revision the caller
/// observed. `None` keeps the current value; a blank `location` clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCageCommand {
    pub lab_id: Uuid,
    pub cage_id: Uuid,
    pub expected_revision: i64,
    pub location: Option<String>,
    pub capacity: Option<i32>,
    pub sort_order: Option<i32>,
    pub now: DateTime<Utc>,
}

pub async fn update_cage(
    store: &dyn MuriArcStore,
    command: UpdateCageCommand,
    audit: &AuditContext,
) -> ApplicationResult<Cage> {
    if command.location.is_none() && command.capacity.is_none() && command.sort_order.is_none() {
        return Err(ApplicationError::Validation(
            "cage update must change location, capacity or sort_order".to_owned(),
        ));
    }
    let location = command
        .location
        .map(|location| {
            normalized_optional("cage.location", Some(location), MAX_CAGE_LOCATION_CHARS)
        })
        .transpose()?;

    // Merging with the current row is safe: the Store rejects the write unless
    // the locked row still has `expected_revision`, i.e. the state merged here.
    let current = store.get_cage(command.cage_id).await?;
    if current.lab_id != command.lab_id {
        return Err(StoreError::NotFound {
            entity: "cage",
            id: command.cage_id,
        }
        .into());
    }
    let update = CageUpdate {
        cage_id: current.id,
        lab_id: command.lab_id,
        expected_revision: command.expected_revision,
        location: location.unwrap_or(current.location),
        capacity: command.capacity.unwrap_or(current.capacity),
        sort_order: command.sort_order.unwrap_or(current.sort_order),
        updated_at: command.now,
    };
    update.validate()?;
    Ok(store.update_cage(&update, audit).await?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveCageCommand {
    pub lab_id: Uuid,
    pub cage_id: Uuid,
    pub expected_revision: i64,
    pub now: DateTime<Utc>,
}

/// Retires an empty cage by soft deletion; residents must be transferred out
/// first so no animal is left pointing at an archived cage.
pub async fn archive_cage(
    store: &dyn MuriArcStore,
    command: ArchiveCageCommand,
    audit: &AuditContext,
) -> ApplicationResult<Cage> {
    Ok(store
        .archive_cage(
            command.lab_id,
            command.cage_id,
            command.expected_revision,
            command.now,
            audit,
        )
        .await?)
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
