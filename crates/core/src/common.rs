use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordMeta {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub revision: i64,
}

impl RecordMeta {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            created_at: now,
            updated_at: now,
            deleted_at: None,
            revision: 1,
        }
    }

    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
        self.revision += 1;
    }

    pub fn soft_delete(&mut self, now: DateTime<Utc>) {
        self.deleted_at = Some(now);
        self.touch(now);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Lab,
    User,
    UserCredential,
    AuthSession,
    ExternalToken,
    Project,
    Membership,
    ProjectAnimalAssignment,
    Cage,
    Animal,
    AnimalEvent,
    GeneLocus,
    Allele,
    Genotype,
    GenotypeDefinition,
    GenotypingRecord,
    BreedingLine,
    Colony,
    BreedingPair,
    BreedingPairMember,
    MatingEvent,
    Litter,
    AnimalDraft,
    Pedigree,
    ExperimentEvent,
    ObservationDefinition,
    Observation,
    ObservationValue,
    ExperimentTemplateVersion,
    Experiment,
    Cohort,
    Participation,
    Procedure,
    Measurement,
    Sample,
    Attachment,
    AttachmentLink,
    AttachmentDerivative,
    AiPrivateImage,
    AiExtractionDraft,
    AiConversation,
    AiConversationMessage,
    AiAutonomyGrant,
    AiProviderSettings,
    AiProviderEndpoint,
    AiLabSettings,
    TechnicalLogPolicy,
    ToolRun,
    Approval,
    Job,
    Provenance,
}

impl EntityType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lab => "lab",
            Self::User => "user",
            Self::UserCredential => "user_credential",
            Self::AuthSession => "auth_session",
            Self::ExternalToken => "external_token",
            Self::Project => "project",
            Self::Membership => "membership",
            Self::ProjectAnimalAssignment => "project_animal_assignment",
            Self::Cage => "cage",
            Self::Animal => "animal",
            Self::AnimalEvent => "animal_event",
            Self::GeneLocus => "gene_locus",
            Self::Allele => "allele",
            Self::Genotype => "genotype",
            Self::GenotypeDefinition => "genotype_definition",
            Self::GenotypingRecord => "genotyping_record",
            Self::BreedingLine => "breeding_line",
            Self::Colony => "colony",
            Self::BreedingPair => "breeding_pair",
            Self::BreedingPairMember => "breeding_pair_member",
            Self::MatingEvent => "mating_event",
            Self::Litter => "litter",
            Self::AnimalDraft => "animal_draft",
            Self::Pedigree => "pedigree",
            Self::ExperimentEvent => "experiment_event",
            Self::ObservationDefinition => "observation_definition",
            Self::Observation => "observation",
            Self::ObservationValue => "observation_value",
            Self::ExperimentTemplateVersion => "experiment_template_version",
            Self::Experiment => "experiment",
            Self::Cohort => "cohort",
            Self::Participation => "participation",
            Self::Procedure => "procedure",
            Self::Measurement => "measurement",
            Self::Sample => "sample",
            Self::Attachment => "attachment",
            Self::AttachmentLink => "attachment_link",
            Self::AttachmentDerivative => "attachment_derivative",
            Self::AiPrivateImage => "ai_private_image",
            Self::AiExtractionDraft => "ai_extraction_draft",
            Self::AiConversation => "ai_conversation",
            Self::AiConversationMessage => "ai_conversation_message",
            Self::AiAutonomyGrant => "ai_autonomy_grant",
            Self::AiProviderSettings => "ai_provider_settings",
            Self::AiProviderEndpoint => "ai_provider_endpoint",
            Self::AiLabSettings => "ai_lab_settings",
            Self::TechnicalLogPolicy => "technical_log_policy",
            Self::ToolRun => "tool_run",
            Self::Approval => "approval",
            Self::Job => "job",
            Self::Provenance => "provenance",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("event belongs to animal {actual}, expected {expected}")]
    EventAnimalMismatch { expected: Uuid, actual: Uuid },
    #[error("invalid animal status transition from {from} to {to}")]
    InvalidStatusTransition { from: String, to: String },
    #[error("published template versions are immutable")]
    PublishedTemplateImmutable,
    #[error("only draft template versions can be published")]
    TemplateNotDraft,
    #[error("measurement value does not match its declared value type")]
    MeasurementTypeMismatch,
    #[error("numeric measurement value must be finite")]
    NonFiniteMeasurement,
    #[error("only draft measurements can be signed")]
    MeasurementNotDraft,
    #[error("measurement signature fields do not match its record status")]
    InvalidMeasurementSignatureState,
    #[error("quantity must be finite and non-negative")]
    InvalidQuantity,
    #[error("cage capacity must be greater than zero")]
    InvalidCageCapacity,
    #[error("at least one animal must be selected")]
    EmptyAnimalSelection,
    #[error("animal selection must not contain duplicates")]
    DuplicateAnimalSelection,
    #[error("animal transfer cannot contain more than {maximum} animals")]
    TransferSelectionTooLarge { maximum: usize },
    #[error("experiment can only be completed or cancelled while it is open")]
    ExperimentNotOpen,
    #[error("experiment lifecycle target must be completed or cancelled")]
    InvalidExperimentTransition,
    #[error("participation can only be completed or withdrawn while it is enrolled")]
    ParticipationNotEnrolled,
    #[error("participation lifecycle target must be completed or withdrawn")]
    InvalidParticipationTransition,
    #[error("completed procedures require performed_at and other procedure states forbid it")]
    InvalidProcedureState,
    #[error("procedure details must be a JSON object")]
    InvalidProcedureDetails,
    #[error("genotype component configuration is invalid")]
    InvalidGenotypeComponent,
    #[error("genotype definition must contain valid components owned by the definition")]
    InvalidGenotypeDefinition,
    #[error("genotyping record state and assessment fields are invalid")]
    InvalidGenotypingRecord,
    #[error("breeding line configuration is invalid")]
    InvalidBreedingLine,
    #[error("colony configuration is invalid")]
    InvalidColony,
    #[error("breeding pair must contain exactly one active male and at least one active female")]
    InvalidBreedingPair,
    #[error("breeding pair member configuration is invalid")]
    InvalidBreedingMember,
    #[error("breeding pair is not active")]
    BreedingPairNotActive,
    #[error("mating event configuration is invalid")]
    InvalidMatingEvent,
    #[error("litter counts or relationships are invalid")]
    InvalidLitter,
    #[error("animal draft configuration is invalid")]
    InvalidAnimalDraft,
    #[error("animal draft is no longer pending registration")]
    AnimalDraftNotPending,
    #[error("parent genotype definitions cannot be used for one prediction")]
    IncompatibleBreedingPrediction,
    #[error("experiment event configuration is invalid")]
    InvalidExperimentEvent,
    #[error("observation definition configuration is invalid")]
    InvalidObservationDefinition,
    #[error("observation value does not match its definition")]
    ObservationValueTypeMismatch,
    #[error("observation relationship or context is invalid")]
    InvalidObservation,
    #[error("immutable observations cannot be revised")]
    ObservationImmutable,
    #[error("membership role does not match its membership scope")]
    InvalidMembershipScope,
    #[error("AI conversation message is invalid")]
    InvalidAiConversationMessage,
    #[error("user email must be a valid non-whitespace address of at most 320 bytes")]
    InvalidUserEmail,
    #[error("user display name must contain 1-200 non-control characters")]
    InvalidUserDisplayName,
}

pub(crate) fn require_non_empty(field: &'static str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::EmptyField { field })
    } else {
        Ok(())
    }
}
