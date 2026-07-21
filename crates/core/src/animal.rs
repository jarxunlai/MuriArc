use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, ParticipationStatus, RecordMeta, require_non_empty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdentifierScope {
    Lab,
    Project { project_id: Uuid },
    Legacy { source: String },
}

impl IdentifierScope {
    pub fn storage_key(&self) -> String {
        match self {
            Self::Lab => "lab".to_owned(),
            Self::Project { project_id } => format!("project:{project_id}"),
            Self::Legacy { source } => format!("legacy:{source}"),
        }
    }

    pub fn from_storage_key(value: &str) -> Self {
        if value == "lab" {
            return Self::Lab;
        }
        if let Some(id) = value
            .strip_prefix("project:")
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            return Self::Project { project_id: id };
        }
        Self::Legacy {
            source: value.strip_prefix("legacy:").unwrap_or(value).to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimalStatus {
    Planned,
    Alive,
    InExperiment,
    Sampled,
    Deceased,
    Euthanized,
    Lost,
    Archived,
}

impl AnimalStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Deceased | Self::Euthanized | Self::Lost | Self::Archived
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Planned => matches!(next, Self::Alive | Self::Archived),
            Self::Alive => matches!(
                next,
                Self::InExperiment
                    | Self::Sampled
                    | Self::Deceased
                    | Self::Euthanized
                    | Self::Lost
                    | Self::Archived
            ),
            Self::InExperiment => matches!(
                next,
                Self::Alive
                    | Self::Sampled
                    | Self::Deceased
                    | Self::Euthanized
                    | Self::Lost
                    | Self::Archived
            ),
            Self::Sampled => matches!(
                next,
                Self::Alive
                    | Self::InExperiment
                    | Self::Deceased
                    | Self::Euthanized
                    | Self::Archived
            ),
            Self::Deceased | Self::Euthanized | Self::Lost => matches!(next, Self::Archived),
            Self::Archived => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Animal {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub identifier_scope: IdentifierScope,
    pub display_id: String,
    pub legacy_id: Option<String>,
    pub species: String,
    pub strain: Option<String>,
    pub sex: Sex,
    pub birth_date: Option<NaiveDate>,
    pub death_date: Option<NaiveDate>,
    pub current_cage_id: Option<Uuid>,
    pub current_status: AnimalStatus,
    pub meta: RecordMeta,
}

impl Animal {
    pub fn new_mouse(
        lab_id: Uuid,
        display_id: impl Into<String>,
        sex: Sex,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let display_id = display_id.into();
        require_non_empty("animal.display_id", &display_id)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            identifier_scope: IdentifierScope::Lab,
            display_id,
            legacy_id: None,
            species: "Mus musculus".to_owned(),
            strain: None,
            sex,
            birth_date: None,
            death_date: None,
            current_cage_id: None,
            current_status: AnimalStatus::Alive,
            meta: RecordMeta::new(now),
        })
    }

    pub fn apply_event(&mut self, event: &AnimalEvent) -> Result<(), DomainError> {
        if event.animal_id != self.id {
            return Err(DomainError::EventAnimalMismatch {
                expected: self.id,
                actual: event.animal_id,
            });
        }

        match &event.kind {
            AnimalEventKind::Registered => {}
            AnimalEventKind::Born { birth_date } => {
                self.birth_date = Some(*birth_date);
                self.transition_status(AnimalStatus::Alive)?;
            }
            AnimalEventKind::Transferred { to_cage_id, .. } => {
                self.current_cage_id = *to_cage_id;
            }
            AnimalEventKind::StatusChanged { to, .. } => {
                self.transition_status(*to)?;
                if matches!(to, AnimalStatus::Deceased | AnimalStatus::Euthanized) {
                    self.death_date = Some(event.occurred_at.date_naive());
                }
            }
            AnimalEventKind::ExperimentEnrolled { .. } => {
                self.transition_status(AnimalStatus::InExperiment)?;
            }
            AnimalEventKind::SampleCollected { terminal, .. } => {
                if *terminal {
                    self.transition_status(AnimalStatus::Sampled)?;
                }
            }
            AnimalEventKind::Genotyped { .. }
            | AnimalEventKind::GenotypingRecorded { .. }
            | AnimalEventKind::ExperimentParticipationEnded { .. }
            | AnimalEventKind::ProcedurePerformed { .. }
            | AnimalEventKind::MeasurementRecorded { .. }
            | AnimalEventKind::Note { .. } => {}
        }
        self.meta.touch(event.recorded_at);
        Ok(())
    }

