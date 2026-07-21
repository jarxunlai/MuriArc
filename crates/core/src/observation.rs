use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{DomainError, RecordMeta, require_non_empty};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentEvent {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub event_key: String,
    pub label: String,
    pub occurred_at: DateTime<Utc>,
    pub details: Value,
    pub meta: RecordMeta,
}

impl ExperimentEvent {
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        experiment_id: Uuid,
        event_key: impl Into<String>,
        label: impl Into<String>,
        occurred_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let event_key = event_key.into();
        let label = label.into();
        require_non_empty("experiment_event.event_key", &event_key)?;
        require_non_empty("experiment_event.label", &label)?;
        if lab_id.is_nil() || project_id.is_nil() || experiment_id.is_nil() {
            return Err(DomainError::InvalidExperimentEvent);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            experiment_id,
            event_key: event_key.trim().to_owned(),
            label: label.trim().to_owned(),
            occurred_at,
            details: Value::Object(Map::new()),
            meta: RecordMeta::new(now),
        })
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || self.event_key.trim().is_empty()
            || self.label.trim().is_empty()
            || !self.details.is_object()
        {
            Err(DomainError::InvalidExperimentEvent)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationValueType {
    Number,
    Text,
    Boolean,
    Date,
    Category,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationPolicy {
    Immutable,
    Mutable,
    Versioned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationDefinition {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub key: String,
    pub label: String,
    pub value_type: ObservationValueType,
    pub unit: Option<String>,
    pub categories: Vec<String>,
    pub policy: ObservationPolicy,
    pub meta: RecordMeta,
}

impl ObservationDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        experiment_id: Uuid,
        key: impl Into<String>,
        label: impl Into<String>,
        value_type: ObservationValueType,
        policy: ObservationPolicy,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let key = key.into();
        let label = label.into();
        require_non_empty("observation_definition.key", &key)?;
        require_non_empty("observation_definition.label", &label)?;
        if lab_id.is_nil() || project_id.is_nil() || experiment_id.is_nil() {
            return Err(DomainError::InvalidObservationDefinition);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            experiment_id,
            key: key.trim().to_owned(),
            label: label.trim().to_owned(),
            value_type,
            unit: None,
            categories: Vec::new(),
            policy,
            meta: RecordMeta::new(now),
        })
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let category_configuration = if self.value_type == ObservationValueType::Category {
            let mut unique = HashSet::with_capacity(self.categories.len());
            !self.categories.is_empty()
                && self
                    .categories
                    .iter()
                    .all(|value| !value.trim().is_empty() && unique.insert(value))
        } else {
            self.categories.is_empty()
        };
        let unit_configuration = if self.value_type == ObservationValueType::Number {
            self.unit
                .as_deref()
                .is_some_and(|unit| !unit.trim().is_empty())
        } else {
            self.unit.is_none()
        };
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || self.key.trim().is_empty()
            || self.label.trim().is_empty()
            || !category_configuration
            || !unit_configuration
        {
            Err(DomainError::InvalidObservationDefinition)
        } else {
            Ok(())
        }
    }

    pub fn validate_value(&self, value: &ObservationValueData) -> Result<(), DomainError> {
        let valid = match (self.value_type, value) {
            (ObservationValueType::Number, ObservationValueData::Number(value)) => {
                value.is_finite()
            }
            (ObservationValueType::Text, ObservationValueData::Text(_))
            | (ObservationValueType::Boolean, ObservationValueData::Boolean(_))
            | (ObservationValueType::Date, ObservationValueData::Date(_))
            | (ObservationValueType::Json, ObservationValueData::Json(_)) => true,
            (ObservationValueType::Category, ObservationValueData::Category(value)) => {
                self.categories.contains(value)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(DomainError::ObservationValueTypeMismatch)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSubjectType {
    Experiment,
    Animal,
    Sample,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ObservationValueData {
    Number(f64),
    Text(String),
    Boolean(bool),
    Date(NaiveDate),
    Category(String),
    Json(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub experiment_event_id: Uuid,
    pub definition_id: Uuid,
    pub subject_type: ObservationSubjectType,
    pub subject_id: Uuid,
    pub context: Value,
    pub current_value_version: i32,
    pub meta: RecordMeta,
}

impl Observation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        experiment_id: Uuid,
        experiment_event_id: Uuid,
        definition_id: Uuid,
        subject_type: ObservationSubjectType,
        subject_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if lab_id.is_nil()
            || project_id.is_nil()
            || experiment_id.is_nil()
            || experiment_event_id.is_nil()
            || definition_id.is_nil()
            || subject_id.is_nil()
        {
            return Err(DomainError::InvalidObservation);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            experiment_id,
            experiment_event_id,
            definition_id,
            subject_type,
            subject_id,
            context: Value::Object(Map::new()),
            current_value_version: 1,
            meta: RecordMeta::new(now),
        })
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || self.experiment_event_id.is_nil()
            || self.definition_id.is_nil()
            || self.subject_id.is_nil()
            || !self.context.is_object()
            || self.current_value_version < 1
        {
            Err(DomainError::InvalidObservation)
        } else {
            Ok(())
        }
    }

    pub fn advance_value_version(
        &mut self,
        next_version: i32,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if next_version != self.current_value_version + 1 {
            return Err(DomainError::InvalidObservation);
        }
        self.current_value_version = next_version;
        self.meta.touch(now);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueRecord {
    pub id: Uuid,
    pub observation_id: Uuid,
    pub version: i32,
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<Uuid>,
    pub notes: Option<String>,
    pub meta: RecordMeta,
}

impl ObservationValueRecord {
    pub fn new(
        observation_id: Uuid,
        version: i32,
        value: ObservationValueData,
        recorded_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let record = Self {
            id: Uuid::new_v4(),
            observation_id,
            version,
            value,
            recorded_at,
            recorded_by: None,
            notes: None,
            meta: RecordMeta::new(now),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.observation_id.is_nil()
            || self.version < 1
            || matches!(self.value, ObservationValueData::Number(value) if !value.is_finite())
        {
            Err(DomainError::ObservationValueTypeMismatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationFilter {
    pub experiment_id: Uuid,
    pub experiment_event_id: Option<Uuid>,
    pub subject_type: Option<ObservationSubjectType>,
    pub subject_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_definition_requires_unit_and_rejects_text() {
        let now = Utc::now();
        let mut definition = ObservationDefinition::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "weight",
            "Body weight",
            ObservationValueType::Number,
            ObservationPolicy::Versioned,
            now,
        )
        .unwrap();
        assert_eq!(
            definition.validate().unwrap_err(),
            DomainError::InvalidObservationDefinition
        );
        definition.unit = Some("g".to_owned());
        definition.validate().unwrap();
        definition
            .validate_value(&ObservationValueData::Number(24.0))
            .unwrap();
        assert_eq!(
            definition
                .validate_value(&ObservationValueData::Text("24".to_owned()))
                .unwrap_err(),
            DomainError::ObservationValueTypeMismatch
        );
    }
}
