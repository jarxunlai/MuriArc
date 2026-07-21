use chrono::{DateTime, Utc};
use muriarc_core::{AuditContext, Measurement, MeasurementValue, MuriArcStore, Sample, StoreError};
use uuid::Uuid;

use crate::validation::{ensure_max_bytes, normalized_optional, normalized_required};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_MEASUREMENT_KEY_CHARS: usize = 128;
pub const MAX_MEASUREMENT_LABEL_CHARS: usize = 256;
pub const MAX_MEASUREMENT_UNIT_CHARS: usize = 64;
pub const MAX_MEASUREMENT_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_SAMPLE_TYPE_CHARS: usize = 256;
pub const MAX_SAMPLE_UNIT_CHARS: usize = 64;
pub const MAX_SAMPLE_LOCATION_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct CreateMeasurementCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub procedure_id: Option<Uuid>,
    pub key: String,
    pub label: String,
    pub value: MeasurementValue,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

pub async fn create_measurement(
    store: &dyn MuriArcStore,
    command: CreateMeasurementCommand,
    audit: &AuditContext,
) -> ApplicationResult<Measurement> {
    let key = normalized_required("measurement.key", command.key, MAX_MEASUREMENT_KEY_CHARS)?;
    let label = normalized_required(
        "measurement.label",
        command.label,
        MAX_MEASUREMENT_LABEL_CHARS,
    )?;
    let unit = normalized_optional("measurement.unit", command.unit, MAX_MEASUREMENT_UNIT_CHARS)?;
    match &command.value {
        MeasurementValue::Number(_) if unit.is_none() => {
            return Err(ApplicationError::Validation(
                "numeric measurements require a unit".to_owned(),
            ));
        }
        MeasurementValue::Number(_) => {}
        MeasurementValue::Text(value) | MeasurementValue::Category(value) => {
            ensure_max_bytes("measurement.value", value, MAX_MEASUREMENT_TEXT_BYTES)?;
            if unit.is_some() {
                return Err(ApplicationError::Validation(
                    "only numeric measurements may define a unit".to_owned(),
                ));
            }
        }
        MeasurementValue::Boolean(_) | MeasurementValue::Date(_) if unit.is_some() => {
            return Err(ApplicationError::Validation(
                "only numeric measurements may define a unit".to_owned(),
            ));
        }
        MeasurementValue::Boolean(_) | MeasurementValue::Date(_) => {}
    }

    let mut measurement = Measurement::draft(
        command.lab_id,
        command.project_id,
        command.animal_id,
        key,
        label,
        command.value,
        command.measured_at,
        command.now,
    )?;
    measurement.experiment_id = command.experiment_id;
    measurement.procedure_id = command.procedure_id;
    measurement.unit = unit;
    store.create_measurement(&measurement, audit).await?;
    Ok(measurement)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignMeasurementCommand {
    pub id: Uuid,
    pub expected_revision: i64,
    pub signed_by: Uuid,
    pub signed_at: DateTime<Utc>,
}

pub async fn sign_measurement(
    store: &dyn MuriArcStore,
    command: SignMeasurementCommand,
    audit: &AuditContext,
) -> ApplicationResult<Measurement> {
    let mut measurement = store.get_measurement(command.id).await?;
    if measurement.meta.revision != command.expected_revision {
        return Err(StoreError::Conflict(
            "measurement revision changed before the signature was requested".to_owned(),
        )
        .into());
    }
    measurement.sign(command.signed_by, command.signed_at)?;
    store
        .update_measurement(&measurement, command.expected_revision, audit)
        .await?;
    Ok(measurement)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateSampleCommand {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub sample_type: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub location: Option<String>,
    pub collected_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

pub async fn create_sample(
    store: &dyn MuriArcStore,
    command: CreateSampleCommand,
    audit: &AuditContext,
) -> ApplicationResult<Sample> {
    let sample_type = normalized_required(
        "sample.sample_type",
        command.sample_type,
        MAX_SAMPLE_TYPE_CHARS,
    )?;
    let unit = normalized_optional("sample.unit", command.unit, MAX_SAMPLE_UNIT_CHARS)?;
    let location = normalized_optional(
        "sample.location",
        command.location,
        MAX_SAMPLE_LOCATION_CHARS,
    )?;
    let mut sample = Sample::new(
        command.lab_id,
        command.project_id,
        command.animal_id,
        sample_type,
        command.collected_at,
        command.now,
    )?;
    sample.experiment_id = command.experiment_id;
    sample.location = location;
    match (command.quantity, unit) {
        (Some(quantity), Some(unit)) => sample.set_quantity(quantity, unit)?,
        (Some(_), None) => {
            return Err(ApplicationError::Validation(
                "unit is required when sample quantity is provided".to_owned(),
            ));
        }
        (None, unit) => sample.unit = unit,
    }
    store.create_sample(&sample, audit).await?;
    Ok(sample)
}
