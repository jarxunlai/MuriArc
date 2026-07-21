use chrono::{DateTime, Utc};
use muriarc_core::{
    AuditContext, ExperimentEvent, MuriArcStore, Observation, ObservationDefinition,
    ObservationPolicy, ObservationSubjectType, ObservationValueData, ObservationValueRecord,
    ObservationValueType,
};
use serde_json::Value;
use uuid::Uuid;

use crate::validation::{normalized_optional, normalized_optional_bytes, normalized_required};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_OBSERVATION_KEY_CHARS: usize = 128;
pub const MAX_OBSERVATION_LABEL_CHARS: usize = 256;
pub const MAX_OBSERVATION_UNIT_CHARS: usize = 64;
pub const MAX_OBSERVATION_CATEGORY_CHARS: usize = 256;
pub const MAX_OBSERVATION_NOTES_BYTES: usize = 16_000;

#[derive(Debug, Clone, PartialEq)]
pub struct CreateExperimentEventCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub event_key: String,
    pub label: String,
    pub occurred_at: DateTime<Utc>,
    pub details: Value,
    pub now: DateTime<Utc>,
}

pub async fn create_experiment_event(
    store: &dyn MuriArcStore,
    command: CreateExperimentEventCommand,
    audit: &AuditContext,
) -> ApplicationResult<ExperimentEvent> {
    let mut event = ExperimentEvent::new(
        command.lab_id,
        command.project_id,
        command.experiment_id,
        normalized_required(
            "experiment_event.event_key",
            command.event_key,
            MAX_OBSERVATION_KEY_CHARS,
        )?,
        normalized_required(
            "experiment_event.label",
            command.label,
            MAX_OBSERVATION_LABEL_CHARS,
        )?,
        command.occurred_at,
        command.now,
    )?;
    if !command.details.is_object() {
        return Err(ApplicationError::Validation(
            "experiment_event.details must be a JSON object".to_owned(),
        ));
    }
    event.details = command.details;
    event.validate()?;
    store.create_experiment_event(&event, audit).await?;
    Ok(event)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateObservationDefinitionCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub key: String,
    pub label: String,
    pub value_type: ObservationValueType,
    pub unit: Option<String>,
    pub categories: Vec<String>,
    pub policy: ObservationPolicy,
    pub now: DateTime<Utc>,
}

pub async fn create_observation_definition(
    store: &dyn MuriArcStore,
    command: CreateObservationDefinitionCommand,
    audit: &AuditContext,
) -> ApplicationResult<ObservationDefinition> {
    let mut definition = ObservationDefinition::new(
        command.lab_id,
        command.project_id,
        command.experiment_id,
        normalized_required(
            "observation_definition.key",
            command.key,
            MAX_OBSERVATION_KEY_CHARS,
        )?,
        normalized_required(
            "observation_definition.label",
            command.label,
            MAX_OBSERVATION_LABEL_CHARS,
        )?,
        command.value_type,
        command.policy,
        command.now,
    )?;
    definition.unit = normalized_optional(
        "observation_definition.unit",
        command.unit,
        MAX_OBSERVATION_UNIT_CHARS,
    )?;
    definition.categories = command
        .categories
        .into_iter()
        .map(|category| {
            normalized_required(
                "observation_definition.category",
                category,
                MAX_OBSERVATION_CATEGORY_CHARS,
            )
        })
        .collect::<ApplicationResult<Vec<_>>>()?;
    definition.validate()?;
    store
        .create_observation_definition(&definition, audit)
        .await?;
    Ok(definition)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordObservationCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub experiment_event_id: Uuid,
    pub definition_id: Uuid,
    pub subject_type: ObservationSubjectType,
    pub subject_id: Uuid,
    pub context: Value,
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedObservation {
    pub observation: Observation,
    pub value: ObservationValueRecord,
}

pub async fn record_observation(
    store: &dyn MuriArcStore,
    command: RecordObservationCommand,
    audit: &AuditContext,
) -> ApplicationResult<RecordedObservation> {
    if !command.context.is_object() {
        return Err(ApplicationError::Validation(
            "observation.context must be a JSON object".to_owned(),
        ));
    }
    let definition = store
        .get_observation_definition(command.definition_id)
        .await?;
    if definition.lab_id != command.lab_id
        || definition.project_id != command.project_id
        || definition.experiment_id != command.experiment_id
    {
        return Err(ApplicationError::Validation(
            "observation definition belongs to a different scope".to_owned(),
        ));
    }
    definition.validate_value(&command.value)?;
    let mut observation = Observation::new(
        command.lab_id,
        command.project_id,
        command.experiment_id,
        command.experiment_event_id,
        command.definition_id,
        command.subject_type,
        command.subject_id,
        command.now,
    )?;
    observation.context = command.context;
    observation.validate()?;
    let mut value = ObservationValueRecord::new(
        observation.id,
        1,
        command.value,
        command.recorded_at,
        command.now,
    )?;
    value.recorded_by = command.recorded_by;
    value.notes = normalized_optional_bytes(
        "observation_value.notes",
        command.notes,
        MAX_OBSERVATION_NOTES_BYTES,
    )?;
    store
        .create_observation(&observation, &value, audit)
        .await?;
    Ok(RecordedObservation { observation, value })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviseObservationValueCommand {
    pub observation_id: Uuid,
    pub expected_revision: i64,
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevisedObservation {
    pub observation: Observation,
    pub value: ObservationValueRecord,
}

pub async fn revise_observation_value(
    store: &dyn MuriArcStore,
    command: ReviseObservationValueCommand,
    audit: &AuditContext,
) -> ApplicationResult<RevisedObservation> {
    let observation = store.get_observation(command.observation_id).await?;
    let definition = store
        .get_observation_definition(observation.definition_id)
        .await?;
    definition.validate_value(&command.value)?;
    if definition.policy == ObservationPolicy::Immutable {
        return Err(ApplicationError::Domain(
            muriarc_core::DomainError::ObservationImmutable,
        ));
    }
    let mut value = ObservationValueRecord::new(
        observation.id,
        observation.current_value_version + 1,
        command.value,
        command.recorded_at,
        command.now,
    )?;
    value.recorded_by = command.recorded_by;
    value.notes = normalized_optional_bytes(
        "observation_value.notes",
        command.notes,
        MAX_OBSERVATION_NOTES_BYTES,
    )?;
    let observation = store
        .revise_observation_value(observation.id, command.expected_revision, &value, audit)
        .await?;
    Ok(RevisedObservation { observation, value })
}
