use std::collections::HashSet;

use chrono::{DateTime, Utc};
use muriarc_core::{
    AuditContext, Cohort, Experiment, ExperimentStatus, ExperimentTemplateVersion, FieldValueType,
    MuriArcStore, Participation, ParticipationStatus, Procedure, ProcedureStatus, TemplateField,
};
use serde_json::Value;
use uuid::Uuid;

use crate::validation::{
    normalized_optional, normalized_optional_bytes, normalized_required, normalized_required_bytes,
};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_EXPERIMENT_NAME_CHARS: usize = 256;
pub const MAX_EXPERIMENT_DESCRIPTION_CHARS: usize = 8_000;
pub const MAX_TEMPLATE_KEY_BYTES: usize = 128;
pub const MAX_TEMPLATE_NAME_BYTES: usize = 256;
pub const MAX_TEMPLATE_DESCRIPTION_BYTES: usize = 8_000;
pub const MAX_TEMPLATE_FIELD_KEY_BYTES: usize = 128;
pub const MAX_TEMPLATE_FIELD_LABEL_BYTES: usize = 256;
pub const MAX_TEMPLATE_UNIT_BYTES: usize = 64;
pub const MAX_TEMPLATE_FIELDS: usize = 128;
pub const MAX_TEMPLATE_FIELD_CATEGORIES: usize = 64;
pub const MAX_TEMPLATE_CATEGORY_BYTES: usize = 256;
pub const MAX_TEMPLATE_VERSION: i32 = 1_000_000;
pub const MAX_PROCEDURE_DETAILS_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateExperimentCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub template_version_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

