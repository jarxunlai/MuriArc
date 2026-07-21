use chrono::{DateTime, NaiveDate, Utc};
use muriarc_core::{
    Animal, AnimalDraft, AuditContext, BreedingLine, BreedingMemberRole, BreedingPair,
    BreedingPairMember, Colony, GenotypeDefinition, Litter, LocusPrediction, MatingEvent,
    MuriArcStore, Sex, predict_mendelian,
};
use uuid::Uuid;

use crate::animal::prepare_animal;
use crate::validation::{normalized_optional, normalized_optional_bytes, normalized_required};
use crate::{
    ApplicationError, ApplicationResult, CreateAnimalCommand, CreateAnimalIdentifierScope,
    MAX_ANIMAL_DISPLAY_ID_CHARS, MAX_ANIMAL_STRAIN_CHARS,
};

pub const MAX_BREEDING_NAME_CHARS: usize = 200;
pub const MAX_BREEDING_DESCRIPTION_BYTES: usize = 8_000;
pub const MAX_BREEDING_NOTES_BYTES: usize = 8_000;
pub const MAX_DRAFT_LABEL_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBreedingLineCommand {
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub genotype_definition_ids: Vec<Uuid>,
    pub now: DateTime<Utc>,
}

pub async fn create_breeding_line(
    store: &dyn MuriArcStore,
    command: CreateBreedingLineCommand,
    audit: &AuditContext,
) -> ApplicationResult<BreedingLine> {
    let name = normalized_required("breeding_line.name", command.name, MAX_BREEDING_NAME_CHARS)?;
    for definition_id in &command.genotype_definition_ids {
        let definition = store.get_genotype_definition(*definition_id).await?;
        if definition.lab_id != command.lab_id {
            return Err(ApplicationError::Validation(
                "breeding line genotype definition belongs to a different lab".to_owned(),
            ));
        }
    }
    let mut line = BreedingLine::new(command.lab_id, name, command.now)?;
    line.description = normalized_optional_bytes(
        "breeding_line.description",
        command.description,
        MAX_BREEDING_DESCRIPTION_BYTES,
    )?;
    line.replace_genotype_definitions(command.genotype_definition_ids)?;
    store.create_breeding_line(&line, audit).await?;
    Ok(line)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateColonyCommand {
    pub lab_id: Uuid,
    pub breeding_line_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_colony(
    store: &dyn MuriArcStore,
    command: CreateColonyCommand,
    audit: &AuditContext,
) -> ApplicationResult<Colony> {
    let name = normalized_required("colony.name", command.name, MAX_BREEDING_NAME_CHARS)?;
    let line = store.get_breeding_line(command.breeding_line_id).await?;
    if line.lab_id != command.lab_id {
        return Err(ApplicationError::Validation(
            "colony breeding line belongs to a different lab".to_owned(),
        ));
    }
    let mut colony = Colony::new(command.lab_id, command.breeding_line_id, name, command.now)?;
    colony.description = normalized_optional_bytes(
        "colony.description",
        command.description,
        MAX_BREEDING_DESCRIPTION_BYTES,
    )?;
    store.create_colony(&colony, audit).await?;
    Ok(colony)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBreedingPairCommand {
    pub lab_id: Uuid,
    pub colony_id: Uuid,
    pub name: String,
    pub male_animal_id: Uuid,
    pub female_animal_ids: Vec<Uuid>,
    pub started_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

pub async fn create_breeding_pair(
    store: &dyn MuriArcStore,
    command: CreateBreedingPairCommand,
    audit: &AuditContext,
) -> ApplicationResult<BreedingPair> {
    let name = normalized_required("breeding_pair.name", command.name, MAX_BREEDING_NAME_CHARS)?;
    let mut pair = BreedingPair::new(
        command.lab_id,
        command.colony_id,
        name,
        command.started_at,
        command.now,
    )?;
    let mut members = Vec::with_capacity(command.female_animal_ids.len() + 1);
    members.push(BreedingPairMember::new(
        pair.id,
        command.male_animal_id,
        BreedingMemberRole::Male,
        command.started_at,
        command.now,
    )?);
    for animal_id in command.female_animal_ids {
        members.push(BreedingPairMember::new(
            pair.id,
            animal_id,
            BreedingMemberRole::Female,
            command.started_at,
            command.now,
        )?);
    }
    pair.replace_members(members)?;
    store.create_breeding_pair(&pair, audit).await?;
    Ok(pair)
}

pub async fn retire_breeding_pair(
    store: &dyn MuriArcStore,
    id: Uuid,
    expected_revision: i64,
    ended_at: DateTime<Utc>,
    audit: &AuditContext,
) -> ApplicationResult<BreedingPair> {
    Ok(store
        .retire_breeding_pair(id, expected_revision, ended_at, audit)
        .await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMatingEventCommand {
    pub lab_id: Uuid,
    pub breeding_pair_id: Uuid,
    pub male_animal_id: Uuid,
    pub female_animal_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_mating_event(
    store: &dyn MuriArcStore,
    command: CreateMatingEventCommand,
    audit: &AuditContext,
) -> ApplicationResult<MatingEvent> {
    let mut event = MatingEvent::new(
        command.lab_id,
        command.breeding_pair_id,
        command.male_animal_id,
        command.female_animal_id,
        command.occurred_at,
        command.now,
    )?;
    event.notes = normalized_optional_bytes(
        "mating_event.notes",
        command.notes,
        MAX_BREEDING_NOTES_BYTES,
    )?;
    store.create_mating_event(&event, audit).await?;
    Ok(event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAnimalDraftInput {
    pub temporary_label: String,
    pub sex: Sex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateLitterCommand {
    pub lab_id: Uuid,
    pub mating_event_id: Uuid,
    pub born_on: NaiveDate,
    pub size_total: i32,
    pub drafts: Vec<CreateAnimalDraftInput>,
    pub notes: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedLitter {
    pub litter: Litter,
    pub drafts: Vec<AnimalDraft>,
}

pub async fn create_litter(
    store: &dyn MuriArcStore,
    command: CreateLitterCommand,
    audit: &AuditContext,
) -> ApplicationResult<CreatedLitter> {
    let size_alive = i32::try_from(command.drafts.len()).map_err(|_| {
        ApplicationError::Validation("litter contains too many live offspring".to_owned())
    })?;
    let mut litter = Litter::new(
        command.lab_id,
        command.mating_event_id,
        command.born_on,
        command.size_total,
        size_alive,
        command.now,
    )?;
    litter.notes =
        normalized_optional_bytes("litter.notes", command.notes, MAX_BREEDING_NOTES_BYTES)?;
    let mut drafts = Vec::with_capacity(command.drafts.len());
    for input in command.drafts {
        let label = normalized_required(
            "animal_draft.temporary_label",
            input.temporary_label,
            MAX_DRAFT_LABEL_CHARS,
        )?;
        drafts.push(AnimalDraft::new(
            command.lab_id,
            litter.id,
            label,
            input.sex,
            command.born_on,
            command.now,
        )?);
    }
    store.create_litter(&litter, &drafts, audit).await?;
    Ok(CreatedLitter { litter, drafts })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAnimalDraftCommand {
    pub lab_id: Uuid,
    pub draft_id: Uuid,
    pub expected_revision: i64,
    pub identifier_scope: CreateAnimalIdentifierScope,
    pub display_id: String,
    pub strain: Option<String>,
    pub initial_cage_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAnimalDraft {
    pub draft: AnimalDraft,
    pub animal: Animal,
}

pub async fn register_animal_draft(
    store: &dyn MuriArcStore,
    command: RegisterAnimalDraftCommand,
    audit: &AuditContext,
) -> ApplicationResult<RegisteredAnimalDraft> {
    let draft = store.get_animal_draft(command.draft_id).await?;
    if draft.lab_id != command.lab_id {
        return Err(ApplicationError::Validation(
            "animal draft belongs to a different lab".to_owned(),
        ));
    }
    let animal = prepare_animal(CreateAnimalCommand {
        lab_id: command.lab_id,
        identifier_scope: command.identifier_scope,
        display_id: normalized_required(
            "animal.display_id",
            command.display_id,
            MAX_ANIMAL_DISPLAY_ID_CHARS,
        )?,
        sex: draft.sex,
        strain: normalized_optional("animal.strain", command.strain, MAX_ANIMAL_STRAIN_CHARS)?,
        birth_date: Some(draft.birth_date),
        legacy_id: None,
        initial_cage_id: command.initial_cage_id,
        now: command.now,
    })?;
    let registered = store
        .register_animal_draft(command.draft_id, command.expected_revision, &animal, audit)
        .await?;
    Ok(RegisteredAnimalDraft {
        draft: registered,
        animal,
    })
}

pub async fn breeding_prediction(
    store: &dyn MuriArcStore,
    male_definition_id: Uuid,
    female_definition_id: Uuid,
) -> ApplicationResult<Vec<LocusPrediction>> {
    let male: GenotypeDefinition = store.get_genotype_definition(male_definition_id).await?;
    let female: GenotypeDefinition = store.get_genotype_definition(female_definition_id).await?;
    Ok(predict_mendelian(&male, &female)?)
}