    fn transition_status(&mut self, next: AnimalStatus) -> Result<(), DomainError> {
        if !self.current_status.can_transition_to(next) {
            return Err(DomainError::InvalidStatusTransition {
                from: format!("{:?}", self.current_status),
                to: format!("{next:?}"),
            });
        }
        self.current_status = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CageKind {
    Standard,
    Breeding,
    Experimental,
    Temporary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cage {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub section: String,
    pub display_id: String,
    pub location: Option<String>,
    pub kind: CageKind,
    pub capacity: i32,
    pub sort_order: i32,
    pub meta: RecordMeta,
}

impl Cage {
    pub fn new(
        lab_id: Uuid,
        section: impl Into<String>,
        display_id: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let section = section.into();
        let display_id = display_id.into();
        require_non_empty("cage.section", &section)?;
        require_non_empty("cage.display_id", &display_id)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            section,
            display_id,
            location: None,
            kind: CageKind::Standard,
            capacity: 5,
            sort_order: 0,
            meta: RecordMeta::new(now),
        })
    }

    pub fn set_capacity(&mut self, capacity: i32) -> Result<(), DomainError> {
        if capacity <= 0 {
            return Err(DomainError::InvalidCageCapacity);
        }
        self.capacity = capacity;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalEvent {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub kind: AnimalEventKind,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
}

impl AnimalEvent {
    pub fn new(
        lab_id: Uuid,
        animal_id: Uuid,
        kind: AnimalEventKind,
        occurred_at: DateTime<Utc>,
        recorded_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id: None,
            animal_id,
            kind,
            occurred_at,
            recorded_at,
            recorded_by: None,
            notes: None,
        }
    }
}

/// Atomic command for moving one or more animals into a cage.
///
/// Store adapters validate the target cage and capacity inside the same
/// transaction that appends transfer events and updates animal projections.
pub const MAX_TRANSFER_ANIMALS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalTransfer {
    pub lab_id: Uuid,
    pub animal_ids: Vec<Uuid>,
    pub target_cage_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
}

impl AnimalTransfer {
    pub fn new(
        lab_id: Uuid,
        animal_ids: Vec<Uuid>,
        target_cage_id: Uuid,
        occurred_at: DateTime<Utc>,
        recorded_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let transfer = Self {
            lab_id,
            animal_ids,
            target_cage_id,
            occurred_at,
            recorded_at,
            recorded_by: None,
            notes: None,
        };
        transfer.validate()?;
        Ok(transfer)
    }

    /// Revalidates deserialized commands before a persistence adapter trusts
    /// them. This keeps transport-specific limits from becoming a bypass.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.animal_ids.is_empty() {
            return Err(DomainError::EmptyAnimalSelection);
        }
        if self.animal_ids.len() > MAX_TRANSFER_ANIMALS {
            return Err(DomainError::TransferSelectionTooLarge {
                maximum: MAX_TRANSFER_ANIMALS,
            });
        }
        let unique_count = self
            .animal_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if unique_count != self.animal_ids.len() {
            return Err(DomainError::DuplicateAnimalSelection);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnimalEventKind {
    Registered,
    Born {
        birth_date: NaiveDate,
    },
    Transferred {
        from_cage_id: Option<Uuid>,
        to_cage_id: Option<Uuid>,
    },
    StatusChanged {
        from: AnimalStatus,
        to: AnimalStatus,
    },
    Genotyped {
        genotype_ids: Vec<Uuid>,
    },
    GenotypingRecorded {
        record_id: Uuid,
        genotype_definition_id: Uuid,
        state: crate::GenotypingState,
    },
    ExperimentEnrolled {
        participation_id: Uuid,
    },
    ExperimentParticipationEnded {
        participation_id: Uuid,
        status: ParticipationStatus,
    },
    ProcedurePerformed {
        procedure_id: Uuid,
    },
    MeasurementRecorded {
        measurement_id: Uuid,
    },
    SampleCollected {
        sample_id: Uuid,
        terminal: bool,
    },
    Note {
        body: String,
    },
}

impl AnimalEventKind {
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Born { .. } => "born",
            Self::Transferred { .. } => "transferred",
            Self::StatusChanged { .. } => "status_changed",
            Self::Genotyped { .. } => "genotyped",
            Self::GenotypingRecorded { .. } => "genotyping_recorded",
            Self::ExperimentEnrolled { .. } => "experiment_enrolled",
            Self::ExperimentParticipationEnded { .. } => "experiment_participation_ended",
            Self::ProcedurePerformed { .. } => "procedure_performed",
            Self::MeasurementRecorded { .. } => "measurement_recorded",
            Self::SampleCollected { .. } => "sample_collected",
            Self::Note { .. } => "note",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneLocus {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub symbol: String,
    pub description: Option<String>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allele {
    pub id: Uuid,
    pub locus_id: Uuid,
    pub symbol: String,
    pub description: Option<String>,
    pub is_wild_type: bool,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Genotype {
    pub id: Uuid,
    pub animal_id: Uuid,
    pub locus_id: Uuid,
    pub allele_1_id: Option<Uuid>,
    pub allele_2_id: Option<Uuid>,
    pub assessed_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentType {
    Father,
    Mother,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pedigree {
    pub id: Uuid,
    pub animal_id: Uuid,
    pub parent_id: Uuid,
    pub parent_type: ParentType,
    pub meta: RecordMeta,
}