pub async fn create_experiment(
    store: &dyn MuriArcStore,
    command: CreateExperimentCommand,
    audit: &AuditContext,
) -> ApplicationResult<Experiment> {
    let name = normalized_required("experiment.name", command.name, MAX_EXPERIMENT_NAME_CHARS)?;
    let mut experiment = Experiment::new(command.lab_id, command.project_id, name, command.now)?;
    experiment.description = normalized_optional(
        "experiment.description",
        command.description,
        MAX_EXPERIMENT_DESCRIPTION_CHARS,
    )?;
    experiment.template_version_id = command.template_version_id;
    experiment.starts_at = command.starts_at;
    store.create_experiment(&experiment, audit).await?;
    Ok(experiment)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionExperimentCommand {
    pub id: Uuid,
    pub target: ExperimentStatus,
    pub expected_revision: i64,
    pub occurred_at: DateTime<Utc>,
}

pub async fn transition_experiment(
    store: &dyn MuriArcStore,
    command: TransitionExperimentCommand,
    audit: &AuditContext,
) -> ApplicationResult<Experiment> {
    Ok(store
        .transition_experiment(
            command.id,
            command.target,
            command.expected_revision,
            command.occurred_at,
            audit,
        )
        .await?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTemplateVersionCommand {
    pub lab_id: Uuid,
    pub template_key: String,
    pub version: i32,
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<TemplateField>,
    pub now: DateTime<Utc>,
}

pub async fn create_template_version(
    store: &dyn MuriArcStore,
    command: CreateTemplateVersionCommand,
    audit: &AuditContext,
) -> ApplicationResult<ExperimentTemplateVersion> {
    if !(1..=MAX_TEMPLATE_VERSION).contains(&command.version) {
        return Err(ApplicationError::Validation(format!(
            "template version must be between 1 and {MAX_TEMPLATE_VERSION}"
        )));
    }
    if command.fields.len() > MAX_TEMPLATE_FIELDS {
        return Err(ApplicationError::Validation(format!(
            "template fields must not contain more than {MAX_TEMPLATE_FIELDS} entries"
        )));
    }
    let template_key = validated_template_key(command.template_key)?;
    let name = normalized_required_bytes("template.name", command.name, MAX_TEMPLATE_NAME_BYTES)?;
    let description = normalized_optional_bytes(
        "template.description",
        command.description,
        MAX_TEMPLATE_DESCRIPTION_BYTES,
    )?;
    let fields = validated_template_fields(command.fields)?;

    let mut template = ExperimentTemplateVersion::draft(
        command.lab_id,
        template_key,
        command.version,
        name,
        command.now,
    )?;
    template.description = description;
    template.replace_fields(fields, command.now)?;
    store.create_template_version(&template, audit).await?;
    Ok(template)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishTemplateVersionCommand {
    pub id: Uuid,
    pub expected_revision: i64,
    pub published_by: Uuid,
    pub published_at: DateTime<Utc>,
}

pub async fn publish_template_version(
    store: &dyn MuriArcStore,
    command: PublishTemplateVersionCommand,
    audit: &AuditContext,
) -> ApplicationResult<ExperimentTemplateVersion> {
    Ok(store
        .publish_template_version(
            command.id,
            command.expected_revision,
            command.published_by,
            command.published_at,
            audit,
        )
        .await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCohortCommand {
    pub experiment_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_cohort(
    store: &dyn MuriArcStore,
    command: CreateCohortCommand,
    audit: &AuditContext,
) -> ApplicationResult<Cohort> {
    let name = normalized_required("cohort.name", command.name, MAX_EXPERIMENT_NAME_CHARS)?;
    let mut cohort = Cohort::new(command.experiment_id, name, command.now)?;
    cohort.description = normalized_optional(
        "cohort.description",
        command.description,
        MAX_EXPERIMENT_DESCRIPTION_CHARS,
    )?;
    store.create_cohort(&cohort, audit).await?;
    Ok(cohort)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateParticipationCommand {
    pub experiment_id: Uuid,
    pub animal_id: Uuid,
    pub cohort_id: Option<Uuid>,
    pub enrolled_at: DateTime<Utc>,
}

pub async fn create_participation(
    store: &dyn MuriArcStore,
    command: CreateParticipationCommand,
    audit: &AuditContext,
) -> ApplicationResult<Participation> {
    let mut participation = Participation::enroll(
        command.experiment_id,
        command.animal_id,
        command.enrolled_at,
    );
    participation.cohort_id = command.cohort_id;
    Ok(store.create_participation(&participation, audit).await?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionParticipationCommand {
    pub id: Uuid,
    pub target: ParticipationStatus,
    pub expected_revision: i64,
    pub occurred_at: DateTime<Utc>,
}

pub async fn transition_participation(
    store: &dyn MuriArcStore,
    command: TransitionParticipationCommand,
    audit: &AuditContext,
) -> ApplicationResult<Participation> {
    Ok(store
        .transition_participation(
            command.id,
            command.target,
            command.expected_revision,
            command.occurred_at,
            audit,
        )
        .await?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateProcedureCommand {
    pub experiment_id: Uuid,
    pub animal_id: Option<Uuid>,
    pub name: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub performed_at: Option<DateTime<Utc>>,
    pub status: ProcedureStatus,
    pub details: Value,
    pub now: DateTime<Utc>,
}

pub async fn create_procedure(
    store: &dyn MuriArcStore,
    command: CreateProcedureCommand,
    audit: &AuditContext,
) -> ApplicationResult<Procedure> {
    let name = normalized_required("procedure.name", command.name, MAX_EXPERIMENT_NAME_CHARS)?;
    let encoded = serde_json::to_vec(&command.details)
        .map_err(|error| ApplicationError::Validation(error.to_string()))?;
    if encoded.len() > MAX_PROCEDURE_DETAILS_BYTES {
        return Err(ApplicationError::TooManyBytes {
            field: "procedure.details",
            max: MAX_PROCEDURE_DETAILS_BYTES,
        });
    }
    let procedure = Procedure::new(
        command.experiment_id,
        command.animal_id,
        name,
        command.status,
        command.scheduled_at,
        command.performed_at,
        command.details,
        command.now,
    )?;
    store.create_procedure(&procedure, audit).await?;
    Ok(procedure)
}

fn validated_template_key(value: String) -> ApplicationResult<String> {
    let value = normalized_required_bytes("template.template_key", value, MAX_TEMPLATE_KEY_BYTES)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ApplicationError::Validation(
            "template key may contain only ASCII letters, digits, '.', '-' and '_'".to_owned(),
        ));
    }
    Ok(value)
}

fn validated_template_fields(fields: Vec<TemplateField>) -> ApplicationResult<Vec<TemplateField>> {
    let mut keys = HashSet::with_capacity(fields.len());
    fields
        .into_iter()
        .map(|field| {
            let key = normalized_required_bytes(
                "template.fields[].key",
                field.key,
                MAX_TEMPLATE_FIELD_KEY_BYTES,
            )?;
            if !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                return Err(ApplicationError::Validation(format!(
                    "template field key '{key}' contains unsupported characters"
                )));
            }
            if !keys.insert(key.clone()) {
                return Err(ApplicationError::Validation(format!(
                    "template field key '{key}' is duplicated"
                )));
            }
            let label = normalized_required_bytes(
                "template.fields[].label",
                field.label,
                MAX_TEMPLATE_FIELD_LABEL_BYTES,
            )?;
            let unit = normalized_optional_bytes(
                "template.fields[].unit",
                field.unit,
                MAX_TEMPLATE_UNIT_BYTES,
            )?;
            if field.categories.len() > MAX_TEMPLATE_FIELD_CATEGORIES {
                return Err(ApplicationError::Validation(format!(
                    "template field categories must not contain more than {MAX_TEMPLATE_FIELD_CATEGORIES} entries"
                )));
            }
            let mut category_set = HashSet::with_capacity(field.categories.len());
            let categories = field
                .categories
                .into_iter()
                .map(|category| {
                    let category = normalized_required_bytes(
                        "template.fields[].categories[]",
                        category,
                        MAX_TEMPLATE_CATEGORY_BYTES,
                    )?;
                    if !category_set.insert(category.clone()) {
                        return Err(ApplicationError::Validation(
                            "template field categories must not contain duplicates".to_owned(),
                        ));
                    }
                    Ok(category)
                })
                .collect::<ApplicationResult<Vec<_>>>()?;

            if field.minimum.is_some_and(|value| !value.is_finite())
                || field.maximum.is_some_and(|value| !value.is_finite())
            {
                return Err(ApplicationError::Validation(
                    "template field bounds must be finite".to_owned(),
                ));
            }
            if matches!(
                (field.minimum, field.maximum),
                (Some(minimum), Some(maximum)) if minimum > maximum
            ) {
                return Err(ApplicationError::Validation(
                    "template field minimum must not exceed maximum".to_owned(),
                ));
            }
            match field.value_type {
                FieldValueType::Number if !categories.is_empty() => {
                    return Err(ApplicationError::Validation(
                        "numeric template fields cannot define categories".to_owned(),
                    ));
                }
                FieldValueType::Category => {
                    if categories.is_empty() {
                        return Err(ApplicationError::Validation(
                            "category template fields require at least one category".to_owned(),
                        ));
                    }
                    if unit.is_some() || field.minimum.is_some() || field.maximum.is_some() {
                        return Err(ApplicationError::Validation(
                            "category template fields cannot define unit or numeric bounds".to_owned(),
                        ));
                    }
                }
                FieldValueType::Text | FieldValueType::Boolean | FieldValueType::Date => {
                    if unit.is_some()
                        || !categories.is_empty()
                        || field.minimum.is_some()
                        || field.maximum.is_some()
                    {
                        return Err(ApplicationError::Validation(
                            "only numeric fields may define unit/bounds and only category fields may define categories"
                                .to_owned(),
                        ));
                    }
                }
                FieldValueType::Number => {}
            }

            Ok(TemplateField {
                key,
                label,
                value_type: field.value_type,
                unit,
                required: field.required,
                categories,
                minimum: field.minimum,
                maximum: field.maximum,
                display_order: field.display_order,
                ai_writable: field.ai_writable,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_validation_rejects_duplicate_field_keys() {
        let field = TemplateField {
            key: "weight".to_owned(),
            label: "Weight".to_owned(),
            value_type: FieldValueType::Number,
            unit: Some("g".to_owned()),
            required: true,
            categories: Vec::new(),
            minimum: None,
            maximum: None,
            display_order: 0,
            ai_writable: false,
        };
        let error = validated_template_fields(vec![field.clone(), field]).unwrap_err();
        assert!(matches!(error, ApplicationError::Validation(_)));
    }
}
