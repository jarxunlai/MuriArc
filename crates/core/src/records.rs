use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, FieldValueType, RecordMeta, require_non_empty};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MeasurementValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Date(NaiveDate),
    Category(String),
}

impl MeasurementValue {
    pub const fn value_type(&self) -> FieldValueType {
        match self {
            Self::Number(_) => FieldValueType::Number,
            Self::Text(_) => FieldValueType::Text,
            Self::Boolean(_) => FieldValueType::Boolean,
            Self::Date(_) => FieldValueType::Date,
            Self::Category(_) => FieldValueType::Category,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Draft,
    Signed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub procedure_id: Option<Uuid>,
    pub key: String,
    pub label: String,
    pub value_type: FieldValueType,
    pub value: MeasurementValue,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
    pub status: RecordStatus,
    pub signed_by: Option<Uuid>,
    pub signed_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

impl Measurement {
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        lab_id: Uuid,
        project_id: Uuid,
        animal_id: Uuid,
        key: impl Into<String>,
        label: impl Into<String>,
        value: MeasurementValue,
        measured_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let key = key.into();
        let label = label.into();
        require_non_empty("measurement.key", &key)?;
        require_non_empty("measurement.label", &label)?;
        if let MeasurementValue::Number(number) = &value
            && !number.is_finite()
        {
            return Err(DomainError::NonFiniteMeasurement);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            experiment_id: None,
            animal_id,
            procedure_id: None,
            key,
            label,
            value_type: value.value_type(),
            value,
            unit: None,
            measured_at,
            status: RecordStatus::Draft,
            signed_by: None,
            signed_at: None,
            meta: RecordMeta::new(now),
        })
    }

    pub fn validate_type(&self) -> Result<(), DomainError> {
        if self.value_type != self.value.value_type() {
            return Err(DomainError::MeasurementTypeMismatch);
        }
        if let MeasurementValue::Number(number) = &self.value
            && !number.is_finite()
        {
            return Err(DomainError::NonFiniteMeasurement);
        }
        Ok(())
    }

    pub fn validate_record(&self) -> Result<(), DomainError> {
        self.validate_type()?;
        match self.status {
            RecordStatus::Draft if self.signed_by.is_some() || self.signed_at.is_some() => {
                Err(DomainError::InvalidMeasurementSignatureState)
            }
            RecordStatus::Signed if self.signed_by.is_none() || self.signed_at.is_none() => {
                Err(DomainError::InvalidMeasurementSignatureState)
            }
            RecordStatus::Draft | RecordStatus::Signed => Ok(()),
        }
    }

    pub fn sign(&mut self, user_id: Uuid, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.status != RecordStatus::Draft {
            return Err(DomainError::MeasurementNotDraft);
        }
        self.validate_type()?;
        self.status = RecordStatus::Signed;
        self.signed_by = Some(user_id);
        self.signed_at = Some(now);
        self.meta.touch(now);
        self.validate_record()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub collection_event_id: Option<Uuid>,
    pub sample_type: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub location: Option<String>,
    pub collected_at: DateTime<Utc>,
    pub meta: RecordMeta,
}

impl Sample {
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        animal_id: Uuid,
        sample_type: impl Into<String>,
        collected_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let sample_type = sample_type.into();
        require_non_empty("sample.sample_type", &sample_type)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            experiment_id: None,
            animal_id,
            collection_event_id: None,
            sample_type,
            quantity: None,
            unit: None,
            location: None,
            collected_at,
            meta: RecordMeta::new(now),
        })
    }

    pub fn set_quantity(
        &mut self,
        quantity: f64,
        unit: impl Into<String>,
    ) -> Result<(), DomainError> {
        if !quantity.is_finite() || quantity < 0.0 {
            return Err(DomainError::InvalidQuantity);
        }
        self.quantity = Some(quantity);
        self.unit = Some(unit.into());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub file_name: String,
    pub media_type: Option<String>,
    pub relative_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub version: i32,
    pub meta: RecordMeta,
}
