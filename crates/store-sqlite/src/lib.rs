mod ai_models;
mod ai_operations;
mod workspace;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr,
};

use chrono::{DateTime, NaiveDate, Utc};
use muriarc_core::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

fn validate_observation_recorder(
    value: &ObservationValueRecord,
    audit: &AuditContext,
) -> StoreResult<()> {
    if audit.actor.actor_type == ActorType::Human && value.recorded_by != audit.actor.user_id {
        return Err(StoreError::Validation(
            "observation value recorded_by must match the human audit actor".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> StoreResult<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(map_sqlx)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let max_connections = if database_url.contains(":memory:") {
            1
        } else {
            5
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;
        Ok(Self { pool })
    }

    pub async fn connect_path(path: impl AsRef<Path>) -> StoreResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;
        Ok(Self { pool })
    }

    pub async fn in_memory() -> StoreResult<Self> {
        Self::connect("sqlite::memory:").await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

fn map_sqlx(error: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(database) = &error
        && database.is_unique_violation()
    {
        return StoreError::Conflict(database.message().to_owned());
    }
    StoreError::Database(error.to_string())
}

fn encode<T: Serialize>(value: &T) -> StoreResult<String> {
    match serde_json::to_value(value).map_err(|e| StoreError::Serialization(e.to_string()))? {
        Value::String(value) => Ok(value),
        value => {
            serde_json::to_string(&value).map_err(|e| StoreError::Serialization(e.to_string()))
        }
    }
}

fn decode<T: DeserializeOwned>(value: &str) -> StoreResult<T> {
    serde_json::from_value(Value::String(value.to_owned()))
        .or_else(|_| serde_json::from_str(value))
        .map_err(|e| StoreError::Serialization(e.to_string()))
}

fn uuid(value: &str) -> StoreResult<Uuid> {
    Uuid::parse_str(value).map_err(|e| StoreError::Serialization(e.to_string()))
}

fn optional_uuid(value: Option<String>) -> StoreResult<Option<Uuid>> {
    value.map(|value| uuid(&value)).transpose()
}

fn checked_count(value: i64, label: &'static str) -> StoreResult<usize> {
    usize::try_from(value)
        .map_err(|_| StoreError::Database(format!("invalid {label} reference count")))
}

fn meta(row: &SqliteRow) -> StoreResult<RecordMeta> {
    Ok(RecordMeta {
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx)?,
        deleted_at: row.try_get("deleted_at").map_err(map_sqlx)?,
        revision: row.try_get("revision").map_err(map_sqlx)?,
    })
}

fn lab_from_row(row: &SqliteRow) -> StoreResult<Lab> {
    Ok(Lab {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn user_from_row(row: &SqliteRow) -> StoreResult<User> {
    Ok(User {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        email: row.try_get("email").map_err(map_sqlx)?,
        display_name: row.try_get("display_name").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}

fn membership_from_row(row: &SqliteRow) -> StoreResult<Membership> {
    let lab_role = row
        .try_get::<Option<String>, _>("lab_role")
        .map_err(map_sqlx)?
        .map(|value| decode(&value))
        .transpose()?;
    let project_role = row
        .try_get::<Option<String>, _>("project_role")
        .map_err(map_sqlx)?
        .map(|value| decode(&value))
        .transpose()?;
    Ok(Membership {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        lab_role,
        project_role,
        meta: meta(row)?,
    })
}

fn project_animal_assignment_from_row(row: &SqliteRow) -> StoreResult<ProjectAnimalAssignment> {
    Ok(ProjectAnimalAssignment {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        assigned_by: optional_uuid(row.try_get("assigned_by").map_err(map_sqlx)?)?,
        reason: row.try_get("reason").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn project_from_row(row: &SqliteRow) -> StoreResult<Project> {
    Ok(Project {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}

fn cage_from_row(row: &SqliteRow) -> StoreResult<Cage> {
    Ok(Cage {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        section: row.try_get("section").map_err(map_sqlx)?,
        display_id: row.try_get("display_id").map_err(map_sqlx)?,
        location: row.try_get("location").map_err(map_sqlx)?,
        kind: decode(row.try_get("kind").map_err(map_sqlx)?)?,
        capacity: row.try_get("capacity").map_err(map_sqlx)?,
        sort_order: row.try_get("sort_order").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn animal_from_row(row: &SqliteRow) -> StoreResult<Animal> {
    Ok(Animal {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        identifier_scope: IdentifierScope::from_storage_key(
            row.try_get("identifier_scope").map_err(map_sqlx)?,
        ),
        display_id: row.try_get("display_id").map_err(map_sqlx)?,
        legacy_id: row.try_get("legacy_id").map_err(map_sqlx)?,
        species: row.try_get("species").map_err(map_sqlx)?,
        strain: row.try_get("strain").map_err(map_sqlx)?,
        sex: decode(row.try_get("sex").map_err(map_sqlx)?)?,
        birth_date: row.try_get("birth_date").map_err(map_sqlx)?,
        death_date: row.try_get("death_date").map_err(map_sqlx)?,
        current_cage_id: optional_uuid(row.try_get("current_cage_id").map_err(map_sqlx)?)?,
        current_status: decode(row.try_get("current_status").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}

fn event_from_row(row: &SqliteRow) -> StoreResult<AnimalEvent> {
    Ok(AnimalEvent {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        kind: serde_json::from_str(row.try_get("payload_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        occurred_at: row.try_get("occurred_at").map_err(map_sqlx)?,
        recorded_at: row.try_get("recorded_at").map_err(map_sqlx)?,
        recorded_by: optional_uuid(row.try_get("recorded_by").map_err(map_sqlx)?)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
    })
}

fn experiment_from_row(row: &SqliteRow) -> StoreResult<Experiment> {
    Ok(Experiment {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        template_version_id: optional_uuid(row.try_get("template_version_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        starts_at: row.try_get("starts_at").map_err(map_sqlx)?,
        ends_at: row.try_get("ends_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}
fn participation_from_row(row: &SqliteRow) -> StoreResult<Participation> {
    Ok(Participation {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        cohort_id: optional_uuid(row.try_get("cohort_id").map_err(map_sqlx)?)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        enrolled_at: row.try_get("enrolled_at").map_err(map_sqlx)?,
        exited_at: row.try_get("exited_at").map_err(map_sqlx)?,
        genotype_snapshot: serde_json::from_str(
            row.try_get("genotype_snapshot_json").map_err(map_sqlx)?,
        )
        .map_err(|error| StoreError::Serialization(error.to_string()))?,
        meta: meta(row)?,
    })
}

fn measurement_from_row(row: &SqliteRow) -> StoreResult<Measurement> {
    let value_type: FieldValueType = decode(row.try_get("value_type").map_err(map_sqlx)?)?;
    let value = match value_type {
        FieldValueType::Number => MeasurementValue::Number(
            row.try_get::<Option<f64>, _>("value_number")
                .map_err(map_sqlx)?
                .ok_or_else(|| StoreError::Serialization("missing numeric value".to_owned()))?,
        ),
        FieldValueType::Text => MeasurementValue::Text(
            row.try_get::<Option<String>, _>("value_text")
                .map_err(map_sqlx)?
                .ok_or_else(|| StoreError::Serialization("missing text value".to_owned()))?,
        ),
        FieldValueType::Boolean => MeasurementValue::Boolean(
            row.try_get::<Option<i64>, _>("value_boolean")
                .map_err(map_sqlx)?
                .ok_or_else(|| StoreError::Serialization("missing boolean value".to_owned()))?
                != 0,
        ),
        FieldValueType::Date => MeasurementValue::Date(
            row.try_get::<Option<NaiveDate>, _>("value_date")
                .map_err(map_sqlx)?
                .ok_or_else(|| StoreError::Serialization("missing date value".to_owned()))?,
        ),
        FieldValueType::Category => MeasurementValue::Category(
            row.try_get::<Option<String>, _>("value_text")
                .map_err(map_sqlx)?
                .ok_or_else(|| StoreError::Serialization("missing category value".to_owned()))?,
        ),
    };
    Ok(Measurement {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: optional_uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        procedure_id: optional_uuid(row.try_get("procedure_id").map_err(map_sqlx)?)?,
        key: row.try_get("measurement_key").map_err(map_sqlx)?,
        label: row.try_get("label").map_err(map_sqlx)?,
        value_type,
        value,
        unit: row.try_get("unit").map_err(map_sqlx)?,
        measured_at: row.try_get("measured_at").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        signed_by: optional_uuid(row.try_get("signed_by").map_err(map_sqlx)?)?,
        signed_at: row.try_get("signed_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn sample_from_row(row: &SqliteRow) -> StoreResult<Sample> {
    Ok(Sample {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: optional_uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        collection_event_id: optional_uuid(row.try_get("collection_event_id").map_err(map_sqlx)?)?,
        sample_type: row.try_get("sample_type").map_err(map_sqlx)?,
        quantity: row.try_get("quantity").map_err(map_sqlx)?,
        unit: row.try_get("unit").map_err(map_sqlx)?,
        location: row.try_get("location").map_err(map_sqlx)?,
        collected_at: row.try_get("collected_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn job_from_row(row: &SqliteRow) -> StoreResult<Job> {
    let result_json = row
        .try_get::<Option<String>, _>("result_json")
        .map_err(map_sqlx)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| StoreError::Serialization(error.to_string()))?;
    let error_report_json = row
        .try_get::<Option<String>, _>("error_report_json")
        .map_err(map_sqlx)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| StoreError::Serialization(error.to_string()))?;

    Ok(Job {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        created_by: uuid(row.try_get("created_by").map_err(map_sqlx)?)?,
        kind: decode(row.try_get("kind").map_err(map_sqlx)?)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        idempotency_key: row.try_get("idempotency_key").map_err(map_sqlx)?,
        progress_current: row.try_get("progress_current").map_err(map_sqlx)?,
        progress_total: row.try_get("progress_total").map_err(map_sqlx)?,
        result: result_json,
        error_report: error_report_json,
        cancellation_requested: row
            .try_get::<i64, _>("cancellation_requested")
            .map_err(map_sqlx)?
            != 0,
        meta: meta(row)?,
    })
}

fn import_commit_from_row(row: &SqliteRow, replayed: bool) -> StoreResult<ImportCommitResult> {
    let count = |column: &'static str| -> StoreResult<usize> {
        usize::try_from(row.try_get::<i64, _>(column).map_err(map_sqlx)?).map_err(|_| {
            StoreError::Database(format!("import receipt contains an invalid {column}"))
        })
    };
    Ok(ImportCommitResult {
        commit_id: uuid(row.try_get("commit_id").map_err(map_sqlx)?)?,
        preview_hash: row.try_get("preview_hash").map_err(map_sqlx)?,
        counts: ImportEntityCounts {
            animals: count("animal_count")?,
            animal_events: count("animal_event_count")?,
            genotypes: count("genotype_count")?,
            pedigrees: count("pedigree_count")?,
            measurements: count("measurement_count")?,
        },
        committed_at: row.try_get("committed_at").map_err(map_sqlx)?,
        replayed,
    })
}

fn audit_from_row(row: &SqliteRow) -> StoreResult<AuditEntry> {
    let parse_json = |value: Option<String>| -> StoreResult<Option<Value>> {
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|e| StoreError::Serialization(e.to_string()))
            })
            .transpose()
    };
    Ok(AuditEntry {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        entity_type: decode(row.try_get("entity_type").map_err(map_sqlx)?)?,
        entity_id: uuid(row.try_get("entity_id").map_err(map_sqlx)?)?,
        action: decode(row.try_get("action").map_err(map_sqlx)?)?,
        actor: Actor {
            actor_type: decode(row.try_get("actor_type").map_err(map_sqlx)?)?,
            user_id: optional_uuid(row.try_get("actor_user_id").map_err(map_sqlx)?)?,
            display_name: row.try_get("actor_display_name").map_err(map_sqlx)?,
        },
        source: decode(row.try_get("source").map_err(map_sqlx)?)?,
        request_id: row.try_get("request_id").map_err(map_sqlx)?,
        reason: row.try_get("reason").map_err(map_sqlx)?,
        before: parse_json(row.try_get("before_json").map_err(map_sqlx)?)?,
        after: parse_json(row.try_get("after_json").map_err(map_sqlx)?)?,
        operation_code: row.try_get("operation_code").map_err(map_sqlx)?,
        operation_version: row.try_get("operation_version").map_err(map_sqlx)?,
        operation_params: serde_json::from_str(
            row.try_get("operation_params_json").map_err(map_sqlx)?,
        )
        .map_err(|error| StoreError::Serialization(error.to_string()))?,
        entity_name_snapshot: row.try_get("entity_name_snapshot").map_err(map_sqlx)?,
        entity_revision: row.try_get("entity_revision").map_err(map_sqlx)?,
        occurred_at: row.try_get("occurred_at").map_err(map_sqlx)?,
    })
}

fn provenance_from_row(row: &SqliteRow) -> StoreResult<Provenance> {
    Ok(Provenance {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        entity_type: decode(row.try_get("entity_type").map_err(map_sqlx)?)?,
        entity_id: uuid(row.try_get("entity_id").map_err(map_sqlx)?)?,
        source: decode(row.try_get("source").map_err(map_sqlx)?)?,
        actor_user_id: optional_uuid(row.try_get("actor_user_id").map_err(map_sqlx)?)?,
        import_job_id: optional_uuid(row.try_get("import_job_id").map_err(map_sqlx)?)?,
        import_commit_id: optional_uuid(row.try_get("import_commit_id").map_err(map_sqlx)?)?,
        tool_run_id: optional_uuid(row.try_get("tool_run_id").map_err(map_sqlx)?)?,
        provider: row.try_get("provider").map_err(map_sqlx)?,
        model: row.try_get("model").map_err(map_sqlx)?,
        confidence: row.try_get("confidence").map_err(map_sqlx)?,
        request_id: row.try_get("request_id").map_err(map_sqlx)?,
        recorded_at: row.try_get("recorded_at").map_err(map_sqlx)?,
    })
}

pub(crate) async fn insert_provenance_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    provenance: &Provenance,
) -> StoreResult<()> {
    provenance
        .validate()
        .map_err(|e| StoreError::Validation(e.to_owned()))?;
    if let Some(project_id) = provenance.project_id {
        let project_lab = required_lab_id(tx, "projects", project_id, "project").await?;
        require_same_uuid(project_lab, provenance.lab_id, "provenance project")?;
    }
    if let Some(user_id) = provenance.actor_user_id {
        let user_lab = required_lab_id(tx, "users", user_id, "user").await?;
        require_same_uuid(user_lab, provenance.lab_id, "provenance actor")?;
    }
    if let Some(job_id) = provenance.import_job_id {
        let job_lab = required_lab_id(tx, "jobs", job_id, "job").await?;
        require_same_uuid(job_lab, provenance.lab_id, "provenance import job")?;
    }
    if let Some(tool_id) = provenance.tool_run_id {
        let tool_lab = required_lab_id(tx, "ai_tool_runs", tool_id, "ai_tool_run").await?;
        require_same_uuid(tool_lab, provenance.lab_id, "provenance tool run")?;
    }
    sqlx::query("INSERT INTO provenance (id, lab_id, project_id, entity_type, entity_id, source, actor_user_id, import_job_id, import_commit_id, tool_run_id, provider, model, confidence, request_id, recorded_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(provenance.id.to_string()).bind(provenance.lab_id.to_string())
        .bind(provenance.project_id.map(|id| id.to_string())).bind(provenance.entity_type.as_str())
        .bind(provenance.entity_id.to_string()).bind(encode(&provenance.source)?)
        .bind(provenance.actor_user_id.map(|id| id.to_string()))
        .bind(provenance.import_job_id.map(|id| id.to_string()))
        .bind(provenance.import_commit_id.map(|id| id.to_string()))
        .bind(provenance.tool_run_id.map(|id| id.to_string()))
        .bind(&provenance.provider).bind(&provenance.model).bind(provenance.confidence)
        .bind(&provenance.request_id).bind(provenance.recorded_at)
        .execute(&mut **tx).await.map_err(map_sqlx)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_audit(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    lab_id: Uuid,
    project_id: Option<Uuid>,
    entity_type: EntityType,
    entity_id: Uuid,
    action: AuditAction,
    context: &AuditContext,
    before: Option<Value>,
    after: Option<Value>,
) -> StoreResult<()> {
    let operation = operation_metadata(entity_type, action, before.as_ref(), after.as_ref());
    sqlx::query(
        "INSERT INTO audit_entries (
            id, lab_id, project_id, entity_type, entity_id, action, actor_type,
            actor_user_id, actor_display_name, source, request_id, reason,
            before_json, after_json, operation_code, operation_version,
            operation_params_json, entity_name_snapshot, entity_revision, occurred_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(lab_id.to_string())
    .bind(project_id.map(|id| id.to_string()))
    .bind(entity_type.as_str())
    .bind(entity_id.to_string())
    .bind(encode(&action)?)
    .bind(encode(&context.actor.actor_type)?)
    .bind(context.actor.user_id.map(|id| id.to_string()))
    .bind(&context.actor.display_name)
    .bind(encode(&context.source)?)
    .bind(&context.request_id)
    .bind(&context.reason)
    .bind(before.map(|value| value.to_string()))
    .bind(after.map(|value| value.to_string()))
    .bind(operation.code)
    .bind(operation.version)
    .bind(operation.params.to_string())
    .bind(operation.entity_name_snapshot)
    .bind(operation.entity_revision)
    .bind(Utc::now())
    .execute(&mut **transaction)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

fn snapshot<T: Serialize>(value: &T) -> StoreResult<Value> {
    serde_json::to_value(value).map_err(|e| StoreError::Serialization(e.to_string()))
}

const LAB_COLUMNS: &str = "id, name, created_at, updated_at, deleted_at, revision";
const USER_COLUMNS: &str =
    "id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision";
const MEMBERSHIP_COLUMNS: &str = "id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision";
const PROJECT_ANIMAL_ASSIGNMENT_COLUMNS: &str = "id, lab_id, project_id, animal_id, assigned_by, reason, created_at, updated_at, deleted_at, revision";
const PROJECT_COLUMNS: &str =
    "id, lab_id, name, description, status, created_at, updated_at, deleted_at, revision";
const CAGE_COLUMNS: &str = "id, lab_id, section, display_id, location, kind, capacity, sort_order, created_at, updated_at, deleted_at, revision";
const ANIMAL_COLUMNS: &str = "id, lab_id, identifier_scope, display_id, legacy_id, species, strain, sex, birth_date, death_date, current_cage_id, current_status, created_at, updated_at, deleted_at, revision";
const EVENT_COLUMNS: &str = "id, lab_id, project_id, animal_id, event_type, payload_json, occurred_at, recorded_at, recorded_by, notes";
const EXPERIMENT_COLUMNS: &str = "id, lab_id, project_id, template_version_id, name, description, status, starts_at, ends_at, created_at, updated_at, deleted_at, revision";
const PARTICIPATION_COLUMNS: &str = "id, experiment_id, animal_id, cohort_id, status, enrolled_at, exited_at, genotype_snapshot_json, created_at, updated_at, deleted_at, revision";
const MEASUREMENT_COLUMNS: &str = "id, lab_id, project_id, experiment_id, animal_id, procedure_id, measurement_key, label, value_type, value_number, value_text, value_boolean, value_date, unit, measured_at, status, signed_by, signed_at, created_at, updated_at, deleted_at, revision";
const SAMPLE_COLUMNS: &str = "id, lab_id, project_id, experiment_id, animal_id, collection_event_id, sample_type, quantity, unit, location, collected_at, created_at, updated_at, deleted_at, revision";
const JOB_COLUMNS: &str = "id, lab_id, project_id, created_by, kind, status, idempotency_key, progress_current, progress_total, result_json, error_report_json, cancellation_requested, created_at, updated_at, deleted_at, revision";
const IMPORT_COMMIT_COLUMNS: &str = "commit_id, preview_hash, animal_count, animal_event_count, genotype_count, pedigree_count, measurement_count, committed_at";
const AUDIT_COLUMNS: &str = "id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, operation_code, operation_version, operation_params_json, entity_name_snapshot, entity_revision, occurred_at";
const PROVENANCE_COLUMNS: &str = "id, lab_id, project_id, entity_type, entity_id, source, actor_user_id, import_job_id, import_commit_id, tool_run_id, provider, model, confidence, request_id, recorded_at";
fn validate_named_update(
    entity: &'static str,
    value: &str,
    meta: &RecordMeta,
    expected_revision: i64,
) -> StoreResult<()> {
    if value.trim().is_empty() || value.chars().count() > 200 {
        return Err(StoreError::Validation(format!(
            "{entity} display name must contain 1-200 characters"
        )));
    }
    if meta.revision != expected_revision + 1 {
        return Err(StoreError::Validation(format!(
            "updated {entity} revision must equal expected revision plus one"
        )));
    }
    Ok(())
}

fn validate_job(job: &Job) -> StoreResult<()> {
    if job.idempotency_key.trim().is_empty()
        || job.idempotency_key.len() > 128
        || job.idempotency_key.chars().any(char::is_control)
    {
        return Err(StoreError::Validation(
            "job idempotency key must contain 1-128 non-control characters".to_owned(),
        ));
    }
    if job.progress_current < 0 || job.progress_total.is_some_and(|total| total < 0) {
        return Err(StoreError::Validation(
            "job progress values must be non-negative".to_owned(),
        ));
    }
    Ok(())
}

async fn required_lab_id(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &'static str,
    id: Uuid,
    entity: &'static str,
) -> StoreResult<Uuid> {
    let sql = format!("SELECT lab_id FROM {table} WHERE id = ? AND deleted_at IS NULL");
    let value = sqlx::query_scalar::<_, String>(&sql)
        .bind(id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity, id })?;
    uuid(&value)
}

async fn required_experiment_scope(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    experiment_id: Uuid,
) -> StoreResult<(Uuid, Uuid)> {
    let row = sqlx::query(
        "SELECT lab_id, project_id FROM experiments WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(experiment_id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "experiment",
        id: experiment_id,
    })?;
    Ok((
        uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
    ))
}

async fn require_experiment_participation(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    experiment_id: Uuid,
    animal_id: Uuid,
    relationship: &'static str,
) -> StoreResult<()> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM experiment_participations WHERE experiment_id = ? AND animal_id = ? AND deleted_at IS NULL",
    )
    .bind(experiment_id.to_string())
    .bind(animal_id.to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if exists == 0 {
        return Err(StoreError::Validation(format!(
            "{relationship} animal does not participate in its experiment"
        )));
    }
    Ok(())
}

fn require_same_uuid(actual: Uuid, expected: Uuid, relationship: &'static str) -> StoreResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StoreError::Validation(format!(
            "{relationship} belongs to a different lab, project, experiment or animal"
        )))
    }
}

async fn validate_animal_relationships(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    animal: &Animal,
) -> StoreResult<()> {
    if let IdentifierScope::Project { project_id } = animal.identifier_scope {
        let project_lab = required_lab_id(tx, "projects", project_id, "project").await?;
        require_same_uuid(project_lab, animal.lab_id, "animal identifier scope")?;
    }
    if let Some(cage_id) = animal.current_cage_id {
        let cage_lab = required_lab_id(tx, "cages", cage_id, "cage").await?;
        require_same_uuid(cage_lab, animal.lab_id, "animal cage")?;
        let (capacity, residents): (i64, i64) = sqlx::query_as(
            "SELECT c.capacity, (SELECT COUNT(*) FROM animals a WHERE a.current_cage_id = c.id AND a.deleted_at IS NULL) FROM cages c WHERE c.id = ? AND c.deleted_at IS NULL",
        )
        .bind(cage_id.to_string())
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        if residents >= capacity {
            return Err(StoreError::Conflict(
                "target cage is at capacity".to_owned(),
            ));
        }
    }
    Ok(())
}

async fn validate_experiment_relationships(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    experiment: &Experiment,
) -> StoreResult<()> {
    let project_lab = required_lab_id(tx, "projects", experiment.project_id, "project").await?;
    require_same_uuid(project_lab, experiment.lab_id, "experiment project")?;
    let Some(template_id) = experiment.template_version_id else {
        return Ok(());
    };
    let row = sqlx::query(
        "SELECT lab_id, status FROM experiment_template_versions WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(template_id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "experiment_template_version",
        id: template_id,
    })?;
    let template_lab = uuid(row.try_get("lab_id").map_err(map_sqlx)?)?;
    require_same_uuid(template_lab, experiment.lab_id, "experiment template")?;
    let status: TemplateStatus = decode(row.try_get("status").map_err(map_sqlx)?)?;
    if status != TemplateStatus::Published {
        return Err(StoreError::Validation(
            "experiment template version must be published".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_measurement_relationships(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    measurement: &Measurement,
    reject_duplicate: bool,
) -> StoreResult<()> {
    let project_lab = required_lab_id(tx, "projects", measurement.project_id, "project").await?;
    require_same_uuid(project_lab, measurement.lab_id, "measurement project")?;
    let animal_lab = required_lab_id(tx, "animals", measurement.animal_id, "animal").await?;
    require_same_uuid(animal_lab, measurement.lab_id, "measurement animal")?;
    if let Some(experiment_id) = measurement.experiment_id {
        let (experiment_lab, experiment_project) =
            required_experiment_scope(tx, experiment_id).await?;
        require_same_uuid(experiment_lab, measurement.lab_id, "measurement experiment")?;
        require_same_uuid(
            experiment_project,
            measurement.project_id,
            "measurement experiment",
        )?;
        require_experiment_participation(tx, experiment_id, measurement.animal_id, "measurement")
            .await?;
    }
    if let Some(procedure_id) = measurement.procedure_id {
        let row = sqlx::query(
            "SELECT experiment_id, animal_id FROM procedures WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(procedure_id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "procedure",
            id: procedure_id,
        })?;
        let procedure_experiment = uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?;
        if measurement.experiment_id != Some(procedure_experiment) {
            return Err(StoreError::Validation(
                "measurement procedure does not belong to its experiment".to_owned(),
            ));
        }
        let procedure_animal = optional_uuid(row.try_get("animal_id").map_err(map_sqlx)?)?;
        if procedure_animal.is_some_and(|animal_id| animal_id != measurement.animal_id) {
            return Err(StoreError::Validation(
                "measurement procedure belongs to a different animal".to_owned(),
            ));
        }
    }
    if reject_duplicate {
        let duplicate = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM measurements WHERE animal_id = ? AND measurement_key = ? AND measured_at = ? AND deleted_at IS NULL",
        )
        .bind(measurement.animal_id.to_string())
        .bind(&measurement.key)
        .bind(measurement.measured_at)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        if duplicate > 0 {
            return Err(StoreError::Conflict(
                "duplicate animal/key/time measurement".to_owned(),
            ));
        }
    }
    Ok(())
}

async fn insert_animal_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    animal: &Animal,
    audit: &AuditContext,
    action: AuditAction,
) -> StoreResult<()> {
    validate_animal_relationships(tx, animal).await?;
    sqlx::query("INSERT INTO animals (id, lab_id, identifier_scope, display_id, legacy_id, species, strain, sex, birth_date, death_date, current_cage_id, current_status, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(animal.id.to_string())
        .bind(animal.lab_id.to_string())
        .bind(animal.identifier_scope.storage_key())
        .bind(&animal.display_id)
        .bind(&animal.legacy_id)
        .bind(&animal.species)
        .bind(&animal.strain)
        .bind(encode(&animal.sex)?)
        .bind(animal.birth_date)
        .bind(animal.death_date)
        .bind(animal.current_cage_id.map(|id| id.to_string()))
        .bind(encode(&animal.current_status)?)
        .bind(animal.meta.created_at)
        .bind(animal.meta.updated_at)
        .bind(animal.meta.deleted_at)
        .bind(animal.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        animal.lab_id,
        None,
        EntityType::Animal,
        animal.id,
        action,
        audit,
        None,
        Some(snapshot(animal)?),
    )
    .await
}

async fn insert_animal_event_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    event: &AnimalEvent,
    audit: &AuditContext,
    action: AuditAction,
) -> StoreResult<()> {
    let animal_lab = required_lab_id(tx, "animals", event.animal_id, "animal").await?;
    require_same_uuid(animal_lab, event.lab_id, "animal event")?;
    if let Some(project_id) = event.project_id {
        let project_lab = required_lab_id(tx, "projects", project_id, "project").await?;
        require_same_uuid(project_lab, event.lab_id, "animal event project")?;
    }
    if let AnimalEventKind::Transferred { to_cage_id, .. } = &event.kind
        && let Some(cage_id) = to_cage_id
    {
        let cage_lab = required_lab_id(tx, "cages", *cage_id, "cage").await?;
        require_same_uuid(cage_lab, event.lab_id, "animal event cage")?;
    }
    sqlx::query("INSERT INTO animal_events (id, lab_id, project_id, animal_id, event_type, payload_json, occurred_at, recorded_at, recorded_by, notes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(event.id.to_string())
        .bind(event.lab_id.to_string())
        .bind(event.project_id.map(|id| id.to_string()))
        .bind(event.animal_id.to_string())
        .bind(event.kind.event_type())
        .bind(serde_json::to_string(&event.kind).map_err(|error| StoreError::Serialization(error.to_string()))?)
        .bind(event.occurred_at)
        .bind(event.recorded_at)
        .bind(event.recorded_by.map(|id| id.to_string()))
        .bind(&event.notes)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        event.lab_id,
        event.project_id,
        EntityType::AnimalEvent,
        event.id,
        action,
        audit,
        None,
        Some(snapshot(event)?),
    )
    .await
}

pub(crate) async fn append_derived_animal_event_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    event: &AnimalEvent,
    audit: &AuditContext,
) -> StoreResult<Animal> {
    let row = sqlx::query(&format!(
        "SELECT {ANIMAL_COLUMNS} FROM animals WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(event.animal_id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "animal",
        id: event.animal_id,
    })?;
    let mut animal = animal_from_row(&row)?;
    if animal.lab_id != event.lab_id {
        return Err(StoreError::Validation(
            "animal event belongs to a different lab".to_owned(),
        ));
    }
    let before_revision = animal.meta.revision;
    let before = snapshot(&animal)?;
    animal
        .apply_event(event)
        .map_err(|e| StoreError::Validation(e.to_string()))?;
    insert_animal_event_tx(tx, event, audit, AuditAction::Create).await?;
    let updated = sqlx::query("UPDATE animals SET birth_date = ?, death_date = ?, current_cage_id = ?, current_status = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
        .bind(animal.birth_date).bind(animal.death_date)
        .bind(animal.current_cage_id.map(|id| id.to_string())).bind(encode(&animal.current_status)?)
        .bind(animal.meta.updated_at).bind(animal.meta.revision).bind(animal.id.to_string())
        .bind(before_revision).execute(&mut **tx).await.map_err(map_sqlx)?;
    if updated.rows_affected() != 1 {
        return Err(StoreError::Conflict(
            "animal projection revision changed during domain write".to_owned(),
        ));
    }
    write_audit(
        tx,
        animal.lab_id,
        event.project_id,
        EntityType::Animal,
        animal.id,
        AuditAction::Update,
        audit,
        Some(before),
        Some(snapshot(&animal)?),
    )
    .await?;
    Ok(animal)
}

async fn count_open_participations_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    animal_id: Uuid,
) -> StoreResult<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM experiment_participations ep          JOIN experiments e ON e.id = ep.experiment_id          WHERE ep.animal_id = ? AND ep.status = ? AND ep.deleted_at IS NULL          AND e.status IN (?, ?) AND e.deleted_at IS NULL",
    )
    .bind(animal_id.to_string())
    .bind(encode(&ParticipationStatus::Enrolled)?)
    .bind(encode(&ExperimentStatus::Draft)?)
    .bind(encode(&ExperimentStatus::Active)?)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx)
}

struct ParticipationTransitionContext<'a> {
    target: ParticipationStatus,
    expected_revision: i64,
    occurred_at: chrono::DateTime<Utc>,
    lab_id: Uuid,
    project_id: Uuid,
    audit: &'a AuditContext,
}

async fn close_participation_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    mut participation: Participation,
    context: ParticipationTransitionContext<'_>,
) -> StoreResult<Participation> {
    if participation.meta.revision != context.expected_revision {
        return Err(StoreError::Conflict(
            "participation revision changed before the transition was applied".to_owned(),
        ));
    }
    let before = snapshot(&participation)?;
    participation
        .close(context.target, context.occurred_at)
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    let updated = sqlx::query(
        "UPDATE experiment_participations SET status = ?, exited_at = ?, updated_at = ?, revision = ?          WHERE id = ? AND revision = ? AND deleted_at IS NULL",
    )
    .bind(encode(&participation.status)?)
    .bind(participation.exited_at)
    .bind(participation.meta.updated_at)
    .bind(participation.meta.revision)
    .bind(participation.id.to_string())
    .bind(context.expected_revision)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if updated.rows_affected() != 1 {
        return Err(StoreError::Conflict(
            "participation revision changed before the transition was applied".to_owned(),
        ));
    }
    write_audit(
        tx,
        context.lab_id,
        Some(context.project_id),
        EntityType::Participation,
        participation.id,
        AuditAction::Update,
        context.audit,
        Some(before),
        Some(snapshot(&participation)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        context.lab_id,
        Some(context.project_id),
        EntityType::Participation,
        participation.id,
        context.audit,
        context.occurred_at,
    );
    insert_provenance_tx(tx, &provenance).await?;

    let mut event = AnimalEvent::new(
        context.lab_id,
        participation.animal_id,
        AnimalEventKind::ExperimentParticipationEnded {
            participation_id: participation.id,
            status: participation.status,
        },
        context.occurred_at,
        context.occurred_at,
    );
    event.project_id = Some(context.project_id);
    event.recorded_by = context.audit.actor.user_id;
    event.notes = Some(
        match participation.status {
            ParticipationStatus::Completed => "实验参与已完成",
            ParticipationStatus::Withdrawn => "动物已退出实验",
            ParticipationStatus::Enrolled => unreachable!("close rejects enrolled target"),
        }
        .to_owned(),
    );
    let projected = append_derived_animal_event_tx(tx, &event, context.audit).await?;

    if projected.current_status == AnimalStatus::InExperiment
        && count_open_participations_tx(tx, participation.animal_id).await? == 0
    {
        let mut status_event = AnimalEvent::new(
            context.lab_id,
            participation.animal_id,
            AnimalEventKind::StatusChanged {
                from: AnimalStatus::InExperiment,
                to: AnimalStatus::Alive,
            },
            context.occurred_at,
            context.occurred_at,
        );
        status_event.project_id = Some(context.project_id);
        status_event.recorded_by = context.audit.actor.user_id;
        status_event.notes = Some("动物已无进行中的实验参与".to_owned());
        append_derived_animal_event_tx(tx, &status_event, context.audit).await?;
    }

    Ok(participation)
}

async fn insert_measurement_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    measurement: &Measurement,
    audit: &AuditContext,
    action: AuditAction,
) -> StoreResult<()> {
    measurement
        .validate_record()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    validate_measurement_relationships(tx, measurement, true).await?;
    let (value_number, value_text, value_boolean, value_date) = match &measurement.value {
        MeasurementValue::Number(value) => (Some(*value), None, None, None),
        MeasurementValue::Text(value) | MeasurementValue::Category(value) => {
            (None, Some(value.clone()), None, None)
        }
        MeasurementValue::Boolean(value) => (None, None, Some(i64::from(*value)), None),
        MeasurementValue::Date(value) => (None, None, None, Some(*value)),
    };
    sqlx::query("INSERT INTO measurements (id, lab_id, project_id, experiment_id, animal_id, procedure_id, measurement_key, label, value_type, value_number, value_text, value_boolean, value_date, unit, measured_at, status, signed_by, signed_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(measurement.id.to_string())
        .bind(measurement.lab_id.to_string())
        .bind(measurement.project_id.to_string())
        .bind(measurement.experiment_id.map(|id| id.to_string()))
        .bind(measurement.animal_id.to_string())
        .bind(measurement.procedure_id.map(|id| id.to_string()))
        .bind(&measurement.key)
        .bind(&measurement.label)
        .bind(encode(&measurement.value_type)?)
        .bind(value_number)
        .bind(value_text)
        .bind(value_boolean)
        .bind(value_date)
        .bind(&measurement.unit)
        .bind(measurement.measured_at)
        .bind(encode(&measurement.status)?)
        .bind(measurement.signed_by.map(|id| id.to_string()))
        .bind(measurement.signed_at)
        .bind(measurement.meta.created_at)
        .bind(measurement.meta.updated_at)
        .bind(measurement.meta.deleted_at)
        .bind(measurement.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        measurement.lab_id,
        Some(measurement.project_id),
        EntityType::Measurement,
        measurement.id,
        action,
        audit,
        None,
        Some(snapshot(measurement)?),
    )
    .await
}

async fn insert_genotype_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    genotype: &Genotype,
    project_id: Option<Uuid>,
    audit: &AuditContext,
    action: AuditAction,
) -> StoreResult<()> {
    let animal_lab = required_lab_id(tx, "animals", genotype.animal_id, "animal").await?;
    let locus_lab = required_lab_id(tx, "gene_loci", genotype.locus_id, "gene_locus").await?;
    require_same_uuid(locus_lab, animal_lab, "genotype locus")?;
    for allele_id in [genotype.allele_1_id, genotype.allele_2_id]
        .into_iter()
        .flatten()
    {
        let allele_locus = sqlx::query_scalar::<_, String>(
            "SELECT locus_id FROM alleles WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(allele_id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "allele",
            id: allele_id,
        })?;
        require_same_uuid(uuid(&allele_locus)?, genotype.locus_id, "genotype allele")?;
    }
    sqlx::query("INSERT INTO genotypes (id, animal_id, locus_id, allele_1_id, allele_2_id, assessed_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(genotype.id.to_string())
        .bind(genotype.animal_id.to_string())
        .bind(genotype.locus_id.to_string())
        .bind(genotype.allele_1_id.map(|id| id.to_string()))
        .bind(genotype.allele_2_id.map(|id| id.to_string()))
        .bind(genotype.assessed_at)
        .bind(genotype.meta.created_at)
        .bind(genotype.meta.updated_at)
        .bind(genotype.meta.deleted_at)
        .bind(genotype.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        animal_lab,
        project_id,
        EntityType::Genotype,
        genotype.id,
        action,
        audit,
        None,
        Some(snapshot(genotype)?),
    )
    .await
}

async fn insert_pedigree_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    pedigree: &Pedigree,
    audit: &AuditContext,
    action: AuditAction,
) -> StoreResult<()> {
    if pedigree.animal_id == pedigree.parent_id {
        return Err(StoreError::Validation(
            "an animal cannot be its own parent".to_owned(),
        ));
    }
    let animal_lab = required_lab_id(tx, "animals", pedigree.animal_id, "animal").await?;
    let parent_lab = required_lab_id(tx, "animals", pedigree.parent_id, "parent").await?;
    require_same_uuid(parent_lab, animal_lab, "pedigree parent")?;
    sqlx::query("INSERT INTO pedigrees (id, animal_id, parent_id, parent_type, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(pedigree.id.to_string())
        .bind(pedigree.animal_id.to_string())
        .bind(pedigree.parent_id.to_string())
        .bind(encode(&pedigree.parent_type)?)
        .bind(pedigree.meta.created_at)
        .bind(pedigree.meta.updated_at)
        .bind(pedigree.meta.deleted_at)
        .bind(pedigree.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        animal_lab,
        None,
        EntityType::Pedigree,
        pedigree.id,
        action,
        audit,
        None,
        Some(snapshot(pedigree)?),
    )
    .await
}
#[async_trait::async_trait]

impl MuriArcStore for SqliteStore {
    async fn migrate(&self) -> StoreResult<()> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|error| StoreError::Database(error.to_string()))
    }

    async fn health_check(&self) -> StoreResult<()> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn create_lab(&self, lab: &Lab, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(lab.id.to_string())
            .bind(&lab.name)
            .bind(lab.meta.created_at)
            .bind(lab.meta.updated_at)
            .bind(lab.meta.deleted_at)
            .bind(lab.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            lab.id,
            None,
            EntityType::Lab,
            lab.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(lab)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_lab(&self, id: Uuid) -> StoreResult<Lab> {
        let row = sqlx::query(&format!(
            "SELECT {LAB_COLUMNS} FROM labs WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "lab", id })?;
        lab_from_row(&row)
    }

    async fn update_lab(
        &self,
        lab: &Lab,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_named_update("lab", &lab.name, &lab.meta, expected_revision)?;
        let before = self.get_lab(lab.id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "lab revision changed before the update was applied".to_owned(),
            ));
        }
        if before.id != lab.id
            || before.meta.created_at != lab.meta.created_at
            || before.meta.deleted_at != lab.meta.deleted_at
        {
            return Err(StoreError::Validation(
                "immutable lab fields cannot be changed".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE labs SET name = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(&lab.name)
        .bind(lab.meta.updated_at)
        .bind(lab.meta.revision)
        .bind(lab.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "lab revision changed before the update was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            lab.id,
            None,
            EntityType::Lab,
            lab.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(lab)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn create_user(&self, user: &User, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(user.id.to_string())
            .bind(user.lab_id.to_string())
            .bind(&user.email)
            .bind(&user.display_name)
            .bind(encode(&user.status)?)
            .bind(user.meta.created_at)
            .bind(user.meta.updated_at)
            .bind(user.meta.deleted_at)
            .bind(user.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            user.lab_id,
            None,
            EntityType::User,
            user.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(user)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_user(&self, id: Uuid) -> StoreResult<User> {
        let row = sqlx::query(&format!(
            "SELECT {USER_COLUMNS} FROM users WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "user", id })?;
        user_from_row(&row)
    }

    async fn list_users(&self, filter: &UserFilter) -> StoreResult<Vec<User>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {USER_COLUMNS} FROM users WHERE lab_id = "
        ));
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(status) = filter.status {
            query.push(" AND status = ").push_bind(encode(&status)?);
        }
        query.push(" ORDER BY lower(email), id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(user_from_row).collect()
    }

    async fn update_user(
        &self,
        user: &User,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_named_update("user", &user.display_name, &user.meta, expected_revision)?;
        let before = self.get_user(user.id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "user revision changed before the update was applied".to_owned(),
            ));
        }
        if before.id != user.id
            || before.lab_id != user.lab_id
            || before.email != user.email
            || before.meta.created_at != user.meta.created_at
            || before.meta.deleted_at != user.meta.deleted_at
        {
            return Err(StoreError::Validation(
                "immutable user fields cannot be changed".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE users SET display_name = ?, status = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(&user.display_name)
        .bind(encode(&user.status)?)
        .bind(user.meta.updated_at)
        .bind(user.meta.revision)
        .bind(user.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "user revision changed before the update was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            user.lab_id,
            None,
            EntityType::User,
            user.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(user)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn create_membership(
        &self,
        membership: &Membership,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        membership
            .validate_scope()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let lab_role = membership.lab_role.map(|role| encode(&role)).transpose()?;
        let project_role = membership
            .project_role
            .map(|role| encode(&role))
            .transpose()?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO memberships (id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(membership.id.to_string())
            .bind(membership.lab_id.to_string())
            .bind(membership.project_id.map(|id| id.to_string()))
            .bind(membership.user_id.to_string())
            .bind(lab_role)
            .bind(project_role)
            .bind(membership.meta.created_at)
            .bind(membership.meta.updated_at)
            .bind(membership.meta.deleted_at)
            .bind(membership.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            membership.lab_id,
            membership.project_id,
            EntityType::Membership,
            membership.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(membership)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_membership(&self, id: Uuid) -> StoreResult<Membership> {
        let row = sqlx::query(&format!(
            "SELECT {MEMBERSHIP_COLUMNS} FROM memberships WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "membership",
            id,
        })?;
        membership_from_row(&row)
    }

    async fn list_memberships(&self, filter: &MembershipFilter) -> StoreResult<Vec<Membership>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {MEMBERSHIP_COLUMNS} FROM memberships WHERE lab_id = "
        ));
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(user_id) = filter.user_id {
            query.push(" AND user_id = ").push_bind(user_id.to_string());
        }
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND project_id = ")
                .push_bind(project_id.to_string());
        }
        query.push(" ORDER BY project_id, created_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(membership_from_row).collect()
    }

    async fn update_membership(
        &self,
        membership: &Membership,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        membership
            .validate_scope()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if membership.meta.revision != expected_revision + 1 {
            return Err(StoreError::Validation(
                "updated membership revision must equal expected revision plus one".to_owned(),
            ));
        }
        let before = self.get_membership(membership.id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "membership revision changed before the update was applied".to_owned(),
            ));
        }
        if before.id != membership.id
            || before.lab_id != membership.lab_id
            || before.project_id != membership.project_id
            || before.user_id != membership.user_id
            || before.meta.created_at != membership.meta.created_at
            || before.meta.deleted_at != membership.meta.deleted_at
        {
            return Err(StoreError::Validation(
                "immutable membership fields cannot be changed".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE memberships SET lab_role = ?, project_role = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(membership.lab_role.map(|role| encode(&role)).transpose()?)
        .bind(
            membership
                .project_role
                .map(|role| encode(&role))
                .transpose()?,
        )
        .bind(membership.meta.updated_at)
        .bind(membership.meta.revision)
        .bind(membership.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "membership revision changed before the update was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            membership.lab_id,
            membership.project_id,
            EntityType::Membership,
            membership.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(membership)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn soft_delete_membership(
        &self,
        id: Uuid,
        expected_revision: i64,
        deleted_at: chrono::DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Membership> {
        let mut membership = self.get_membership(id).await?;
        if membership.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "membership revision changed before the delete was applied".to_owned(),
            ));
        }
        let before = membership.clone();
        membership.soft_delete(deleted_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE memberships SET updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(membership.meta.updated_at)
        .bind(membership.meta.deleted_at)
        .bind(membership.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "membership revision changed before the delete was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            membership.lab_id,
            membership.project_id,
            EntityType::Membership,
            membership.id,
            AuditAction::SoftDelete,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&membership)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(membership)
    }

    async fn create_project(&self, project: &Project, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO projects (id, lab_id, name, description, status, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(project.id.to_string())
            .bind(project.lab_id.to_string())
            .bind(&project.name)
            .bind(&project.description)
            .bind(encode(&project.status)?)
            .bind(project.meta.created_at)
            .bind(project.meta.updated_at)
            .bind(project.meta.deleted_at)
            .bind(project.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            project.lab_id,
            Some(project.id),
            EntityType::Project,
            project.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(project)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_project(&self, id: Uuid) -> StoreResult<Project> {
        let row = sqlx::query(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "project",
            id,
        })?;
        project_from_row(&row)
    }

    async fn list_projects(&self, lab_id: Uuid) -> StoreResult<Vec<Project>> {
        let rows = sqlx::query(&format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE lab_id = ? AND deleted_at IS NULL ORDER BY name, id"))
            .bind(lab_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(project_from_row).collect()
    }

    async fn assign_animals_to_project(
        &self,
        assignments: &[ProjectAnimalAssignment],
        audit: &AuditContext,
    ) -> StoreResult<Vec<ProjectAnimalAssignment>> {
        if assignments.is_empty() || assignments.len() > 100 {
            return Err(StoreError::Validation(
                "project animal assignment batch must contain 1-100 items".to_owned(),
            ));
        }
        let first = &assignments[0];
        if assignments.iter().any(|assignment| {
            assignment.lab_id != first.lab_id
                || assignment.project_id != first.project_id
                || assignment.meta.deleted_at.is_some()
                || assignment.meta.revision != 1
        }) {
            return Err(StoreError::Validation(
                "project animal assignments must be one new lab/project batch".to_owned(),
            ));
        }
        if audit.actor.actor_type == ActorType::Human
            && assignments
                .iter()
                .any(|assignment| assignment.assigned_by != audit.actor.user_id)
        {
            return Err(StoreError::Validation(
                "assignment actor must match the human audit actor".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let project_lab = required_lab_id(&mut tx, "projects", first.project_id, "project").await?;
        require_same_uuid(project_lab, first.lab_id, "project animal assignment")?;
        for assignment in assignments {
            let animal_lab =
                required_lab_id(&mut tx, "animals", assignment.animal_id, "animal").await?;
            require_same_uuid(animal_lab, first.lab_id, "project animal assignment")?;
        }
        for assignment in assignments {
            sqlx::query("INSERT INTO project_animal_assignments (id, lab_id, project_id, animal_id, assigned_by, reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(assignment.id.to_string())
                .bind(assignment.lab_id.to_string())
                .bind(assignment.project_id.to_string())
                .bind(assignment.animal_id.to_string())
                .bind(assignment.assigned_by.map(|id| id.to_string()))
                .bind(&assignment.reason)
                .bind(assignment.meta.created_at)
                .bind(assignment.meta.updated_at)
                .bind(assignment.meta.deleted_at)
                .bind(assignment.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                assignment.lab_id,
                Some(assignment.project_id),
                EntityType::ProjectAnimalAssignment,
                assignment.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(assignment)?),
            )
            .await?;
            insert_provenance_tx(
                &mut tx,
                &Provenance::from_audit(
                    assignment.lab_id,
                    Some(assignment.project_id),
                    EntityType::ProjectAnimalAssignment,
                    assignment.id,
                    audit,
                    assignment.meta.created_at,
                ),
            )
            .await?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(assignments.to_vec())
    }

    async fn list_project_animal_assignments(
        &self,
        filter: &ProjectAnimalAssignmentFilter,
    ) -> StoreResult<Vec<ProjectAnimalAssignment>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {PROJECT_ANIMAL_ASSIGNMENT_COLUMNS} FROM project_animal_assignments WHERE lab_id = "
        ));
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND project_id = ")
                .push_bind(project_id.to_string());
        }
        if let Some(animal_id) = filter.animal_id {
            query
                .push(" AND animal_id = ")
                .push_bind(animal_id.to_string());
        }
        query.push(" ORDER BY project_id, created_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter()
            .map(project_animal_assignment_from_row)
            .collect()
    }

    async fn remove_animals_from_project(
        &self,
        removals: &[ProjectAnimalAssignmentRemoval],
        deleted_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Vec<ProjectAnimalAssignment>> {
        if removals.is_empty() || removals.len() > 100 {
            return Err(StoreError::Validation(
                "project animal removal batch must contain 1-100 items".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let mut assignments = Vec::with_capacity(removals.len());
        for removal in removals {
            let row = sqlx::query(&format!(
                "SELECT {PROJECT_ANIMAL_ASSIGNMENT_COLUMNS} FROM project_animal_assignments WHERE id = ? AND deleted_at IS NULL"
            ))
            .bind(removal.assignment_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "project_animal_assignment",
                id: removal.assignment_id,
            })?;
            let assignment = project_animal_assignment_from_row(&row)?;
            if assignment.meta.revision != removal.expected_revision {
                return Err(StoreError::Conflict(
                    "project animal assignment revision changed before removal".to_owned(),
                ));
            }
            assignments.push(assignment);
        }
        let scope = (assignments[0].lab_id, assignments[0].project_id);
        if assignments
            .iter()
            .any(|assignment| (assignment.lab_id, assignment.project_id) != scope)
        {
            return Err(StoreError::Validation(
                "project animal removals must belong to one lab/project".to_owned(),
            ));
        }
        for (assignment, removal) in assignments.iter_mut().zip(removals) {
            let before = assignment.clone();
            assignment.soft_delete(deleted_at);
            let result = sqlx::query("UPDATE project_animal_assignments SET updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
                .bind(assignment.meta.updated_at)
                .bind(assignment.meta.deleted_at)
                .bind(assignment.meta.revision)
                .bind(assignment.id.to_string())
                .bind(removal.expected_revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            if result.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "project animal assignment revision changed before removal".to_owned(),
                ));
            }
            write_audit(
                &mut tx,
                assignment.lab_id,
                Some(assignment.project_id),
                EntityType::ProjectAnimalAssignment,
                assignment.id,
                AuditAction::SoftDelete,
                audit,
                Some(snapshot(&before)?),
                Some(snapshot(assignment)?),
            )
            .await?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(assignments)
    }

    async fn create_cage(&self, cage: &Cage, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO cages (id, lab_id, section, display_id, location, kind, capacity, sort_order, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(cage.id.to_string())
            .bind(cage.lab_id.to_string())
            .bind(&cage.section)
            .bind(&cage.display_id)
            .bind(&cage.location)
            .bind(encode(&cage.kind)?)
            .bind(cage.capacity)
            .bind(cage.sort_order)
            .bind(cage.meta.created_at)
            .bind(cage.meta.updated_at)
            .bind(cage.meta.deleted_at)
            .bind(cage.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            cage.lab_id,
            None,
            EntityType::Cage,
            cage.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(cage)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_cage(&self, id: Uuid) -> StoreResult<Cage> {
        let row = sqlx::query(&format!(
            "SELECT {CAGE_COLUMNS} FROM cages WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "cage", id })?;
        cage_from_row(&row)
    }

    async fn list_cages(&self, lab_id: Uuid) -> StoreResult<Vec<Cage>> {
        let rows = sqlx::query(&format!("SELECT {CAGE_COLUMNS} FROM cages WHERE lab_id = ? AND deleted_at IS NULL ORDER BY sort_order, section, display_id"))
            .bind(lab_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(cage_from_row).collect()
    }

    async fn list_cages_for_project(
        &self,
        lab_id: Uuid,
        project_id: Uuid,
    ) -> StoreResult<Vec<Cage>> {
        let rows = sqlx::query(
            "SELECT DISTINCT c.* FROM cages c JOIN animals a ON a.current_cage_id = c.id AND a.deleted_at IS NULL JOIN project_animal_assignments paa ON paa.animal_id = a.id AND paa.deleted_at IS NULL WHERE c.lab_id = ? AND c.deleted_at IS NULL AND paa.project_id = ? ORDER BY c.sort_order, c.section, c.display_id"
        )
        .bind(lab_id.to_string())
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(cage_from_row).collect()
    }

    async fn create_animal(&self, animal: &Animal, audit: &AuditContext) -> StoreResult<()> {
        self.create_animal_with_genotyping_records(animal, &[], audit)
            .await
    }

    async fn create_animal_with_genotyping_records(
        &self,
        animal: &Animal,
        genotyping_records: &[GenotypingRecord],
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        insert_animal_tx(&mut tx, animal, audit, AuditAction::Create).await?;
        let mut events = vec![AnimalEvent::new(
            animal.lab_id,
            animal.id,
            AnimalEventKind::Registered,
            animal.meta.created_at,
            animal.meta.created_at,
        )];
        if let Some(birth_date) = animal.birth_date {
            events.push(AnimalEvent::new(
                animal.lab_id,
                animal.id,
                AnimalEventKind::Born { birth_date },
                animal.meta.created_at,
                animal.meta.created_at,
            ));
        }
        if let Some(cage_id) = animal.current_cage_id {
            events.push(AnimalEvent::new(
                animal.lab_id,
                animal.id,
                AnimalEventKind::Transferred {
                    from_cage_id: None,
                    to_cage_id: Some(cage_id),
                },
                animal.meta.created_at,
                animal.meta.created_at,
            ));
        }
        for event in &mut events {
            event.recorded_by = audit.actor.user_id;
            insert_animal_event_tx(&mut tx, event, audit, AuditAction::Create).await?;
        }
        let provenance = Provenance::from_audit(
            animal.lab_id,
            None,
            EntityType::Animal,
            animal.id,
            audit,
            animal.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut definition_ids = BTreeSet::new();
        for record in genotyping_records {
            record
                .validate()
                .map_err(|error| StoreError::Validation(error.to_string()))?;
            if record.lab_id != animal.lab_id
                || record.animal_id != animal.id
                || record.supersedes_record_id.is_some()
                || record.is_voided()
                || record.meta.deleted_at.is_some()
                || !definition_ids.insert(record.genotype_definition_id)
            {
                return Err(StoreError::Validation(
                    "initial genotyping record has incompatible identity or lifecycle fields"
                        .to_owned(),
                ));
            }
            let definition_lab = sqlx::query_scalar::<_, String>(
                "SELECT lab_id FROM genotype_definitions WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(record.genotype_definition_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "genotype_definition",
                id: record.genotype_definition_id,
            })?;
            require_same_uuid(
                uuid(&definition_lab)?,
                animal.lab_id,
                "initial genotyping record definition",
            )?;
            if let Some(project_id) = record.project_id {
                let project_lab =
                    required_lab_id(&mut tx, "projects", project_id, "project").await?;
                require_same_uuid(
                    project_lab,
                    animal.lab_id,
                    "initial genotyping record project",
                )?;
            }
            sqlx::query("INSERT INTO genotyping_records (id, lab_id, project_id, animal_id, genotype_definition_id, state, assessed_at, method, notes, supersedes_record_id, voided_at, void_reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(record.id.to_string())
                .bind(record.lab_id.to_string())
                .bind(record.project_id.map(|id| id.to_string()))
                .bind(record.animal_id.to_string())
                .bind(record.genotype_definition_id.to_string())
                .bind(encode(&record.state)?)
                .bind(record.assessed_at)
                .bind(&record.method)
                .bind(&record.notes)
                .bind(record.supersedes_record_id.map(|id| id.to_string()))
                .bind(record.voided_at)
                .bind(&record.void_reason)
                .bind(record.meta.created_at)
                .bind(record.meta.updated_at)
                .bind(record.meta.deleted_at)
                .bind(record.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                record.lab_id,
                record.project_id,
                EntityType::GenotypingRecord,
                record.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(record)?),
            )
            .await?;
            let provenance = Provenance::from_audit(
                record.lab_id,
                record.project_id,
                EntityType::GenotypingRecord,
                record.id,
                audit,
                record.meta.created_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
            let mut event = AnimalEvent::new(
                record.lab_id,
                record.animal_id,
                AnimalEventKind::GenotypingRecorded {
                    record_id: record.id,
                    genotype_definition_id: record.genotype_definition_id,
                    state: record.state,
                },
                record.assessed_at.unwrap_or(record.meta.created_at),
                record.meta.created_at,
            );
            event.project_id = record.project_id;
            event.recorded_by = audit.actor.user_id;
            append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        }
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_animal(&self, id: Uuid) -> StoreResult<Animal> {
        let row = sqlx::query(&format!(
            "SELECT {ANIMAL_COLUMNS} FROM animals WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "animal",
            id,
        })?;
        animal_from_row(&row)
    }

    async fn list_animals(&self, filter: &AnimalFilter) -> StoreResult<Vec<Animal>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {ANIMAL_COLUMNS} FROM animals a WHERE a.lab_id = "
        ));
        query.push_bind(filter.lab_id.to_string());
        query.push(" AND a.deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query.push(" AND EXISTS (SELECT 1 FROM project_animal_assignments paa WHERE paa.animal_id = a.id AND paa.deleted_at IS NULL AND paa.project_id = ");
            query.push_bind(project_id.to_string());
            query.push(")");
        }
        if let Some(cage_id) = filter.cage_id {
            query.push(" AND a.current_cage_id = ");
            query.push_bind(cage_id.to_string());
        }
        if let Some(status) = filter.status {
            query.push(" AND a.current_status = ");
            query.push_bind(encode(&status)?);
        }
        if let Some(term) = filter
            .query
            .as_deref()
            .map(str::trim)
            .filter(|term| !term.is_empty())
        {
            let pattern = format!("%{term}%");
            query.push(" AND (a.display_id LIKE ");
            query.push_bind(pattern.clone());
            query.push(" OR a.legacy_id LIKE ");
            query.push_bind(pattern.clone());
            query.push(" OR a.strain LIKE ");
            query.push_bind(pattern);
            query.push(")");
        }
        query.push(" ORDER BY a.display_id, a.id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(animal_from_row).collect()
    }

    async fn list_animal_overviews(
        &self,
        filter: &AnimalFilter,
        offset: u32,
        limit: u32,
    ) -> StoreResult<Vec<AnimalOverview>> {
        const MAX_PAGE: u32 = 1_000;
        if limit == 0 || limit > MAX_PAGE {
            return Err(StoreError::Validation(format!(
                "animal overview limit must be between 1 and {MAX_PAGE}"
            )));
        }

        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {ANIMAL_COLUMNS} FROM animals a WHERE a.lab_id = "
        ));
        query.push_bind(filter.lab_id.to_string());
        query.push(" AND a.deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query.push(" AND EXISTS (SELECT 1 FROM project_animal_assignments paa WHERE paa.animal_id = a.id AND paa.deleted_at IS NULL AND paa.project_id = ");
            query.push_bind(project_id.to_string());
            query.push(")");
        }
        if let Some(cage_id) = filter.cage_id {
            query.push(" AND a.current_cage_id = ");
            query.push_bind(cage_id.to_string());
        }
        if let Some(status) = filter.status {
            query.push(" AND a.current_status = ");
            query.push_bind(encode(&status)?);
        }
        if let Some(term) = filter
            .query
            .as_deref()
            .map(str::trim)
            .filter(|term| !term.is_empty())
        {
            let pattern = format!("%{term}%");
            query.push(" AND (a.display_id LIKE ");
            query.push_bind(pattern.clone());
            query.push(" OR a.legacy_id LIKE ");
            query.push_bind(pattern.clone());
            query.push(" OR a.strain LIKE ");
            query.push_bind(pattern);
            query.push(")");
        }
        query
            .push(" ORDER BY a.display_id, a.id LIMIT ")
            .push_bind(i64::from(limit))
            .push(" OFFSET ")
            .push_bind(i64::from(offset));
        let animal_rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let animals = animal_rows
            .iter()
            .map(animal_from_row)
            .collect::<StoreResult<Vec<_>>>()?;
        if animals.is_empty() {
            return Ok(Vec::new());
        }

        let ids = animals.iter().map(|animal| animal.id).collect::<Vec<_>>();
        let mut overviews = animals
            .into_iter()
            .map(|animal| {
                (
                    animal.id,
                    AnimalOverview {
                        animal,
                        genotype_labels: Vec::new(),
                        projects: Vec::new(),
                        latest_weight: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut genotype_query = QueryBuilder::<Sqlite>::new(
            "SELECT current_records.animal_id, current_records.state, definitions.name AS definition_name FROM (SELECT records.animal_id, records.genotype_definition_id, records.state, ROW_NUMBER() OVER (PARTITION BY records.animal_id, records.genotype_definition_id ORDER BY records.created_at DESC, records.id DESC) AS current_rank FROM genotyping_records records WHERE records.deleted_at IS NULL AND records.voided_at IS NULL AND records.animal_id IN (",
        );
        {
            let mut separated = genotype_query.separated(", ");
            for id in &ids {
                separated.push_bind(id.to_string());
            }
            separated.push_unseparated(") ) current_records JOIN genotype_definitions definitions ON definitions.id = current_records.genotype_definition_id WHERE current_records.current_rank = 1 ORDER BY current_records.animal_id, definitions.name, current_records.genotype_definition_id");
        }
        for row in genotype_query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        {
            let animal_id = uuid(row.try_get("animal_id").map_err(map_sqlx)?)?;
            let definition_name: String = row.try_get("definition_name").map_err(map_sqlx)?;
            let state_value: String = row.try_get("state").map_err(map_sqlx)?;
            let state: GenotypingState = decode(&state_value)?;
            let state_label = match state {
                GenotypingState::Unknown => "unknown",
                GenotypingState::Expected => "expected",
                GenotypingState::Confirmed => "confirmed",
                GenotypingState::Rejected => "rejected",
            };
            let label = format!("{definition_name} [{state_label}]");
            if let Some(overview) = overviews.get_mut(&animal_id) {
                overview.genotype_labels.push(label);
            }
        }

        let mut project_query = QueryBuilder::<Sqlite>::new(
            "SELECT paa.animal_id, p.id AS project_id, p.name AS project_name FROM project_animal_assignments paa JOIN projects p ON p.id = paa.project_id AND p.deleted_at IS NULL WHERE paa.deleted_at IS NULL AND p.lab_id = ",
        );
        project_query.push_bind(filter.lab_id.to_string());
        if let Some(project_id) = filter.project_id {
            project_query
                .push(" AND paa.project_id = ")
                .push_bind(project_id.to_string());
        }
        project_query.push(" AND paa.animal_id IN (");
        {
            let mut separated = project_query.separated(", ");
            for id in &ids {
                separated.push_bind(id.to_string());
            }
            separated.push_unseparated(") ORDER BY paa.animal_id, p.name, p.id");
        }
        for row in project_query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        {
            let animal_id = uuid(row.try_get("animal_id").map_err(map_sqlx)?)?;
            if let Some(overview) = overviews.get_mut(&animal_id) {
                overview.projects.push(AnimalProjectRef {
                    id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
                    name: row.try_get("project_name").map_err(map_sqlx)?,
                });
            }
        }

        let mut weight_query = QueryBuilder::<Sqlite>::new(
            "SELECT animal_id, value_number, unit, measured_at FROM (SELECT m.animal_id, m.value_number, m.unit, m.measured_at, ROW_NUMBER() OVER (PARTITION BY m.animal_id ORDER BY m.measured_at DESC, m.id DESC) AS row_number FROM measurements m WHERE m.deleted_at IS NULL AND m.value_number IS NOT NULL AND lower(m.measurement_key) IN ('weight', 'body_weight')",
        );
        if let Some(project_id) = filter.project_id {
            weight_query
                .push(" AND m.project_id = ")
                .push_bind(project_id.to_string());
        }
        weight_query.push(" AND m.animal_id IN (");
        {
            let mut separated = weight_query.separated(", ");
            for id in &ids {
                separated.push_bind(id.to_string());
            }
            separated.push_unseparated(") ) ranked WHERE row_number = 1 ORDER BY animal_id");
        }
        for row in weight_query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        {
            let animal_id = uuid(row.try_get("animal_id").map_err(map_sqlx)?)?;
            if let Some(overview) = overviews.get_mut(&animal_id) {
                overview.latest_weight = Some(LatestAnimalWeight {
                    value: row.try_get("value_number").map_err(map_sqlx)?,
                    unit: row.try_get("unit").map_err(map_sqlx)?,
                    measured_at: row.try_get("measured_at").map_err(map_sqlx)?,
                });
            }
        }

        Ok(ids
            .into_iter()
            .filter_map(|id| overviews.remove(&id))
            .collect())
    }

    async fn list_animals_by_ids(
        &self,
        lab_id: Uuid,
        project_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> StoreResult<Vec<Animal>> {
        const MAX_IDS: usize = 512;
        if ids.len() > MAX_IDS {
            return Err(StoreError::Validation(format!(
                "animal id batch must not exceed {MAX_IDS}"
            )));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {ANIMAL_COLUMNS} FROM animals WHERE lab_id = "
        ));
        query
            .push_bind(lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(project_id) = project_id {
            query.push(" AND EXISTS (SELECT 1 FROM project_animal_assignments paa WHERE paa.animal_id = animals.id AND paa.deleted_at IS NULL AND paa.project_id = ");
            query.push_bind(project_id.to_string());
            query.push(")");
        }
        query.push(" AND id IN (");
        {
            let mut separated = query.separated(", ");
            for id in ids {
                separated.push_bind(id.to_string());
            }
            separated.push_unseparated(") ORDER BY display_id, id");
        }
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(animal_from_row).collect()
    }

    async fn append_animal_event(
        &self,
        event: &AnimalEvent,
        audit: &AuditContext,
    ) -> StoreResult<Animal> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {ANIMAL_COLUMNS} FROM animals WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(event.animal_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "animal",
            id: event.animal_id,
        })?;
        let mut animal = animal_from_row(&row)?;
        if event.lab_id != animal.lab_id {
            return Err(StoreError::Validation(
                "animal event belongs to a different lab".to_owned(),
            ));
        }
        let before = snapshot(&animal)?;
        animal
            .apply_event(event)
            .map_err(|error| StoreError::Validation(error.to_string()))?;

        insert_animal_event_tx(&mut tx, event, audit, AuditAction::Create).await?;

        sqlx::query("UPDATE animals SET birth_date = ?, death_date = ?, current_cage_id = ?, current_status = ?, updated_at = ?, revision = ? WHERE id = ?")
            .bind(animal.birth_date)
            .bind(animal.death_date)
            .bind(animal.current_cage_id.map(|id| id.to_string()))
            .bind(encode(&animal.current_status)?)
            .bind(animal.meta.updated_at)
            .bind(animal.meta.revision)
            .bind(animal.id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        write_audit(
            &mut tx,
            animal.lab_id,
            event.project_id,
            EntityType::Animal,
            animal.id,
            AuditAction::Update,
            audit,
            Some(before),
            Some(snapshot(&animal)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(animal)
    }

    async fn list_animal_events(&self, animal_id: Uuid) -> StoreResult<Vec<AnimalEvent>> {
        let rows = sqlx::query(&format!("SELECT {EVENT_COLUMNS} FROM animal_events WHERE animal_id = ? ORDER BY occurred_at, recorded_at, id"))
            .bind(animal_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(event_from_row).collect()
    }

    async fn transfer_animals(
        &self,
        transfer: &AnimalTransfer,
        audit: &AuditContext,
    ) -> StoreResult<Vec<Animal>> {
        transfer
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut animal_ids = transfer.animal_ids.clone();
        animal_ids.sort_unstable();

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let target_row = sqlx::query(&format!(
            "SELECT {CAGE_COLUMNS} FROM cages WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(transfer.target_cage_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "cage",
            id: transfer.target_cage_id,
        })?;
        let target = cage_from_row(&target_row)?;
        if target.lab_id != transfer.lab_id {
            return Err(StoreError::Validation(
                "target cage belongs to a different lab".to_owned(),
            ));
        }

        let mut animals = Vec::with_capacity(animal_ids.len());
        for animal_id in animal_ids {
            let row = sqlx::query(&format!(
                "SELECT {ANIMAL_COLUMNS} FROM animals WHERE id = ? AND deleted_at IS NULL"
            ))
            .bind(animal_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "animal",
                id: animal_id,
            })?;
            let animal = animal_from_row(&row)?;
            if animal.lab_id != transfer.lab_id {
                return Err(StoreError::Validation(
                    "animal belongs to a different lab".to_owned(),
                ));
            }
            animals.push(animal);
        }

        let resident_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM animals WHERE current_cage_id = ? AND deleted_at IS NULL",
        )
        .bind(target.id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let incoming_count = animals
            .iter()
            .filter(|animal| animal.current_cage_id != Some(target.id))
            .count() as i64;
        if resident_count + incoming_count > i64::from(target.capacity) {
            return Err(StoreError::Conflict(format!(
                "target cage {} capacity exceeded ({}/{})",
                target.display_id,
                resident_count + incoming_count,
                target.capacity
            )));
        }

        for animal in &mut animals {
            if animal.current_cage_id == Some(target.id) {
                continue;
            }
            let before = snapshot(animal)?;
            let mut event = AnimalEvent::new(
                transfer.lab_id,
                animal.id,
                AnimalEventKind::Transferred {
                    from_cage_id: animal.current_cage_id,
                    to_cage_id: Some(target.id),
                },
                transfer.occurred_at,
                transfer.recorded_at,
            );
            event.recorded_by = transfer.recorded_by;
            event.notes.clone_from(&transfer.notes);
            animal
                .apply_event(&event)
                .map_err(|error| StoreError::Validation(error.to_string()))?;

            sqlx::query("INSERT INTO animal_events (id, lab_id, project_id, animal_id, event_type, payload_json, occurred_at, recorded_at, recorded_by, notes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(event.id.to_string())
                .bind(event.lab_id.to_string())
                .bind(Option::<String>::None)
                .bind(event.animal_id.to_string())
                .bind(event.kind.event_type())
                .bind(serde_json::to_string(&event.kind).map_err(|e| StoreError::Serialization(e.to_string()))?)
                .bind(event.occurred_at)
                .bind(event.recorded_at)
                .bind(event.recorded_by.map(|id| id.to_string()))
                .bind(&event.notes)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            sqlx::query(
                "UPDATE animals SET current_cage_id = ?, updated_at = ?, revision = ? WHERE id = ?",
            )
            .bind(animal.current_cage_id.map(|id| id.to_string()))
            .bind(animal.meta.updated_at)
            .bind(animal.meta.revision)
            .bind(animal.id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                event.lab_id,
                None,
                EntityType::AnimalEvent,
                event.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(&event)?),
            )
            .await?;
            write_audit(
                &mut tx,
                animal.lab_id,
                None,
                EntityType::Animal,
                animal.id,
                AuditAction::Update,
                audit,
                Some(before),
                Some(snapshot(animal)?),
            )
            .await?;
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(animals)
    }

    async fn create_experiment(
        &self,
        experiment: &Experiment,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_experiment_relationships(&mut tx, experiment).await?;
        sqlx::query("INSERT INTO experiments (id, lab_id, project_id, template_version_id, name, description, status, starts_at, ends_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(experiment.id.to_string())
            .bind(experiment.lab_id.to_string())
            .bind(experiment.project_id.to_string())
            .bind(experiment.template_version_id.map(|id| id.to_string()))
            .bind(&experiment.name)
            .bind(&experiment.description)
            .bind(encode(&experiment.status)?)
            .bind(experiment.starts_at)
            .bind(experiment.ends_at)
            .bind(experiment.meta.created_at)
            .bind(experiment.meta.updated_at)
            .bind(experiment.meta.deleted_at)
            .bind(experiment.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            experiment.lab_id,
            Some(experiment.project_id),
            EntityType::Experiment,
            experiment.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(experiment)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_experiment(&self, id: Uuid) -> StoreResult<Experiment> {
        let row = sqlx::query(&format!(
            "SELECT {EXPERIMENT_COLUMNS} FROM experiments WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "experiment",
            id,
        })?;
        experiment_from_row(&row)
    }

    async fn list_experiments(&self, filter: &ExperimentFilter) -> StoreResult<Vec<Experiment>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {EXPERIMENT_COLUMNS} FROM experiments WHERE project_id = "
        ));
        query.push_bind(filter.project_id.to_string());
        query.push(" AND deleted_at IS NULL");
        if let Some(status) = filter.status {
            query.push(" AND status = ");
            query.push_bind(encode(&status)?);
        }
        query.push(" ORDER BY created_at DESC, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(experiment_from_row).collect()
    }

    async fn transition_experiment(
        &self,
        id: Uuid,
        target: ExperimentStatus,
        expected_revision: i64,
        occurred_at: chrono::DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Experiment> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {EXPERIMENT_COLUMNS} FROM experiments WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "experiment",
            id,
        })?;
        let mut experiment = experiment_from_row(&row)?;
        if experiment.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "experiment revision changed before the transition was applied".to_owned(),
            ));
        }
        let before = snapshot(&experiment)?;
        experiment
            .close(target, occurred_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let updated = sqlx::query(
            "UPDATE experiments SET status = ?, ends_at = ?, updated_at = ?, revision = ?              WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(encode(&experiment.status)?)
        .bind(experiment.ends_at)
        .bind(experiment.meta.updated_at)
        .bind(experiment.meta.revision)
        .bind(experiment.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "experiment revision changed before the transition was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            experiment.lab_id,
            Some(experiment.project_id),
            EntityType::Experiment,
            experiment.id,
            AuditAction::Update,
            audit,
            Some(before),
            Some(snapshot(&experiment)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            experiment.lab_id,
            Some(experiment.project_id),
            EntityType::Experiment,
            experiment.id,
            audit,
            occurred_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;

        let participation_target = match target {
            ExperimentStatus::Completed => ParticipationStatus::Completed,
            ExperimentStatus::Cancelled => ParticipationStatus::Withdrawn,
            _ => unreachable!("domain close rejects non-terminal target"),
        };
        let rows = sqlx::query(&format!(
            "SELECT {PARTICIPATION_COLUMNS} FROM experiment_participations              WHERE experiment_id = ? AND status = ? AND deleted_at IS NULL ORDER BY id"
        ))
        .bind(experiment.id.to_string())
        .bind(encode(&ParticipationStatus::Enrolled)?)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        for row in rows {
            let participation = participation_from_row(&row)?;
            let revision = participation.meta.revision;
            close_participation_tx(
                &mut tx,
                participation,
                ParticipationTransitionContext {
                    target: participation_target,
                    expected_revision: revision,
                    occurred_at,
                    lab_id: experiment.lab_id,
                    project_id: experiment.project_id,
                    audit,
                },
            )
            .await?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(experiment)
    }

    async fn create_participation(
        &self,
        participation: &Participation,
        audit: &AuditContext,
    ) -> StoreResult<Participation> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let (experiment_lab, experiment_project) =
            required_experiment_scope(&mut tx, participation.experiment_id).await?;
        let animal_lab =
            required_lab_id(&mut tx, "animals", participation.animal_id, "animal").await?;
        require_same_uuid(animal_lab, experiment_lab, "participation animal")?;
        let assigned = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_animal_assignments WHERE project_id = ? AND animal_id = ? AND deleted_at IS NULL",
        )
        .bind(experiment_project.to_string())
        .bind(participation.animal_id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if assigned == 0 {
            let assignment = ProjectAnimalAssignment::new(
                experiment_lab,
                experiment_project,
                participation.animal_id,
                audit.actor.user_id,
                Some("Assigned during local experiment enrollment".to_owned()),
                participation.enrolled_at,
            );
            sqlx::query("INSERT INTO project_animal_assignments (id, lab_id, project_id, animal_id, assigned_by, reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(assignment.id.to_string())
                .bind(assignment.lab_id.to_string())
                .bind(assignment.project_id.to_string())
                .bind(assignment.animal_id.to_string())
                .bind(assignment.assigned_by.map(|id| id.to_string()))
                .bind(&assignment.reason)
                .bind(assignment.meta.created_at)
                .bind(assignment.meta.updated_at)
                .bind(assignment.meta.deleted_at)
                .bind(assignment.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                assignment.lab_id,
                Some(assignment.project_id),
                EntityType::ProjectAnimalAssignment,
                assignment.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(&assignment)?),
            )
            .await?;
            insert_provenance_tx(
                &mut tx,
                &Provenance::from_audit(
                    assignment.lab_id,
                    Some(assignment.project_id),
                    EntityType::ProjectAnimalAssignment,
                    assignment.id,
                    audit,
                    assignment.meta.created_at,
                ),
            )
            .await?;
        }
        if let Some(cohort_id) = participation.cohort_id {
            let cohort_experiment = sqlx::query_scalar::<_, String>(
                "SELECT experiment_id FROM cohorts WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(cohort_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "cohort",
                id: cohort_id,
            })?;
            require_same_uuid(
                uuid(&cohort_experiment)?,
                participation.experiment_id,
                "participation cohort",
            )?;
        }
        // Acquire SQLite's writer lock before capturing the snapshot so a
        // concurrent genotyping write is ordered strictly before or after enrollment.
        sqlx::query("UPDATE animals SET updated_at = updated_at WHERE id = ?")
            .bind(participation.animal_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let genotype_rows = sqlx::query(&format!(
            "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE animal_id = ? AND deleted_at IS NULL AND voided_at IS NULL ORDER BY created_at, id"
        ))
        .bind(participation.animal_id.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let mut latest = BTreeMap::<Uuid, GenotypingRecord>::new();
        for row in &genotype_rows {
            let record = genotyping_record_from_row(row)?;
            latest.insert(record.genotype_definition_id, record);
        }
        let mut participation = participation.clone();
        participation.genotype_snapshot = latest
            .into_values()
            .map(|record| GenotypeSnapshotEntry {
                genotyping_record_id: record.id,
                genotype_definition_id: record.genotype_definition_id,
                state: record.state,
                assessed_at: record.assessed_at,
            })
            .collect();
        sqlx::query("INSERT INTO experiment_participations (id, experiment_id, animal_id, cohort_id, status, enrolled_at, exited_at, genotype_snapshot_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(participation.id.to_string())
            .bind(participation.experiment_id.to_string())
            .bind(participation.animal_id.to_string())
            .bind(participation.cohort_id.map(|id| id.to_string()))
            .bind(encode(&participation.status)?)
            .bind(participation.enrolled_at)
            .bind(participation.exited_at)
            .bind(serde_json::to_string(&participation.genotype_snapshot).map_err(|error| StoreError::Serialization(error.to_string()))?)
            .bind(participation.meta.created_at)
            .bind(participation.meta.updated_at)
            .bind(participation.meta.deleted_at)
            .bind(participation.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            experiment_lab,
            Some(experiment_project),
            EntityType::Participation,
            participation.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(&participation)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            experiment_lab,
            Some(experiment_project),
            EntityType::Participation,
            participation.id,
            audit,
            participation.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            experiment_lab,
            participation.animal_id,
            AnimalEventKind::ExperimentEnrolled {
                participation_id: participation.id,
            },
            participation.enrolled_at,
            participation.meta.created_at,
        );
        event.project_id = Some(experiment_project);
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(participation)
    }

    async fn get_participation(&self, id: Uuid) -> StoreResult<Participation> {
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPATION_COLUMNS} FROM experiment_participations              WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "participation",
            id,
        })?;
        participation_from_row(&row)
    }

    async fn list_participations(
        &self,
        filter: &ParticipationFilter,
    ) -> StoreResult<Vec<Participation>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {PARTICIPATION_COLUMNS} FROM experiment_participations WHERE deleted_at IS NULL AND experiment_id IN (SELECT id FROM experiments WHERE project_id = "
        ));
        query
            .push_bind(filter.project_id.to_string())
            .push(" AND deleted_at IS NULL)");
        if let Some(experiment_id) = filter.experiment_id {
            query
                .push(" AND experiment_id = ")
                .push_bind(experiment_id.to_string());
        }
        if let Some(animal_id) = filter.animal_id {
            query
                .push(" AND animal_id = ")
                .push_bind(animal_id.to_string());
        }
        if let Some(cohort_id) = filter.cohort_id {
            query
                .push(" AND cohort_id = ")
                .push_bind(cohort_id.to_string());
        }
        query.push(" ORDER BY enrolled_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(participation_from_row).collect()
    }

    async fn transition_participation(
        &self,
        id: Uuid,
        target: ParticipationStatus,
        expected_revision: i64,
        occurred_at: chrono::DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Participation> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPATION_COLUMNS} FROM experiment_participations              WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "participation",
            id,
        })?;
        let participation = participation_from_row(&row)?;
        let (lab_id, project_id) =
            required_experiment_scope(&mut tx, participation.experiment_id).await?;
        let participation = close_participation_tx(
            &mut tx,
            participation,
            ParticipationTransitionContext {
                target,
                expected_revision,
                occurred_at,
                lab_id,
                project_id,
                audit,
            },
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(participation)
    }

    async fn create_measurement(
        &self,
        measurement: &Measurement,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        insert_measurement_tx(&mut tx, measurement, audit, AuditAction::Create).await?;
        let provenance = Provenance::from_audit(
            measurement.lab_id,
            Some(measurement.project_id),
            EntityType::Measurement,
            measurement.id,
            audit,
            measurement.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            measurement.lab_id,
            measurement.animal_id,
            AnimalEventKind::MeasurementRecorded {
                measurement_id: measurement.id,
            },
            measurement.measured_at,
            measurement.meta.created_at,
        );
        event.project_id = Some(measurement.project_id);
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_measurement(&self, id: Uuid) -> StoreResult<Measurement> {
        let row = sqlx::query(&format!(
            "SELECT {MEASUREMENT_COLUMNS} FROM measurements WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "measurement",
            id,
        })?;
        measurement_from_row(&row)
    }

    async fn list_measurements(&self, filter: &MeasurementFilter) -> StoreResult<Vec<Measurement>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {MEASUREMENT_COLUMNS} FROM measurements WHERE project_id = "
        ));
        query.push_bind(filter.project_id.to_string());
        query.push(" AND deleted_at IS NULL");
        if let Some(experiment_id) = filter.experiment_id {
            query.push(" AND experiment_id = ");
            query.push_bind(experiment_id.to_string());
        }
        if let Some(animal_id) = filter.animal_id {
            query.push(" AND animal_id = ");
            query.push_bind(animal_id.to_string());
        }
        query.push(" ORDER BY measured_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(measurement_from_row).collect()
    }

    async fn update_measurement(
        &self,
        measurement: &Measurement,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        measurement
            .validate_record()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if measurement.meta.revision != expected_revision + 1 {
            return Err(StoreError::Validation(
                "updated measurement revision must equal expected revision plus one".to_owned(),
            ));
        }

        let before = self.get_measurement(measurement.id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "measurement revision changed before the update was applied".to_owned(),
            ));
        }
        if before.lab_id != measurement.lab_id
            || before.project_id != measurement.project_id
            || before.experiment_id != measurement.experiment_id
            || before.animal_id != measurement.animal_id
            || before.procedure_id != measurement.procedure_id
            || before.key != measurement.key
            || before.label != measurement.label
            || before.value_type != measurement.value_type
            || before.value != measurement.value
            || before.unit != measurement.unit
            || before.measured_at != measurement.measured_at
            || before.meta.created_at != measurement.meta.created_at
            || before.meta.deleted_at != measurement.meta.deleted_at
        {
            return Err(StoreError::Validation(
                "signed measurement content and identity fields cannot be changed".to_owned(),
            ));
        }
        if before.status != RecordStatus::Draft || measurement.status != RecordStatus::Signed {
            return Err(StoreError::Validation(
                "measurement update must sign an existing draft".to_owned(),
            ));
        }
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != measurement.signed_by
        {
            return Err(StoreError::Validation(
                "measurement signer must match the human audit actor".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE measurements SET status = ?, signed_by = ?, signed_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND status = ? AND deleted_at IS NULL",
        )
        .bind(encode(&measurement.status)?)
        .bind(measurement.signed_by.map(|id| id.to_string()))
        .bind(measurement.signed_at)
        .bind(measurement.meta.updated_at)
        .bind(measurement.meta.revision)
        .bind(measurement.id.to_string())
        .bind(expected_revision)
        .bind(encode(&RecordStatus::Draft)?)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "measurement revision changed before the signature was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            measurement.lab_id,
            Some(measurement.project_id),
            EntityType::Measurement,
            measurement.id,
            AuditAction::Sign,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(measurement)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn create_sample(&self, sample: &Sample, audit: &AuditContext) -> StoreResult<()> {
        if let Some(quantity) = sample.quantity
            && (!quantity.is_finite() || quantity < 0.0)
        {
            return Err(StoreError::Validation(
                "sample quantity must be finite and non-negative".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let project_lab =
            required_lab_id(&mut tx, "projects", sample.project_id, "project").await?;
        require_same_uuid(project_lab, sample.lab_id, "sample project")?;
        let animal_lab = required_lab_id(&mut tx, "animals", sample.animal_id, "animal").await?;
        require_same_uuid(animal_lab, sample.lab_id, "sample animal")?;
        if let Some(experiment_id) = sample.experiment_id {
            let (experiment_lab, experiment_project) =
                required_experiment_scope(&mut tx, experiment_id).await?;
            require_same_uuid(experiment_lab, sample.lab_id, "sample experiment")?;
            require_same_uuid(experiment_project, sample.project_id, "sample experiment")?;
            require_experiment_participation(&mut tx, experiment_id, sample.animal_id, "sample")
                .await?;
        }
        if let Some(event_id) = sample.collection_event_id {
            let row =
                sqlx::query("SELECT lab_id, project_id, animal_id FROM animal_events WHERE id = ?")
                    .bind(event_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(map_sqlx)?
                    .ok_or(StoreError::NotFound {
                        entity: "animal_event",
                        id: event_id,
                    })?;
            let event_lab = uuid(row.try_get("lab_id").map_err(map_sqlx)?)?;
            let event_animal = uuid(row.try_get("animal_id").map_err(map_sqlx)?)?;
            let event_project = optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?;
            require_same_uuid(event_lab, sample.lab_id, "sample collection event")?;
            require_same_uuid(event_animal, sample.animal_id, "sample collection event")?;
            if let Some(event_project) = event_project {
                require_same_uuid(event_project, sample.project_id, "sample collection event")?;
            }
        }
        sqlx::query("INSERT INTO samples (id, lab_id, project_id, experiment_id, animal_id, collection_event_id, sample_type, quantity, unit, location, collected_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(sample.id.to_string())
            .bind(sample.lab_id.to_string())
            .bind(sample.project_id.to_string())
            .bind(sample.experiment_id.map(|id| id.to_string()))
            .bind(sample.animal_id.to_string())
            .bind(sample.collection_event_id.map(|id| id.to_string()))
            .bind(&sample.sample_type)
            .bind(sample.quantity)
            .bind(&sample.unit)
            .bind(&sample.location)
            .bind(sample.collected_at)
            .bind(sample.meta.created_at)
            .bind(sample.meta.updated_at)
            .bind(sample.meta.deleted_at)
            .bind(sample.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            sample.lab_id,
            Some(sample.project_id),
            EntityType::Sample,
            sample.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(sample)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            sample.lab_id,
            Some(sample.project_id),
            EntityType::Sample,
            sample.id,
            audit,
            sample.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            sample.lab_id,
            sample.animal_id,
            AnimalEventKind::SampleCollected {
                sample_id: sample.id,
                terminal: false,
            },
            sample.collected_at,
            sample.meta.created_at,
        );
        event.project_id = Some(sample.project_id);
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_sample(&self, id: Uuid) -> StoreResult<Sample> {
        let row = sqlx::query(&format!(
            "SELECT {SAMPLE_COLUMNS} FROM samples WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "sample",
            id,
        })?;
        sample_from_row(&row)
    }

    async fn list_samples(&self, filter: &SampleFilter) -> StoreResult<Vec<Sample>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {SAMPLE_COLUMNS} FROM samples WHERE project_id = "
        ));
        query.push_bind(filter.project_id.to_string());
        query.push(" AND deleted_at IS NULL");
        if let Some(experiment_id) = filter.experiment_id {
            query.push(" AND experiment_id = ");
            query.push_bind(experiment_id.to_string());
        }
        if let Some(animal_id) = filter.animal_id {
            query.push(" AND animal_id = ");
            query.push_bind(animal_id.to_string());
        }
        query.push(" ORDER BY collected_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(sample_from_row).collect()
    }

    async fn create_job(&self, job: &Job, audit: &AuditContext) -> StoreResult<()> {
        validate_job(job)?;
        let result_json = job.result.as_ref().map(Value::to_string);
        let error_report_json = job.error_report.as_ref().map(Value::to_string);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query(
            "INSERT INTO jobs (id, lab_id, project_id, created_by, kind, status, idempotency_key, progress_current, progress_total, result_json, error_report_json, cancellation_requested, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(job.id.to_string())
        .bind(job.lab_id.to_string())
        .bind(job.project_id.map(|id| id.to_string()))
        .bind(job.created_by.to_string())
        .bind(encode(&job.kind)?)
        .bind(encode(&job.status)?)
        .bind(&job.idempotency_key)
        .bind(job.progress_current)
        .bind(job.progress_total)
        .bind(result_json)
        .bind(error_report_json)
        .bind(i64::from(job.cancellation_requested))
        .bind(job.meta.created_at)
        .bind(job.meta.updated_at)
        .bind(job.meta.deleted_at)
        .bind(job.meta.revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            job.lab_id,
            job.project_id,
            EntityType::Job,
            job.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(job)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_job(&self, id: Uuid) -> StoreResult<Job> {
        let row = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "job", id })?;
        job_from_row(&row)
    }

    async fn find_job_by_idempotency(
        &self,
        lab_id: Uuid,
        created_by: Uuid,
        idempotency_key: &str,
    ) -> StoreResult<Option<Job>> {
        let row = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs WHERE lab_id = ? AND created_by = ? AND idempotency_key = ? AND deleted_at IS NULL"
        ))
        .bind(lab_id.to_string())
        .bind(created_by.to_string())
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(job_from_row).transpose()
    }

    async fn list_jobs(&self, filter: &JobFilter) -> StoreResult<Vec<Job>> {
        let mut query =
            QueryBuilder::<Sqlite>::new(format!("SELECT {JOB_COLUMNS} FROM jobs WHERE lab_id = "));
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND project_id = ")
                .push_bind(project_id.to_string());
        }
        if let Some(created_by) = filter.created_by {
            query
                .push(" AND created_by = ")
                .push_bind(created_by.to_string());
        }
        query.push(" ORDER BY created_at DESC, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(job_from_row).collect()
    }

    async fn update_job(
        &self,
        job: &Job,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_job(job)?;
        if job.meta.revision != expected_revision + 1 {
            return Err(StoreError::Validation(
                "updated job revision must equal expected revision plus one".to_owned(),
            ));
        }
        let before = self.get_job(job.id).await?;
        if before.lab_id != job.lab_id
            || before.project_id != job.project_id
            || before.created_by != job.created_by
            || before.kind != job.kind
            || before.idempotency_key != job.idempotency_key
        {
            return Err(StoreError::Validation(
                "immutable job identity fields cannot be changed".to_owned(),
            ));
        }

        let result_json = job.result.as_ref().map(Value::to_string);
        let error_report_json = job.error_report.as_ref().map(Value::to_string);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE jobs SET status = ?, progress_current = ?, progress_total = ?, result_json = ?, error_report_json = ?, cancellation_requested = ?, updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(encode(&job.status)?)
        .bind(job.progress_current)
        .bind(job.progress_total)
        .bind(result_json)
        .bind(error_report_json)
        .bind(i64::from(job.cancellation_requested))
        .bind(job.meta.updated_at)
        .bind(job.meta.deleted_at)
        .bind(job.meta.revision)
        .bind(job.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "job revision changed before the update was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            job.lab_id,
            job.project_id,
            EntityType::Job,
            job.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(job)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn commit_import(
        &self,
        plan: &ImportPlan,
        options: ImportCommitOptions,
        audit: &AuditContext,
    ) -> StoreResult<ImportCommitResult> {
        plan.validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if options.cancellation_requested {
            return Err(StoreError::Conflict(
                "import confirmation was cancelled before it started".to_owned(),
            ));
        }
        if audit.actor.actor_type != ActorType::Human || audit.actor.user_id.is_none() {
            return Err(StoreError::Validation(
                "import confirmation requires an authenticated human actor".to_owned(),
            ));
        }
        let actor_id = audit.actor.user_id.expect("human actor checked above");
        if plan
            .animal_events
            .iter()
            .any(|event| event.recorded_by != Some(actor_id))
        {
            return Err(StoreError::Validation(
                "imported animal events must be recorded by the confirming human actor".to_owned(),
            ));
        }

        let preview_hash = plan.preview_hash.to_ascii_lowercase();
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let by_key = sqlx::query(&format!(
            "SELECT {IMPORT_COMMIT_COLUMNS} FROM import_commits WHERE lab_id = ? AND idempotency_key = ?"
        ))
        .bind(plan.lab_id.to_string())
        .bind(&plan.idempotency_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = by_key {
            let receipt = import_commit_from_row(&row, true)?;
            if receipt.preview_hash.eq_ignore_ascii_case(&preview_hash) {
                tx.rollback().await.map_err(map_sqlx)?;
                return Ok(receipt);
            }
            return Err(StoreError::Conflict(
                "idempotency key was already used for a different import preview".to_owned(),
            ));
        }

        let by_hash = sqlx::query_scalar::<_, String>(
            "SELECT idempotency_key FROM import_commits WHERE lab_id = ? AND lower(preview_hash) = ?",
        )
        .bind(plan.lab_id.to_string())
        .bind(&preview_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if by_hash.is_some() {
            return Err(StoreError::Conflict(
                "import preview was already committed with a different idempotency key".to_owned(),
            ));
        }

        let counts = plan.entity_counts();
        let committed_at = Utc::now();
        let claim = sqlx::query(
            "INSERT OR IGNORE INTO import_commits (commit_id, lab_id, idempotency_key, preview_hash, animal_count, animal_event_count, genotype_count, pedigree_count, measurement_count, committed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(plan.commit_id.to_string())
        .bind(plan.lab_id.to_string())
        .bind(&plan.idempotency_key)
        .bind(&preview_hash)
        .bind(i64::try_from(counts.animals).map_err(|_| StoreError::Validation("animal import count exceeds database limits".to_owned()))?)
        .bind(i64::try_from(counts.animal_events).map_err(|_| StoreError::Validation("animal event import count exceeds database limits".to_owned()))?)
        .bind(i64::try_from(counts.genotypes).map_err(|_| StoreError::Validation("genotype import count exceeds database limits".to_owned()))?)
        .bind(i64::try_from(counts.pedigrees).map_err(|_| StoreError::Validation("pedigree import count exceeds database limits".to_owned()))?)
        .bind(i64::try_from(counts.measurements).map_err(|_| StoreError::Validation("measurement import count exceeds database limits".to_owned()))?)
        .bind(committed_at)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if claim.rows_affected() == 0 {
            let by_key = sqlx::query(&format!(
                "SELECT {IMPORT_COMMIT_COLUMNS} FROM import_commits WHERE lab_id = ? AND idempotency_key = ?"
            ))
            .bind(plan.lab_id.to_string())
            .bind(&plan.idempotency_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if let Some(row) = by_key {
                let receipt = import_commit_from_row(&row, true)?;
                if receipt.preview_hash.eq_ignore_ascii_case(&preview_hash) {
                    tx.rollback().await.map_err(map_sqlx)?;
                    return Ok(receipt);
                }
                return Err(StoreError::Conflict(
                    "idempotency key was already used for a different import preview".to_owned(),
                ));
            }
            let matching_hash = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM import_commits WHERE lab_id = ? AND lower(preview_hash) = ?",
            )
            .bind(plan.lab_id.to_string())
            .bind(&preview_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if matching_hash > 0 {
                return Err(StoreError::Conflict(
                    "import preview was already committed with a different idempotency key"
                        .to_owned(),
                ));
            }
            return Err(StoreError::Conflict(
                "import receipt identifier is already in use".to_owned(),
            ));
        }
        let import_source = if audit.source == WriteSource::Migration {
            ProvenanceSource::Migration
        } else {
            ProvenanceSource::Import
        };
        for animal in &plan.animals {
            insert_animal_tx(&mut tx, animal, audit, AuditAction::Import).await?;
            let mut provenance = Provenance::from_audit(
                plan.lab_id,
                None,
                EntityType::Animal,
                animal.id,
                audit,
                committed_at,
            );
            provenance.source = import_source;
            provenance.import_job_id = options.job_id;
            provenance.import_commit_id = Some(plan.commit_id);
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        for event in &plan.animal_events {
            insert_animal_event_tx(&mut tx, event, audit, AuditAction::Import).await?;
            let mut provenance = Provenance::from_audit(
                plan.lab_id,
                event.project_id,
                EntityType::AnimalEvent,
                event.id,
                audit,
                committed_at,
            );
            provenance.source = import_source;
            provenance.import_job_id = options.job_id;
            provenance.import_commit_id = Some(plan.commit_id);
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        for record in &plan.genotyping_records {
            let animal_lab =
                required_lab_id(&mut tx, "animals", record.animal_id, "animal").await?;
            require_same_uuid(animal_lab, plan.lab_id, "imported genotyping record animal")?;
            let definition_lab = sqlx::query_scalar::<_, String>(
                "SELECT lab_id FROM genotype_definitions WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(record.genotype_definition_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "genotype_definition",
                id: record.genotype_definition_id,
            })?;
            require_same_uuid(
                uuid(&definition_lab)?,
                plan.lab_id,
                "imported genotyping record definition",
            )?;
            if let Some(project_id) = record.project_id {
                let project_lab =
                    required_lab_id(&mut tx, "projects", project_id, "project").await?;
                require_same_uuid(
                    project_lab,
                    plan.lab_id,
                    "imported genotyping record project",
                )?;
            }
            sqlx::query("INSERT INTO genotyping_records (id, lab_id, project_id, animal_id, genotype_definition_id, state, assessed_at, method, notes, supersedes_record_id, voided_at, void_reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(record.id.to_string())
                .bind(record.lab_id.to_string())
                .bind(record.project_id.map(|id| id.to_string()))
                .bind(record.animal_id.to_string())
                .bind(record.genotype_definition_id.to_string())
                .bind(encode(&record.state)?)
                .bind(record.assessed_at)
                .bind(&record.method)
                .bind(&record.notes)
                .bind(record.supersedes_record_id.map(|id| id.to_string()))
                .bind(record.voided_at)
                .bind(&record.void_reason)
                .bind(record.meta.created_at)
                .bind(record.meta.updated_at)
                .bind(record.meta.deleted_at)
                .bind(record.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                plan.lab_id,
                record.project_id,
                EntityType::GenotypingRecord,
                record.id,
                AuditAction::Import,
                audit,
                None,
                Some(snapshot(record)?),
            )
            .await?;
            let mut provenance = Provenance::from_audit(
                plan.lab_id,
                record.project_id,
                EntityType::GenotypingRecord,
                record.id,
                audit,
                committed_at,
            );
            provenance.source = import_source;
            provenance.import_job_id = options.job_id;
            provenance.import_commit_id = Some(plan.commit_id);
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        for pedigree in &plan.pedigrees {
            insert_pedigree_tx(&mut tx, pedigree, audit, AuditAction::Import).await?;
            let mut provenance = Provenance::from_audit(
                plan.lab_id,
                None,
                EntityType::Pedigree,
                pedigree.id,
                audit,
                committed_at,
            );
            provenance.source = import_source;
            provenance.import_job_id = options.job_id;
            provenance.import_commit_id = Some(plan.commit_id);
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        for measurement in &plan.measurements {
            insert_measurement_tx(&mut tx, measurement, audit, AuditAction::Import).await?;
            let mut provenance = Provenance::from_audit(
                plan.lab_id,
                Some(measurement.project_id),
                EntityType::Measurement,
                measurement.id,
                audit,
                committed_at,
            );
            provenance.source = import_source;
            provenance.import_job_id = options.job_id;
            provenance.import_commit_id = Some(plan.commit_id);
            insert_provenance_tx(&mut tx, &provenance).await?;
            let mut event = AnimalEvent::new(
                plan.lab_id,
                measurement.animal_id,
                AnimalEventKind::MeasurementRecorded {
                    measurement_id: measurement.id,
                },
                measurement.measured_at,
                committed_at,
            );
            event.project_id = Some(measurement.project_id);
            event.recorded_by = audit.actor.user_id;
            append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(ImportCommitResult {
            commit_id: plan.commit_id,
            preview_hash,
            counts,
            committed_at,
            replayed: false,
        })
    }

    async fn list_audit_entries(&self, filter: &AuditFilter) -> StoreResult<Vec<AuditEntry>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {AUDIT_COLUMNS} FROM audit_entries WHERE lab_id = "
        ));
        query.push_bind(filter.lab_id.to_string());
        if let Some(project_id) = filter.project_id {
            query.push(" AND project_id = ");
            query.push_bind(project_id.to_string());
        }
        if let Some(entity_id) = filter.entity_id {
            query.push(" AND entity_id = ");
            query.push_bind(entity_id.to_string());
        }
        query.push(" ORDER BY occurred_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(audit_from_row).collect()
    }
    async fn list_provenance(&self, filter: &ProvenanceFilter) -> StoreResult<Vec<Provenance>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {PROVENANCE_COLUMNS} FROM provenance WHERE lab_id = "
        ));
        query.push_bind(filter.lab_id.to_string());
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND project_id = ")
                .push_bind(project_id.to_string());
        }
        if let Some(entity_type) = filter.entity_type {
            query
                .push(" AND entity_type = ")
                .push_bind(entity_type.as_str());
        }
        if let Some(entity_id) = filter.entity_id {
            query
                .push(" AND entity_id = ")
                .push_bind(entity_id.to_string());
        }
        if let Some(source) = filter.source {
            query.push(" AND source = ").push_bind(encode(&source)?);
        }
        query.push(" ORDER BY recorded_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(provenance_from_row).collect()
    }
    async fn create_gene_locus(&self, locus: &GeneLocus, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO gene_loci (id, lab_id, symbol, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(locus.id.to_string()).bind(locus.lab_id.to_string()).bind(&locus.symbol).bind(&locus.description).bind(locus.meta.created_at).bind(locus.meta.updated_at).bind(locus.meta.deleted_at).bind(locus.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            locus.lab_id,
            None,
            EntityType::GeneLocus,
            locus.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(locus)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            locus.lab_id,
            None,
            EntityType::GeneLocus,
            locus.id,
            audit,
            locus.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_gene_locus(&self, id: Uuid) -> StoreResult<GeneLocus> {
        let row = sqlx::query(&format!(
            "SELECT {LOCUS_COLUMNS} FROM gene_loci WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "gene_locus",
            id,
        })?;
        locus_from_row(&row)
    }
    async fn list_gene_loci(&self, lab_id: Uuid) -> StoreResult<Vec<GeneLocus>> {
        let rows = sqlx::query(&format!("SELECT {LOCUS_COLUMNS} FROM gene_loci WHERE lab_id = ? AND deleted_at IS NULL ORDER BY symbol, id")).bind(lab_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(locus_from_row).collect()
    }

    async fn list_gene_loci_including_archived(&self, lab_id: Uuid) -> StoreResult<Vec<GeneLocus>> {
        let rows = sqlx::query(&format!(
            "SELECT {LOCUS_COLUMNS} FROM gene_loci WHERE lab_id = ? ORDER BY deleted_at IS NOT NULL, symbol, id"
        ))
        .bind(lab_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(locus_from_row).collect()
    }

    async fn gene_locus_reference_counts(&self, id: Uuid) -> StoreResult<GeneticsReferenceCounts> {
        self.get_gene_locus(id).await?;
        let id = id.to_string();
        let active_genotype_definitions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT gd.id) FROM genotype_components gc JOIN genotype_definitions gd ON gd.id = gc.genotype_definition_id WHERE gc.locus_id = ? AND gc.deleted_at IS NULL AND gd.deleted_at IS NULL",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let genotype_definitions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT genotype_definition_id) FROM genotype_components WHERE locus_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let genotyping_records = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT gr.id) FROM genotyping_records gr JOIN genotype_components gc ON gc.genotype_definition_id = gr.genotype_definition_id WHERE gc.locus_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let breeding_lines = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT blgd.breeding_line_id) FROM breeding_line_genotype_definitions blgd JOIN genotype_components gc ON gc.genotype_definition_id = blgd.genotype_definition_id WHERE gc.locus_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(GeneticsReferenceCounts {
            active_genotype_definitions: checked_count(
                active_genotype_definitions,
                "active genotype definition",
            )?,
            genotype_definitions: checked_count(genotype_definitions, "genotype definition")?,
            genotyping_records: checked_count(genotyping_records, "genotyping record")?,
            breeding_lines: checked_count(breeding_lines, "breeding line")?,
        })
    }

    async fn archive_gene_locus(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<GeneLocus> {
        let before = self.get_gene_locus(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "gene locus revision changed before archival".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_some() {
            return Err(StoreError::Conflict(
                "gene locus is already archived".to_owned(),
            ));
        }
        if self
            .gene_locus_reference_counts(id)
            .await?
            .active_genotype_definitions
            > 0
        {
            return Err(StoreError::Validation(
                "gene locus is referenced by an active genotype definition".to_owned(),
            ));
        }
        let mut after = before.clone();
        after.meta.soft_delete(archived_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE gene_loci SET deleted_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(after.meta.deleted_at)
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "gene locus revision changed before archival".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            after.lab_id,
            None,
            EntityType::GeneLocus,
            id,
            AuditAction::Archive,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            after.lab_id,
            None,
            EntityType::GeneLocus,
            id,
            audit,
            archived_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn restore_gene_locus(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<GeneLocus> {
        let before = self.get_gene_locus(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "gene locus revision changed before restoration".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_none() {
            return Err(StoreError::Conflict(
                "gene locus is not archived".to_owned(),
            ));
        }
        let mut after = before.clone();
        after.meta.deleted_at = None;
        after.meta.touch(restored_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE gene_loci SET deleted_at = NULL, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NOT NULL",
        )
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "gene locus revision changed before restoration".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            after.lab_id,
            None,
            EntityType::GeneLocus,
            id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            after.lab_id,
            None,
            EntityType::GeneLocus,
            id,
            audit,
            restored_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn create_allele(&self, allele: &Allele, audit: &AuditContext) -> StoreResult<()> {
        let locus = self.get_gene_locus(allele.locus_id).await?;
        if locus.meta.deleted_at.is_some() {
            return Err(StoreError::Validation(
                "allele locus is archived".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO alleles (id, locus_id, symbol, description, is_wild_type, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(allele.id.to_string()).bind(allele.locus_id.to_string()).bind(&allele.symbol).bind(&allele.description).bind(i64::from(allele.is_wild_type)).bind(allele.meta.created_at).bind(allele.meta.updated_at).bind(allele.meta.deleted_at).bind(allele.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            locus.lab_id,
            None,
            EntityType::Allele,
            allele.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(allele)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            locus.lab_id,
            None,
            EntityType::Allele,
            allele.id,
            audit,
            allele.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_allele(&self, id: Uuid) -> StoreResult<Allele> {
        let row = sqlx::query(&format!(
            "SELECT {ALLELE_COLUMNS} FROM alleles WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "allele",
            id,
        })?;
        allele_from_row(&row)
    }
    async fn list_alleles(&self, locus_id: Uuid) -> StoreResult<Vec<Allele>> {
        let rows = sqlx::query(&format!("SELECT {ALLELE_COLUMNS} FROM alleles WHERE locus_id = ? AND deleted_at IS NULL ORDER BY symbol, id")).bind(locus_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(allele_from_row).collect()
    }

    async fn list_alleles_including_archived(&self, locus_id: Uuid) -> StoreResult<Vec<Allele>> {
        let rows = sqlx::query(&format!(
            "SELECT {ALLELE_COLUMNS} FROM alleles WHERE locus_id = ? ORDER BY deleted_at IS NOT NULL, symbol, id"
        ))
        .bind(locus_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(allele_from_row).collect()
    }

    async fn allele_reference_counts(&self, id: Uuid) -> StoreResult<GeneticsReferenceCounts> {
        self.get_allele(id).await?;
        let id = id.to_string();
        let component_match = "(gc.allele_1_id = ? OR gc.allele_2_id = ?)";
        let active_genotype_definitions = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(DISTINCT gd.id) FROM genotype_components gc JOIN genotype_definitions gd ON gd.id = gc.genotype_definition_id WHERE {component_match} AND gc.deleted_at IS NULL AND gd.deleted_at IS NULL"
        ))
        .bind(&id)
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let genotype_definitions = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(DISTINCT gc.genotype_definition_id) FROM genotype_components gc WHERE {component_match}"
        ))
        .bind(&id)
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let genotyping_records = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(DISTINCT gr.id) FROM genotyping_records gr JOIN genotype_components gc ON gc.genotype_definition_id = gr.genotype_definition_id WHERE {component_match}"
        ))
        .bind(&id)
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let breeding_lines = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(DISTINCT blgd.breeding_line_id) FROM breeding_line_genotype_definitions blgd JOIN genotype_components gc ON gc.genotype_definition_id = blgd.genotype_definition_id WHERE {component_match}"
        ))
        .bind(&id)
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(GeneticsReferenceCounts {
            active_genotype_definitions: checked_count(
                active_genotype_definitions,
                "active genotype definition",
            )?,
            genotype_definitions: checked_count(genotype_definitions, "genotype definition")?,
            genotyping_records: checked_count(genotyping_records, "genotyping record")?,
            breeding_lines: checked_count(breeding_lines, "breeding line")?,
        })
    }

    async fn archive_allele(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Allele> {
        let before = self.get_allele(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "allele revision changed before archival".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_some() {
            return Err(StoreError::Conflict(
                "allele is already archived".to_owned(),
            ));
        }
        if self
            .allele_reference_counts(id)
            .await?
            .active_genotype_definitions
            > 0
        {
            return Err(StoreError::Validation(
                "allele is referenced by an active genotype definition".to_owned(),
            ));
        }
        let locus = self.get_gene_locus(before.locus_id).await?;
        let mut after = before.clone();
        after.meta.soft_delete(archived_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE alleles SET deleted_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(after.meta.deleted_at)
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "allele revision changed before archival".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            locus.lab_id,
            None,
            EntityType::Allele,
            id,
            AuditAction::Archive,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            locus.lab_id,
            None,
            EntityType::Allele,
            id,
            audit,
            archived_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn restore_allele(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Allele> {
        let before = self.get_allele(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "allele revision changed before restoration".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_none() {
            return Err(StoreError::Conflict("allele is not archived".to_owned()));
        }
        let locus = self.get_gene_locus(before.locus_id).await?;
        if locus.meta.deleted_at.is_some() {
            return Err(StoreError::Validation(
                "allele cannot be restored while its locus is archived".to_owned(),
            ));
        }
        let mut after = before.clone();
        after.meta.deleted_at = None;
        after.meta.touch(restored_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE alleles SET deleted_at = NULL, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NOT NULL",
        )
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "allele revision changed before restoration".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            locus.lab_id,
            None,
            EntityType::Allele,
            id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            locus.lab_id,
            None,
            EntityType::Allele,
            id,
            audit,
            restored_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn create_genotype(
        &self,
        genotype: &Genotype,
        project_id: Option<Uuid>,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let lab_id = required_lab_id(&mut tx, "animals", genotype.animal_id, "animal").await?;
        if let Some(project_id) = project_id {
            let project_lab = required_lab_id(&mut tx, "projects", project_id, "project").await?;
            require_same_uuid(project_lab, lab_id, "genotype project")?;
        }
        insert_genotype_tx(&mut tx, genotype, project_id, audit, AuditAction::Create).await?;
        let provenance = Provenance::from_audit(
            lab_id,
            project_id,
            EntityType::Genotype,
            genotype.id,
            audit,
            genotype.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            lab_id,
            genotype.animal_id,
            AnimalEventKind::Genotyped {
                genotype_ids: vec![genotype.id],
            },
            genotype.assessed_at.unwrap_or(genotype.meta.created_at),
            genotype.meta.created_at,
        );
        event.project_id = project_id;
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_genotype(&self, id: Uuid) -> StoreResult<Genotype> {
        let row = sqlx::query(&format!(
            "SELECT {GENOTYPE_COLUMNS} FROM genotypes WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "genotype",
            id,
        })?;
        genotype_from_row(&row)
    }
    async fn list_genotypes(&self, animal_id: Uuid) -> StoreResult<Vec<Genotype>> {
        let rows = sqlx::query(&format!("SELECT {GENOTYPE_COLUMNS} FROM genotypes WHERE animal_id = ? AND deleted_at IS NULL ORDER BY created_at, id")).bind(animal_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(genotype_from_row).collect()
    }

    async fn create_genotype_definition(
        &self,
        definition: &GenotypeDefinition,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        if definition.components.is_empty() {
            return Err(StoreError::Validation(
                "genotype definition requires at least one component".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO genotype_definitions (id, lab_id, name, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(definition.id.to_string())
            .bind(definition.lab_id.to_string())
            .bind(&definition.name)
            .bind(&definition.description)
            .bind(definition.meta.created_at)
            .bind(definition.meta.updated_at)
            .bind(definition.meta.deleted_at)
            .bind(definition.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        for component in &definition.components {
            component
                .validate()
                .map_err(|error| StoreError::Validation(error.to_string()))?;
            if component.genotype_definition_id != definition.id {
                return Err(StoreError::Validation(
                    "genotype component belongs to a different definition".to_owned(),
                ));
            }
            let locus_lab = sqlx::query_scalar::<_, String>(
                "SELECT lab_id FROM gene_loci WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(component.locus_id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "gene_locus",
                id: component.locus_id,
            })?;
            let locus_lab = uuid(&locus_lab)?;
            require_same_uuid(locus_lab, definition.lab_id, "genotype definition locus")?;
            for allele_id in [Some(component.allele_1_id), component.allele_2_id]
                .into_iter()
                .flatten()
            {
                let allele_locus = sqlx::query_scalar::<_, String>(
                    "SELECT locus_id FROM alleles WHERE id = ? AND deleted_at IS NULL",
                )
                .bind(allele_id.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx)?
                .ok_or(StoreError::NotFound {
                    entity: "allele",
                    id: allele_id,
                })?;
                require_same_uuid(
                    uuid(&allele_locus)?,
                    component.locus_id,
                    "genotype component allele",
                )?;
            }
            sqlx::query("INSERT INTO genotype_components (id, genotype_definition_id, locus_id, allele_1_id, allele_2_id, mode, display_order, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(component.id.to_string())
                .bind(component.genotype_definition_id.to_string())
                .bind(component.locus_id.to_string())
                .bind(component.allele_1_id.to_string())
                .bind(component.allele_2_id.map(|id| id.to_string()))
                .bind(encode(&component.mode)?)
                .bind(component.display_order)
                .bind(component.meta.created_at)
                .bind(component.meta.updated_at)
                .bind(component.meta.deleted_at)
                .bind(component.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }
        write_audit(
            &mut tx,
            definition.lab_id,
            None,
            EntityType::GenotypeDefinition,
            definition.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(definition)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            definition.lab_id,
            None,
            EntityType::GenotypeDefinition,
            definition.id,
            audit,
            definition.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_genotype_definition(&self, id: Uuid) -> StoreResult<GenotypeDefinition> {
        let row = sqlx::query(&format!(
            "SELECT {GENOTYPE_DEFINITION_COLUMNS} FROM genotype_definitions WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "genotype_definition",
            id,
        })?;
        let mut definition = genotype_definition_from_row(&row)?;
        definition.components = load_genotype_components_sqlite(&self.pool, definition.id).await?;
        Ok(definition)
    }

    async fn list_genotype_definitions(
        &self,
        lab_id: Uuid,
    ) -> StoreResult<Vec<GenotypeDefinition>> {
        let rows = sqlx::query(&format!(
            "SELECT {GENOTYPE_DEFINITION_COLUMNS} FROM genotype_definitions WHERE lab_id = ? AND deleted_at IS NULL ORDER BY name, id"
        ))
        .bind(lab_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let mut definitions = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut definition = genotype_definition_from_row(row)?;
            definition.components =
                load_genotype_components_sqlite(&self.pool, definition.id).await?;
            definitions.push(definition);
        }
        Ok(definitions)
    }

    async fn list_genotype_definitions_including_archived(
        &self,
        lab_id: Uuid,
    ) -> StoreResult<Vec<GenotypeDefinition>> {
        let rows = sqlx::query(&format!(
            "SELECT {GENOTYPE_DEFINITION_COLUMNS} FROM genotype_definitions WHERE lab_id = ? ORDER BY deleted_at IS NOT NULL, name, id"
        ))
        .bind(lab_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let mut definitions = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut definition = genotype_definition_from_row(row)?;
            definition.components =
                load_genotype_components_sqlite(&self.pool, definition.id).await?;
            definitions.push(definition);
        }
        Ok(definitions)
    }

    async fn genotype_definition_reference_counts(
        &self,
        id: Uuid,
    ) -> StoreResult<GeneticsReferenceCounts> {
        self.get_genotype_definition(id).await?;
        let id = id.to_string();
        let genotyping_records = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM genotyping_records WHERE genotype_definition_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let breeding_lines = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM breeding_line_genotype_definitions WHERE genotype_definition_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(GeneticsReferenceCounts {
            active_genotype_definitions: 0,
            genotype_definitions: 0,
            genotyping_records: checked_count(genotyping_records, "genotyping record")?,
            breeding_lines: checked_count(breeding_lines, "breeding line")?,
        })
    }

    async fn archive_genotype_definition(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<GenotypeDefinition> {
        let before = self.get_genotype_definition(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "genotype definition revision changed before archival".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_some() {
            return Err(StoreError::Conflict(
                "genotype definition is already archived".to_owned(),
            ));
        }
        let mut after = before.clone();
        after.meta.soft_delete(archived_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE genotype_definitions SET deleted_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(after.meta.deleted_at)
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "genotype definition revision changed before archival".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            after.lab_id,
            None,
            EntityType::GenotypeDefinition,
            id,
            AuditAction::Archive,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            after.lab_id,
            None,
            EntityType::GenotypeDefinition,
            id,
            audit,
            archived_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn restore_genotype_definition(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<GenotypeDefinition> {
        let before = self.get_genotype_definition(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "genotype definition revision changed before restoration".to_owned(),
            ));
        }
        if before.meta.deleted_at.is_none() {
            return Err(StoreError::Conflict(
                "genotype definition is not archived".to_owned(),
            ));
        }
        let inactive_components = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM genotype_components gc JOIN gene_loci gl ON gl.id = gc.locus_id JOIN alleles a1 ON a1.id = gc.allele_1_id LEFT JOIN alleles a2 ON a2.id = gc.allele_2_id WHERE gc.genotype_definition_id = ? AND gc.deleted_at IS NULL AND (gl.deleted_at IS NOT NULL OR a1.deleted_at IS NOT NULL OR (gc.allele_2_id IS NOT NULL AND a2.deleted_at IS NOT NULL))",
        )
        .bind(id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if inactive_components > 0 {
            return Err(StoreError::Validation(
                "genotype definition cannot be restored while a locus or allele is archived"
                    .to_owned(),
            ));
        }
        let mut after = before.clone();
        after.meta.deleted_at = None;
        after.meta.touch(restored_at);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE genotype_definitions SET deleted_at = NULL, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NOT NULL",
        )
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "genotype definition revision changed before restoration".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            after.lab_id,
            None,
            EntityType::GenotypeDefinition,
            id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            after.lab_id,
            None,
            EntityType::GenotypeDefinition,
            id,
            audit,
            restored_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn create_genotyping_record(
        &self,
        record: &GenotypingRecord,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        record
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if record.supersedes_record_id.is_some() || record.is_voided() {
            return Err(StoreError::Validation(
                "standard genotyping record creation cannot set lifecycle fields".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("UPDATE animals SET updated_at = updated_at WHERE id = ?")
            .bind(record.animal_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let animal_lab = required_lab_id(&mut tx, "animals", record.animal_id, "animal").await?;
        require_same_uuid(animal_lab, record.lab_id, "genotyping record animal")?;
        let definition_lab = sqlx::query_scalar::<_, String>(
            "SELECT lab_id FROM genotype_definitions WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(record.genotype_definition_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "genotype_definition",
            id: record.genotype_definition_id,
        })?;
        let definition_lab = uuid(&definition_lab)?;
        require_same_uuid(
            definition_lab,
            record.lab_id,
            "genotyping record definition",
        )?;
        if let Some(project_id) = record.project_id {
            let project_lab = required_lab_id(&mut tx, "projects", project_id, "project").await?;
            require_same_uuid(project_lab, record.lab_id, "genotyping record project")?;
        }
        sqlx::query("INSERT INTO genotyping_records (id, lab_id, project_id, animal_id, genotype_definition_id, state, assessed_at, method, notes, supersedes_record_id, voided_at, void_reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(record.id.to_string())
            .bind(record.lab_id.to_string())
            .bind(record.project_id.map(|id| id.to_string()))
            .bind(record.animal_id.to_string())
            .bind(record.genotype_definition_id.to_string())
            .bind(encode(&record.state)?)
            .bind(record.assessed_at)
            .bind(&record.method)
            .bind(&record.notes)
            .bind(record.supersedes_record_id.map(|id| id.to_string()))
            .bind(record.voided_at)
            .bind(&record.void_reason)
            .bind(record.meta.created_at)
            .bind(record.meta.updated_at)
            .bind(record.meta.deleted_at)
            .bind(record.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            record.lab_id,
            record.project_id,
            EntityType::GenotypingRecord,
            record.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(record)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            record.lab_id,
            record.project_id,
            EntityType::GenotypingRecord,
            record.id,
            audit,
            record.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            record.lab_id,
            record.animal_id,
            AnimalEventKind::GenotypingRecorded {
                record_id: record.id,
                genotype_definition_id: record.genotype_definition_id,
                state: record.state,
            },
            record.assessed_at.unwrap_or(record.meta.created_at),
            record.meta.created_at,
        );
        event.project_id = record.project_id;
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_genotyping_record(&self, id: Uuid) -> StoreResult<GenotypingRecord> {
        let row = sqlx::query(&format!(
            "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "genotyping_record",
            id,
        })?;
        genotyping_record_from_row(&row)
    }

    async fn list_genotyping_records(&self, animal_id: Uuid) -> StoreResult<Vec<GenotypingRecord>> {
        let rows = sqlx::query(&format!(
            "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE animal_id = ? ORDER BY created_at, id"
        ))
        .bind(animal_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(genotyping_record_from_row).collect()
    }

    async fn list_current_genotyping_records(
        &self,
        animal_id: Uuid,
    ) -> StoreResult<Vec<GenotypingRecord>> {
        let rows = sqlx::query(&format!(
            "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE animal_id = ? AND deleted_at IS NULL AND voided_at IS NULL ORDER BY created_at, id"
        ))
        .bind(animal_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let mut latest = BTreeMap::<Uuid, GenotypingRecord>::new();
        for row in &rows {
            let record = genotyping_record_from_row(row)?;
            latest.insert(record.genotype_definition_id, record);
        }
        Ok(latest.into_values().collect())
    }

    async fn void_genotyping_record(
        &self,
        id: Uuid,
        expected_revision: i64,
        reason: &str,
        voided_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<GenotypingRecord> {
        let before = self.get_genotyping_record(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "genotyping record revision changed before voiding".to_owned(),
            ));
        }
        let mut after = before.clone();
        after
            .void(reason, voided_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut operation_audit = audit.clone();
        operation_audit.reason = Some(after.void_reason.clone().expect("void reason was set"));
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("UPDATE animals SET updated_at = updated_at WHERE id = ?")
            .bind(after.animal_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let result = sqlx::query(
            "UPDATE genotyping_records SET voided_at = ?, void_reason = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL AND voided_at IS NULL",
        )
        .bind(after.voided_at)
        .bind(&after.void_reason)
        .bind(after.meta.updated_at)
        .bind(after.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "genotyping record revision changed before voiding".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            after.lab_id,
            after.project_id,
            EntityType::GenotypingRecord,
            id,
            AuditAction::Revoke,
            &operation_audit,
            Some(snapshot(&before)?),
            Some(snapshot(&after)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            after.lab_id,
            after.project_id,
            EntityType::GenotypingRecord,
            id,
            &operation_audit,
            voided_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(after)
    }

    async fn correct_genotyping_record(
        &self,
        id: Uuid,
        expected_revision: i64,
        reason: &str,
        voided_at: DateTime<Utc>,
        replacement: &GenotypingRecord,
        audit: &AuditContext,
    ) -> StoreResult<(GenotypingRecord, GenotypingRecord)> {
        replacement
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let before = self.get_genotyping_record(id).await?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "genotyping record revision changed before correction".to_owned(),
            ));
        }
        if replacement.supersedes_record_id != Some(id)
            || replacement.lab_id != before.lab_id
            || replacement.project_id != before.project_id
            || replacement.animal_id != before.animal_id
            || replacement.is_voided()
        {
            return Err(StoreError::Validation(
                "replacement genotyping record has incompatible identity or lifecycle fields"
                    .to_owned(),
            ));
        }
        let mut voided = before.clone();
        voided
            .void(reason, voided_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut operation_audit = audit.clone();
        operation_audit.reason = Some(voided.void_reason.clone().expect("void reason was set"));
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("UPDATE animals SET updated_at = updated_at WHERE id = ?")
            .bind(before.animal_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let definition_lab = sqlx::query_scalar::<_, String>(
            "SELECT lab_id FROM genotype_definitions WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(replacement.genotype_definition_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "genotype_definition",
            id: replacement.genotype_definition_id,
        })?;
        require_same_uuid(
            uuid(&definition_lab)?,
            replacement.lab_id,
            "replacement genotyping record definition",
        )?;
        let result = sqlx::query(
            "UPDATE genotyping_records SET voided_at = ?, void_reason = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL AND voided_at IS NULL",
        )
        .bind(voided.voided_at)
        .bind(&voided.void_reason)
        .bind(voided.meta.updated_at)
        .bind(voided.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "genotyping record revision changed before correction".to_owned(),
            ));
        }
        sqlx::query("INSERT INTO genotyping_records (id, lab_id, project_id, animal_id, genotype_definition_id, state, assessed_at, method, notes, supersedes_record_id, voided_at, void_reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(replacement.id.to_string())
            .bind(replacement.lab_id.to_string())
            .bind(replacement.project_id.map(|value| value.to_string()))
            .bind(replacement.animal_id.to_string())
            .bind(replacement.genotype_definition_id.to_string())
            .bind(encode(&replacement.state)?)
            .bind(replacement.assessed_at)
            .bind(&replacement.method)
            .bind(&replacement.notes)
            .bind(replacement.supersedes_record_id.map(|value| value.to_string()))
            .bind(replacement.voided_at)
            .bind(&replacement.void_reason)
            .bind(replacement.meta.created_at)
            .bind(replacement.meta.updated_at)
            .bind(replacement.meta.deleted_at)
            .bind(replacement.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            voided.lab_id,
            voided.project_id,
            EntityType::GenotypingRecord,
            id,
            AuditAction::Revoke,
            &operation_audit,
            Some(snapshot(&before)?),
            Some(snapshot(&voided)?),
        )
        .await?;
        let void_provenance = Provenance::from_audit(
            voided.lab_id,
            voided.project_id,
            EntityType::GenotypingRecord,
            id,
            &operation_audit,
            voided_at,
        );
        insert_provenance_tx(&mut tx, &void_provenance).await?;
        write_audit(
            &mut tx,
            replacement.lab_id,
            replacement.project_id,
            EntityType::GenotypingRecord,
            replacement.id,
            AuditAction::Create,
            &operation_audit,
            None,
            Some(snapshot(replacement)?),
        )
        .await?;
        let replacement_provenance = Provenance::from_audit(
            replacement.lab_id,
            replacement.project_id,
            EntityType::GenotypingRecord,
            replacement.id,
            &operation_audit,
            replacement.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &replacement_provenance).await?;
        let mut event = AnimalEvent::new(
            replacement.lab_id,
            replacement.animal_id,
            AnimalEventKind::GenotypingRecorded {
                record_id: replacement.id,
                genotype_definition_id: replacement.genotype_definition_id,
                state: replacement.state,
            },
            replacement
                .assessed_at
                .unwrap_or(replacement.meta.created_at),
            replacement.meta.created_at,
        );
        event.project_id = replacement.project_id;
        event.recorded_by = operation_audit.actor.user_id;
        append_derived_animal_event_tx(&mut tx, &event, &operation_audit).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok((voided, replacement.clone()))
    }

    async fn create_breeding_line(
        &self,
        line: &BreedingLine,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        if line.genotype_definition_ids.is_empty() {
            return Err(StoreError::Validation(
                "breeding line requires at least one genotype definition".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO breeding_lines (id, lab_id, name, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(line.id.to_string())
            .bind(line.lab_id.to_string())
            .bind(&line.name)
            .bind(&line.description)
            .bind(line.meta.created_at)
            .bind(line.meta.updated_at)
            .bind(line.meta.deleted_at)
            .bind(line.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        for (index, definition_id) in line.genotype_definition_ids.iter().enumerate() {
            let definition_lab = required_lab_id(
                &mut tx,
                "genotype_definitions",
                *definition_id,
                "genotype_definition",
            )
            .await?;
            require_same_uuid(
                definition_lab,
                line.lab_id,
                "breeding line genotype definition",
            )?;
            let display_order = i32::try_from(index).map_err(|_| {
                StoreError::Validation("too many breeding line genotype definitions".to_owned())
            })?;
            sqlx::query("INSERT INTO breeding_line_genotype_definitions (breeding_line_id, genotype_definition_id, display_order, created_at) VALUES (?, ?, ?, ?)")
                .bind(line.id.to_string())
                .bind(definition_id.to_string())
                .bind(display_order)
                .bind(line.meta.created_at)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }
        write_audit(
            &mut tx,
            line.lab_id,
            None,
            EntityType::BreedingLine,
            line.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(line)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            line.lab_id,
            None,
            EntityType::BreedingLine,
            line.id,
            audit,
            line.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_breeding_line(&self, id: Uuid) -> StoreResult<BreedingLine> {
        let row = sqlx::query(&format!(
            "SELECT {BREEDING_LINE_COLUMNS} FROM breeding_lines WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "breeding_line",
            id,
        })?;
        let mut line = breeding_line_from_row(&row)?;
        line.genotype_definition_ids =
            load_breeding_line_definition_ids_sqlite(&self.pool, id).await?;
        Ok(line)
    }

    async fn list_breeding_lines(&self, lab_id: Uuid) -> StoreResult<Vec<BreedingLine>> {
        let rows = sqlx::query(&format!("SELECT {BREEDING_LINE_COLUMNS} FROM breeding_lines WHERE lab_id = ? AND deleted_at IS NULL ORDER BY name, id"))
            .bind(lab_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let mut lines = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut line = breeding_line_from_row(row)?;
            line.genotype_definition_ids =
                load_breeding_line_definition_ids_sqlite(&self.pool, line.id).await?;
            lines.push(line);
        }
        Ok(lines)
    }

    async fn create_colony(&self, colony: &Colony, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let line_lab = required_lab_id(
            &mut tx,
            "breeding_lines",
            colony.breeding_line_id,
            "breeding_line",
        )
        .await?;
        require_same_uuid(line_lab, colony.lab_id, "colony breeding line")?;
        sqlx::query("INSERT INTO colonies (id, lab_id, breeding_line_id, name, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(colony.id.to_string()).bind(colony.lab_id.to_string())
            .bind(colony.breeding_line_id.to_string()).bind(&colony.name).bind(&colony.description)
            .bind(colony.meta.created_at).bind(colony.meta.updated_at).bind(colony.meta.deleted_at)
            .bind(colony.meta.revision).execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            colony.lab_id,
            None,
            EntityType::Colony,
            colony.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(colony)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            colony.lab_id,
            None,
            EntityType::Colony,
            colony.id,
            audit,
            colony.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_colony(&self, id: Uuid) -> StoreResult<Colony> {
        let row = sqlx::query(&format!(
            "SELECT {COLONY_COLUMNS} FROM colonies WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "colony",
            id,
        })?;
        colony_from_row(&row)
    }

    async fn list_colonies(
        &self,
        lab_id: Uuid,
        breeding_line_id: Option<Uuid>,
    ) -> StoreResult<Vec<Colony>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {COLONY_COLUMNS} FROM colonies WHERE lab_id = "
        ));
        query
            .push_bind(lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(line_id) = breeding_line_id {
            query
                .push(" AND breeding_line_id = ")
                .push_bind(line_id.to_string());
        }
        query.push(" ORDER BY name, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(colony_from_row).collect()
    }

    async fn create_breeding_pair(
        &self,
        pair: &BreedingPair,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut validated = pair.clone();
        validated
            .replace_members(pair.members.clone())
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if pair.status != BreedingPairStatus::Active || pair.ended_at.is_some() {
            return Err(StoreError::Validation(
                "new breeding pair must be active".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let colony_lab = required_lab_id(&mut tx, "colonies", pair.colony_id, "colony").await?;
        require_same_uuid(colony_lab, pair.lab_id, "breeding pair colony")?;
        sqlx::query("INSERT INTO breeding_pairs (id, lab_id, colony_id, name, status, started_at, ended_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(pair.id.to_string()).bind(pair.lab_id.to_string()).bind(pair.colony_id.to_string())
            .bind(&pair.name).bind(encode(&pair.status)?).bind(pair.started_at).bind(pair.ended_at)
            .bind(pair.meta.created_at).bind(pair.meta.updated_at).bind(pair.meta.deleted_at).bind(pair.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        for member in &pair.members {
            let animal_row =
                sqlx::query("SELECT lab_id, sex FROM animals WHERE id = ? AND deleted_at IS NULL")
                    .bind(member.animal_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(map_sqlx)?
                    .ok_or(StoreError::NotFound {
                        entity: "animal",
                        id: member.animal_id,
                    })?;
            let animal_lab = uuid(animal_row.try_get("lab_id").map_err(map_sqlx)?)?;
            require_same_uuid(animal_lab, pair.lab_id, "breeding pair animal")?;
            let sex: Sex = decode(animal_row.try_get("sex").map_err(map_sqlx)?)?;
            if !matches!(
                (member.role, sex),
                (BreedingMemberRole::Male, Sex::Male) | (BreedingMemberRole::Female, Sex::Female)
            ) {
                return Err(StoreError::Validation(
                    "breeding pair member role does not match animal sex".to_owned(),
                ));
            }
            let active_memberships: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM breeding_pair_members bpm JOIN breeding_pairs bp ON bp.id = bpm.breeding_pair_id WHERE bpm.animal_id = ? AND bpm.left_at IS NULL AND bpm.deleted_at IS NULL AND bp.status = ? AND bp.deleted_at IS NULL")
                .bind(member.animal_id.to_string()).bind(encode(&BreedingPairStatus::Active)?)
                .fetch_one(&mut *tx).await.map_err(map_sqlx)?;
            if active_memberships > 0 {
                return Err(StoreError::Conflict(
                    "animal already belongs to an active breeding pair".to_owned(),
                ));
            }
            sqlx::query("INSERT INTO breeding_pair_members (id, breeding_pair_id, animal_id, role, joined_at, left_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(member.id.to_string()).bind(member.breeding_pair_id.to_string()).bind(member.animal_id.to_string())
                .bind(encode(&member.role)?).bind(member.joined_at).bind(member.left_at)
                .bind(member.meta.created_at).bind(member.meta.updated_at).bind(member.meta.deleted_at).bind(member.meta.revision)
                .execute(&mut *tx).await.map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                pair.lab_id,
                None,
                EntityType::BreedingPairMember,
                member.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(member)?),
            )
            .await?;
            let provenance = Provenance::from_audit(
                pair.lab_id,
                None,
                EntityType::BreedingPairMember,
                member.id,
                audit,
                member.meta.created_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        write_audit(
            &mut tx,
            pair.lab_id,
            None,
            EntityType::BreedingPair,
            pair.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(pair)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            pair.lab_id,
            None,
            EntityType::BreedingPair,
            pair.id,
            audit,
            pair.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_breeding_pair(&self, id: Uuid) -> StoreResult<BreedingPair> {
        let row = sqlx::query(&format!(
            "SELECT {BREEDING_PAIR_COLUMNS} FROM breeding_pairs WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "breeding_pair",
            id,
        })?;
        let mut pair = breeding_pair_from_row(&row)?;
        pair.members = load_breeding_pair_members_sqlite(&self.pool, id).await?;
        Ok(pair)
    }

    async fn list_breeding_pairs(
        &self,
        lab_id: Uuid,
        colony_id: Option<Uuid>,
    ) -> StoreResult<Vec<BreedingPair>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {BREEDING_PAIR_COLUMNS} FROM breeding_pairs WHERE lab_id = "
        ));
        query
            .push_bind(lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(colony_id) = colony_id {
            query
                .push(" AND colony_id = ")
                .push_bind(colony_id.to_string());
        }
        query.push(" ORDER BY started_at DESC, name, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut pair = breeding_pair_from_row(row)?;
            pair.members = load_breeding_pair_members_sqlite(&self.pool, pair.id).await?;
            pairs.push(pair);
        }
        Ok(pairs)
    }

    async fn retire_breeding_pair(
        &self,
        id: Uuid,
        expected_revision: i64,
        ended_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<BreedingPair> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {BREEDING_PAIR_COLUMNS} FROM breeding_pairs WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "breeding_pair",
            id,
        })?;
        let mut pair = breeding_pair_from_row(&row)?;
        let member_rows = sqlx::query(&format!("SELECT {BREEDING_PAIR_MEMBER_COLUMNS} FROM breeding_pair_members WHERE breeding_pair_id = ? AND deleted_at IS NULL ORDER BY role, joined_at, id"))
            .bind(id.to_string()).fetch_all(&mut *tx).await.map_err(map_sqlx)?;
        pair.members = member_rows
            .iter()
            .map(breeding_pair_member_from_row)
            .collect::<StoreResult<Vec<_>>>()?;
        if pair.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "breeding pair revision changed before retirement".to_owned(),
            ));
        }
        let before = pair.clone();
        pair.retire(ended_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let updated = sqlx::query("UPDATE breeding_pairs SET status = ?, ended_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
            .bind(encode(&pair.status)?).bind(pair.ended_at).bind(pair.meta.updated_at).bind(pair.meta.revision)
            .bind(id.to_string()).bind(expected_revision).execute(&mut *tx).await.map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "breeding pair revision changed before retirement".to_owned(),
            ));
        }
        for (before_member, member) in before.members.iter().zip(&pair.members) {
            let updated = sqlx::query("UPDATE breeding_pair_members SET left_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
                .bind(member.left_at).bind(member.meta.updated_at).bind(member.meta.revision)
                .bind(member.id.to_string()).bind(before_member.meta.revision).execute(&mut *tx).await.map_err(map_sqlx)?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "breeding pair member revision changed before retirement".to_owned(),
                ));
            }
            write_audit(
                &mut tx,
                pair.lab_id,
                None,
                EntityType::BreedingPairMember,
                member.id,
                AuditAction::Update,
                audit,
                Some(snapshot(before_member)?),
                Some(snapshot(member)?),
            )
            .await?;
        }
        write_audit(
            &mut tx,
            pair.lab_id,
            None,
            EntityType::BreedingPair,
            pair.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&pair)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            pair.lab_id,
            None,
            EntityType::BreedingPair,
            pair.id,
            audit,
            ended_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(pair)
    }

    async fn create_mating_event(
        &self,
        event: &MatingEvent,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let pair_row = sqlx::query(
            "SELECT lab_id, status FROM breeding_pairs WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(event.breeding_pair_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "breeding_pair",
            id: event.breeding_pair_id,
        })?;
        let pair_lab = uuid(pair_row.try_get("lab_id").map_err(map_sqlx)?)?;
        require_same_uuid(pair_lab, event.lab_id, "mating event pair")?;
        let status: BreedingPairStatus = decode(pair_row.try_get("status").map_err(map_sqlx)?)?;
        if status != BreedingPairStatus::Active {
            return Err(StoreError::Validation(
                "mating event requires an active breeding pair".to_owned(),
            ));
        }
        for (animal_id, role) in [
            (event.male_animal_id, BreedingMemberRole::Male),
            (event.female_animal_id, BreedingMemberRole::Female),
        ] {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM breeding_pair_members WHERE breeding_pair_id = ? AND animal_id = ? AND role = ? AND left_at IS NULL AND deleted_at IS NULL")
                .bind(event.breeding_pair_id.to_string()).bind(animal_id.to_string()).bind(encode(&role)?)
                .fetch_one(&mut *tx).await.map_err(map_sqlx)?;
            if exists != 1 {
                return Err(StoreError::Validation(
                    "mating event parents must be active pair members with matching roles"
                        .to_owned(),
                ));
            }
        }
        sqlx::query("INSERT INTO mating_events (id, lab_id, breeding_pair_id, male_animal_id, female_animal_id, occurred_at, notes, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(event.id.to_string()).bind(event.lab_id.to_string()).bind(event.breeding_pair_id.to_string())
            .bind(event.male_animal_id.to_string()).bind(event.female_animal_id.to_string()).bind(event.occurred_at)
            .bind(&event.notes).bind(event.meta.created_at).bind(event.meta.updated_at).bind(event.meta.deleted_at).bind(event.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            event.lab_id,
            None,
            EntityType::MatingEvent,
            event.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(event)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            event.lab_id,
            None,
            EntityType::MatingEvent,
            event.id,
            audit,
            event.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_mating_event(&self, id: Uuid) -> StoreResult<MatingEvent> {
        let row = sqlx::query(&format!(
            "SELECT {MATING_EVENT_COLUMNS} FROM mating_events WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "mating_event",
            id,
        })?;
        mating_event_from_row(&row)
    }

    async fn list_mating_events(&self, breeding_pair_id: Uuid) -> StoreResult<Vec<MatingEvent>> {
        let rows = sqlx::query(&format!("SELECT {MATING_EVENT_COLUMNS} FROM mating_events WHERE breeding_pair_id = ? AND deleted_at IS NULL ORDER BY occurred_at DESC, id"))
            .bind(breeding_pair_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(mating_event_from_row).collect()
    }

    async fn create_litter(
        &self,
        litter: &Litter,
        drafts: &[AnimalDraft],
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let expected_drafts = usize::try_from(litter.size_alive)
            .map_err(|_| StoreError::Validation("invalid litter size_alive".to_owned()))?;
        if drafts.len() != expected_drafts {
            return Err(StoreError::Validation(
                "litter must provide one animal draft per live offspring".to_owned(),
            ));
        }
        let mut labels = std::collections::HashSet::with_capacity(drafts.len());
        if drafts.iter().any(|draft| {
            draft.lab_id != litter.lab_id
                || draft.litter_id != litter.id
                || draft.birth_date != litter.born_on
                || draft.status != AnimalDraftStatus::Pending
                || draft.registered_animal_id.is_some()
                || !labels.insert(draft.temporary_label.clone())
        }) {
            return Err(StoreError::Validation(
                "litter animal drafts are inconsistent".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let event_lab = required_lab_id(
            &mut tx,
            "mating_events",
            litter.mating_event_id,
            "mating_event",
        )
        .await?;
        require_same_uuid(event_lab, litter.lab_id, "litter mating event")?;
        sqlx::query("INSERT INTO litters (id, lab_id, mating_event_id, born_on, size_total, size_alive, notes, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(litter.id.to_string()).bind(litter.lab_id.to_string()).bind(litter.mating_event_id.to_string())
            .bind(litter.born_on).bind(litter.size_total).bind(litter.size_alive).bind(&litter.notes)
            .bind(litter.meta.created_at).bind(litter.meta.updated_at).bind(litter.meta.deleted_at).bind(litter.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        for draft in drafts {
            sqlx::query("INSERT INTO animal_drafts (id, lab_id, litter_id, temporary_label, sex, birth_date, status, registered_animal_id, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(draft.id.to_string()).bind(draft.lab_id.to_string()).bind(draft.litter_id.to_string()).bind(&draft.temporary_label)
                .bind(encode(&draft.sex)?).bind(draft.birth_date).bind(encode(&draft.status)?)
                .bind(draft.registered_animal_id.map(|id| id.to_string())).bind(draft.meta.created_at).bind(draft.meta.updated_at)
                .bind(draft.meta.deleted_at).bind(draft.meta.revision).execute(&mut *tx).await.map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                draft.lab_id,
                None,
                EntityType::AnimalDraft,
                draft.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(draft)?),
            )
            .await?;
            let provenance = Provenance::from_audit(
                draft.lab_id,
                None,
                EntityType::AnimalDraft,
                draft.id,
                audit,
                draft.meta.created_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        write_audit(
            &mut tx,
            litter.lab_id,
            None,
            EntityType::Litter,
            litter.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(litter)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            litter.lab_id,
            None,
            EntityType::Litter,
            litter.id,
            audit,
            litter.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_litter(&self, id: Uuid) -> StoreResult<Litter> {
        let row = sqlx::query(&format!(
            "SELECT {LITTER_COLUMNS} FROM litters WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "litter",
            id,
        })?;
        litter_from_row(&row)
    }

    async fn list_litters(&self, breeding_pair_id: Uuid) -> StoreResult<Vec<Litter>> {
        let rows = sqlx::query("SELECT l.id, l.lab_id, l.mating_event_id, l.born_on, l.size_total, l.size_alive, l.notes, l.created_at, l.updated_at, l.deleted_at, l.revision FROM litters l JOIN mating_events me ON me.id = l.mating_event_id WHERE me.breeding_pair_id = ? AND me.deleted_at IS NULL AND l.deleted_at IS NULL ORDER BY l.born_on DESC, l.id")
            .bind(breeding_pair_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(litter_from_row).collect()
    }

    async fn get_animal_draft(&self, id: Uuid) -> StoreResult<AnimalDraft> {
        let row = sqlx::query(&format!(
            "SELECT {ANIMAL_DRAFT_COLUMNS} FROM animal_drafts WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "animal_draft",
            id,
        })?;
        animal_draft_from_row(&row)
    }

    async fn list_animal_drafts(&self, litter_id: Uuid) -> StoreResult<Vec<AnimalDraft>> {
        let rows = sqlx::query(&format!("SELECT {ANIMAL_DRAFT_COLUMNS} FROM animal_drafts WHERE litter_id = ? AND deleted_at IS NULL ORDER BY temporary_label, id"))
            .bind(litter_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(animal_draft_from_row).collect()
    }

    async fn register_animal_draft(
        &self,
        draft_id: Uuid,
        expected_revision: i64,
        animal: &Animal,
        audit: &AuditContext,
    ) -> StoreResult<AnimalDraft> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {ANIMAL_DRAFT_COLUMNS} FROM animal_drafts WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(draft_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "animal_draft",
            id: draft_id,
        })?;
        let mut draft = animal_draft_from_row(&row)?;
        if draft.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "animal draft revision changed before registration".to_owned(),
            ));
        }
        if draft.status != AnimalDraftStatus::Pending
            || animal.lab_id != draft.lab_id
            || animal.sex != draft.sex
            || animal.birth_date != Some(draft.birth_date)
        {
            return Err(StoreError::Validation(
                "registered animal must match its pending draft lab, sex and birth date".to_owned(),
            ));
        }
        let parent_row = sqlx::query("SELECT me.male_animal_id, me.female_animal_id FROM animal_drafts ad JOIN litters l ON l.id = ad.litter_id JOIN mating_events me ON me.id = l.mating_event_id WHERE ad.id = ? AND l.deleted_at IS NULL AND me.deleted_at IS NULL")
            .bind(draft_id.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx)?
            .ok_or(StoreError::Validation("animal draft lineage is incomplete".to_owned()))?;
        let male_id = uuid(parent_row.try_get("male_animal_id").map_err(map_sqlx)?)?;
        let female_id = uuid(parent_row.try_get("female_animal_id").map_err(map_sqlx)?)?;
        let before = draft.clone();
        draft
            .mark_registered(animal.id, animal.meta.created_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        insert_animal_tx(&mut tx, animal, audit, AuditAction::Create).await?;
        let mut events = vec![
            AnimalEvent::new(
                animal.lab_id,
                animal.id,
                AnimalEventKind::Registered,
                animal.meta.created_at,
                animal.meta.created_at,
            ),
            AnimalEvent::new(
                animal.lab_id,
                animal.id,
                AnimalEventKind::Born {
                    birth_date: draft.birth_date,
                },
                animal.meta.created_at,
                animal.meta.created_at,
            ),
        ];
        if let Some(cage_id) = animal.current_cage_id {
            events.push(AnimalEvent::new(
                animal.lab_id,
                animal.id,
                AnimalEventKind::Transferred {
                    from_cage_id: None,
                    to_cage_id: Some(cage_id),
                },
                animal.meta.created_at,
                animal.meta.created_at,
            ));
        }
        for event in &mut events {
            event.recorded_by = audit.actor.user_id;
            insert_animal_event_tx(&mut tx, event, audit, AuditAction::Create).await?;
        }
        let provenance = Provenance::from_audit(
            animal.lab_id,
            None,
            EntityType::Animal,
            animal.id,
            audit,
            animal.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        for (parent_id, parent_type) in [
            (male_id, ParentType::Father),
            (female_id, ParentType::Mother),
        ] {
            let pedigree = Pedigree {
                id: Uuid::new_v4(),
                animal_id: animal.id,
                parent_id,
                parent_type,
                meta: RecordMeta::new(animal.meta.created_at),
            };
            insert_pedigree_tx(&mut tx, &pedigree, audit, AuditAction::Create).await?;
            let provenance = Provenance::from_audit(
                animal.lab_id,
                None,
                EntityType::Pedigree,
                pedigree.id,
                audit,
                animal.meta.created_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        let updated = sqlx::query("UPDATE animal_drafts SET status = ?, registered_animal_id = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
            .bind(encode(&draft.status)?).bind(animal.id.to_string()).bind(draft.meta.updated_at).bind(draft.meta.revision)
            .bind(draft.id.to_string()).bind(expected_revision).execute(&mut *tx).await.map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "animal draft revision changed before registration".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            draft.lab_id,
            None,
            EntityType::AnimalDraft,
            draft.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&draft)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            draft.lab_id,
            None,
            EntityType::AnimalDraft,
            draft.id,
            audit,
            draft.meta.updated_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(draft)
    }

    async fn create_experiment_event(
        &self,
        event: &ExperimentEvent,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        event
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_observation_scope_sqlite(
            &mut tx,
            event.lab_id,
            event.project_id,
            event.experiment_id,
        )
        .await?;
        sqlx::query("INSERT INTO experiment_events (id, lab_id, project_id, experiment_id, event_key, label, occurred_at, details_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(event.id.to_string())
            .bind(event.lab_id.to_string())
            .bind(event.project_id.to_string())
            .bind(event.experiment_id.to_string())
            .bind(&event.event_key)
            .bind(&event.label)
            .bind(event.occurred_at)
            .bind(serde_json::to_string(&event.details).map_err(|error| StoreError::Serialization(error.to_string()))?)
            .bind(event.meta.created_at)
            .bind(event.meta.updated_at)
            .bind(event.meta.deleted_at)
            .bind(event.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            event.lab_id,
            Some(event.project_id),
            EntityType::ExperimentEvent,
            event.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(event)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            event.lab_id,
            Some(event.project_id),
            EntityType::ExperimentEvent,
            event.id,
            audit,
            event.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_experiment_event(&self, id: Uuid) -> StoreResult<ExperimentEvent> {
        let row = sqlx::query(&format!("SELECT {EXPERIMENT_EVENT_COLUMNS} FROM experiment_events WHERE id = ? AND deleted_at IS NULL"))
            .bind(id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "experiment_event", id })?;
        experiment_event_from_row(&row)
    }

    async fn list_experiment_events(
        &self,
        experiment_id: Uuid,
    ) -> StoreResult<Vec<ExperimentEvent>> {
        let rows = sqlx::query(&format!("SELECT {EXPERIMENT_EVENT_COLUMNS} FROM experiment_events WHERE experiment_id = ? AND deleted_at IS NULL ORDER BY occurred_at, id"))
            .bind(experiment_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(experiment_event_from_row).collect()
    }

    async fn create_observation_definition(
        &self,
        definition: &ObservationDefinition,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        definition
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_observation_scope_sqlite(
            &mut tx,
            definition.lab_id,
            definition.project_id,
            definition.experiment_id,
        )
        .await?;
        sqlx::query("INSERT INTO observation_definitions (id, lab_id, project_id, experiment_id, observation_key, label, value_type, unit, categories_json, policy, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(definition.id.to_string())
            .bind(definition.lab_id.to_string())
            .bind(definition.project_id.to_string())
            .bind(definition.experiment_id.to_string())
            .bind(&definition.key)
            .bind(&definition.label)
            .bind(encode(&definition.value_type)?)
            .bind(&definition.unit)
            .bind(serde_json::to_string(&definition.categories).map_err(|error| StoreError::Serialization(error.to_string()))?)
            .bind(encode(&definition.policy)?)
            .bind(definition.meta.created_at)
            .bind(definition.meta.updated_at)
            .bind(definition.meta.deleted_at)
            .bind(definition.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            definition.lab_id,
            Some(definition.project_id),
            EntityType::ObservationDefinition,
            definition.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(definition)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            definition.lab_id,
            Some(definition.project_id),
            EntityType::ObservationDefinition,
            definition.id,
            audit,
            definition.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_observation_definition(&self, id: Uuid) -> StoreResult<ObservationDefinition> {
        let row = sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE id = ? AND deleted_at IS NULL"))
            .bind(id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "observation_definition", id })?;
        observation_definition_from_row(&row)
    }

    async fn list_observation_definitions(
        &self,
        experiment_id: Uuid,
    ) -> StoreResult<Vec<ObservationDefinition>> {
        let rows = sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE experiment_id = ? AND deleted_at IS NULL ORDER BY observation_key, id"))
            .bind(experiment_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(observation_definition_from_row).collect()
    }

    async fn create_observation(
        &self,
        observation: &Observation,
        value: &ObservationValueRecord,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_observation_recorder(value, audit)?;
        observation
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if value.observation_id != observation.id
            || value.version != 1
            || observation.current_value_version != 1
        {
            return Err(StoreError::Validation(
                "initial observation value must be version one and owned by the observation"
                    .to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_observation_scope_sqlite(
            &mut tx,
            observation.lab_id,
            observation.project_id,
            observation.experiment_id,
        )
        .await?;
        let event_row = sqlx::query("SELECT lab_id, project_id, experiment_id FROM experiment_events WHERE id = ? AND deleted_at IS NULL")
            .bind(observation.experiment_event_id.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "experiment_event", id: observation.experiment_event_id })?;
        let definition_row = sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE id = ? AND deleted_at IS NULL"))
            .bind(observation.definition_id.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "observation_definition", id: observation.definition_id })?;
        for (row_lab, row_project, row_experiment, relationship) in [
            (
                uuid(event_row.try_get("lab_id").map_err(map_sqlx)?)?,
                uuid(event_row.try_get("project_id").map_err(map_sqlx)?)?,
                uuid(event_row.try_get("experiment_id").map_err(map_sqlx)?)?,
                "observation event",
            ),
            (
                uuid(definition_row.try_get("lab_id").map_err(map_sqlx)?)?,
                uuid(definition_row.try_get("project_id").map_err(map_sqlx)?)?,
                uuid(definition_row.try_get("experiment_id").map_err(map_sqlx)?)?,
                "observation definition",
            ),
        ] {
            require_same_uuid(row_lab, observation.lab_id, relationship)?;
            require_same_uuid(row_project, observation.project_id, relationship)?;
            require_same_uuid(row_experiment, observation.experiment_id, relationship)?;
        }
        let definition = observation_definition_from_row(&definition_row)?;
        definition
            .validate_value(&value.value)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        validate_observation_subject_sqlite(&mut tx, observation).await?;
        sqlx::query("INSERT INTO observations (id, lab_id, project_id, experiment_id, experiment_event_id, definition_id, subject_type, subject_id, context_json, current_value_version, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(observation.id.to_string())
            .bind(observation.lab_id.to_string())
            .bind(observation.project_id.to_string())
            .bind(observation.experiment_id.to_string())
            .bind(observation.experiment_event_id.to_string())
            .bind(observation.definition_id.to_string())
            .bind(encode(&observation.subject_type)?)
            .bind(observation.subject_id.to_string())
            .bind(serde_json::to_string(&observation.context).map_err(|error| StoreError::Serialization(error.to_string()))?)
            .bind(observation.current_value_version)
            .bind(observation.meta.created_at)
            .bind(observation.meta.updated_at)
            .bind(observation.meta.deleted_at)
            .bind(observation.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        insert_observation_value_sqlite(&mut tx, value).await?;
        write_audit(
            &mut tx,
            observation.lab_id,
            Some(observation.project_id),
            EntityType::Observation,
            observation.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(observation)?),
        )
        .await?;
        write_audit(
            &mut tx,
            observation.lab_id,
            Some(observation.project_id),
            EntityType::ObservationValue,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        for (entity_type, entity_id, recorded_at) in [
            (
                EntityType::Observation,
                observation.id,
                observation.meta.created_at,
            ),
            (
                EntityType::ObservationValue,
                value.id,
                value.meta.created_at,
            ),
        ] {
            let provenance = Provenance::from_audit(
                observation.lab_id,
                Some(observation.project_id),
                entity_type,
                entity_id,
                audit,
                recorded_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_observation(&self, id: Uuid) -> StoreResult<Observation> {
        let row = sqlx::query(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "observation",
            id,
        })?;
        observation_from_row(&row)
    }

    async fn list_observations(&self, filter: &ObservationFilter) -> StoreResult<Vec<Observation>> {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations WHERE experiment_id = "
        ));
        query
            .push_bind(filter.experiment_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(event_id) = filter.experiment_event_id {
            query
                .push(" AND experiment_event_id = ")
                .push_bind(event_id.to_string());
        }
        if let Some(subject_type) = filter.subject_type {
            query
                .push(" AND subject_type = ")
                .push_bind(encode(&subject_type)?);
        }
        if let Some(subject_id) = filter.subject_id {
            query
                .push(" AND subject_id = ")
                .push_bind(subject_id.to_string());
        }
        query.push(" ORDER BY created_at, id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(observation_from_row).collect()
    }

    async fn get_observation_value(&self, id: Uuid) -> StoreResult<ObservationValueRecord> {
        let row = sqlx::query(&format!("SELECT {OBSERVATION_VALUE_COLUMNS} FROM observation_values WHERE id = ? AND deleted_at IS NULL"))
            .bind(id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "observation_value", id })?;
        observation_value_from_row(&row)
    }

    async fn list_observation_values(
        &self,
        observation_id: Uuid,
    ) -> StoreResult<Vec<ObservationValueRecord>> {
        let rows = sqlx::query(&format!("SELECT {OBSERVATION_VALUE_COLUMNS} FROM observation_values WHERE observation_id = ? AND deleted_at IS NULL ORDER BY version, id"))
            .bind(observation_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(observation_value_from_row).collect()
    }

    async fn revise_observation_value(
        &self,
        observation_id: Uuid,
        expected_revision: i64,
        value: &ObservationValueRecord,
        audit: &AuditContext,
    ) -> StoreResult<Observation> {
        validate_observation_recorder(value, audit)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(observation_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "observation",
            id: observation_id,
        })?;
        let mut observation = observation_from_row(&row)?;
        if observation.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "observation revision changed before value revision".to_owned(),
            ));
        }
        if value.observation_id != observation.id
            || value.version != observation.current_value_version + 1
        {
            return Err(StoreError::Validation(
                "observation value version must follow the current version".to_owned(),
            ));
        }
        let definition_row = sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE id = ? AND deleted_at IS NULL"))
            .bind(observation.definition_id.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "observation_definition", id: observation.definition_id })?;
        let definition = observation_definition_from_row(&definition_row)?;
        if definition.policy == ObservationPolicy::Immutable {
            return Err(StoreError::Validation(
                DomainError::ObservationImmutable.to_string(),
            ));
        }
        definition
            .validate_value(&value.value)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let before = observation.clone();
        observation
            .advance_value_version(value.version, value.meta.created_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        insert_observation_value_sqlite(&mut tx, value).await?;
        let updated = sqlx::query("UPDATE observations SET current_value_version = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
            .bind(observation.current_value_version)
            .bind(observation.meta.updated_at)
            .bind(observation.meta.revision)
            .bind(observation.id.to_string())
            .bind(expected_revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "observation revision changed before value revision".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            observation.lab_id,
            Some(observation.project_id),
            EntityType::ObservationValue,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        write_audit(
            &mut tx,
            observation.lab_id,
            Some(observation.project_id),
            EntityType::Observation,
            observation.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&observation)?),
        )
        .await?;
        for (entity_type, entity_id) in [
            (EntityType::ObservationValue, value.id),
            (EntityType::Observation, observation.id),
        ] {
            let provenance = Provenance::from_audit(
                observation.lab_id,
                Some(observation.project_id),
                entity_type,
                entity_id,
                audit,
                value.meta.created_at,
            );
            insert_provenance_tx(&mut tx, &provenance).await?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(observation)
    }

    async fn create_pedigree(&self, pedigree: &Pedigree, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        insert_pedigree_tx(&mut tx, pedigree, audit, AuditAction::Create).await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_pedigree(&self, id: Uuid) -> StoreResult<Pedigree> {
        let row = sqlx::query(&format!(
            "SELECT {PEDIGREE_COLUMNS} FROM pedigrees WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "pedigree",
            id,
        })?;
        pedigree_from_row(&row)
    }
    async fn list_pedigrees(&self, animal_id: Uuid) -> StoreResult<Vec<Pedigree>> {
        let rows = sqlx::query(&format!("SELECT {PEDIGREE_COLUMNS} FROM pedigrees WHERE animal_id = ? AND deleted_at IS NULL ORDER BY parent_type, id")).bind(animal_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(pedigree_from_row).collect()
    }
    async fn list_related_pedigrees(&self, animal_id: Uuid) -> StoreResult<Vec<Pedigree>> {
        let rows = sqlx::query(&format!(
            "SELECT {PEDIGREE_COLUMNS} FROM pedigrees WHERE (animal_id = ? OR parent_id = ?) AND deleted_at IS NULL ORDER BY animal_id, parent_type, id"
        ))
        .bind(animal_id.to_string())
        .bind(animal_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(pedigree_from_row).collect()
    }
    async fn create_template_version(
        &self,
        template: &ExperimentTemplateVersion,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("INSERT INTO experiment_template_versions (id, lab_id, template_key, version, name, description, status, fields_json, published_at, published_by, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(template.id.to_string()).bind(template.lab_id.to_string()).bind(&template.template_key).bind(template.version).bind(&template.name).bind(&template.description).bind(encode(&template.status)?).bind(serde_json::to_string(&template.fields).map_err(|e| StoreError::Serialization(e.to_string()))?).bind(template.published_at).bind(template.published_by.map(|id| id.to_string())).bind(template.meta.created_at).bind(template.meta.updated_at).bind(template.meta.deleted_at).bind(template.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        let action = if template.status == TemplateStatus::Published {
            AuditAction::Publish
        } else {
            AuditAction::Create
        };
        write_audit(
            &mut tx,
            template.lab_id,
            None,
            EntityType::ExperimentTemplateVersion,
            template.id,
            action,
            audit,
            None,
            Some(snapshot(template)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_template_version(&self, id: Uuid) -> StoreResult<ExperimentTemplateVersion> {
        let row = sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM experiment_template_versions WHERE id = ? AND deleted_at IS NULL")).bind(id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound { entity: "experiment_template_version", id })?;
        template_from_row(&row)
    }
    async fn publish_template_version(
        &self,
        id: Uuid,
        expected_revision: i64,
        published_by: Uuid,
        published_at: chrono::DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<ExperimentTemplateVersion> {
        if audit.actor.actor_type != ActorType::Human || audit.actor.user_id != Some(published_by) {
            return Err(StoreError::Validation(
                "template publication must identify the publishing human actor".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {TEMPLATE_COLUMNS} FROM experiment_template_versions WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "experiment_template_version",
            id,
        })?;
        let before = template_from_row(&row)?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "template revision changed before publication was applied".to_owned(),
            ));
        }
        let mut published = before.clone();
        published
            .publish(published_by, published_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;

        let result = sqlx::query(
            "UPDATE experiment_template_versions SET status = ?, published_at = ?, published_by = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND status = ? AND deleted_at IS NULL",
        )
        .bind(encode(&published.status)?)
        .bind(published.published_at)
        .bind(published.published_by.map(|value| value.to_string()))
        .bind(published.meta.updated_at)
        .bind(published.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .bind(encode(&TemplateStatus::Draft)?)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "template revision changed before publication was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            published.lab_id,
            None,
            EntityType::ExperimentTemplateVersion,
            published.id,
            AuditAction::Publish,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&published)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(published)
    }
    async fn list_template_versions(
        &self,
        lab_id: Uuid,
        template_key: Option<&str>,
    ) -> StoreResult<Vec<ExperimentTemplateVersion>> {
        let mut q = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {TEMPLATE_COLUMNS} FROM experiment_template_versions WHERE lab_id = "
        ));
        q.push_bind(lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(key) = template_key {
            q.push(" AND template_key = ").push_bind(key.to_owned());
        }
        q.push(" ORDER BY template_key, version DESC");
        let rows = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(template_from_row).collect()
    }

    async fn create_cohort(&self, cohort: &Cohort, audit: &AuditContext) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let (experiment_lab, experiment_project) =
            required_experiment_scope(&mut tx, cohort.experiment_id).await?;
        sqlx::query("INSERT INTO cohorts (id, experiment_id, name, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(cohort.id.to_string()).bind(cohort.experiment_id.to_string()).bind(&cohort.name).bind(&cohort.description).bind(cohort.meta.created_at).bind(cohort.meta.updated_at).bind(cohort.meta.deleted_at).bind(cohort.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            experiment_lab,
            Some(experiment_project),
            EntityType::Cohort,
            cohort.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(cohort)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn list_cohorts(&self, experiment_id: Uuid) -> StoreResult<Vec<Cohort>> {
        let rows = sqlx::query(&format!("SELECT {COHORT_COLUMNS} FROM cohorts WHERE experiment_id = ? AND deleted_at IS NULL ORDER BY name, id")).bind(experiment_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(cohort_from_row).collect()
    }

    async fn create_procedure(
        &self,
        procedure: &Procedure,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let (experiment_lab, experiment_project) =
            required_experiment_scope(&mut tx, procedure.experiment_id).await?;
        if let Some(animal_id) = procedure.animal_id {
            let animal_lab = required_lab_id(&mut tx, "animals", animal_id, "animal").await?;
            require_same_uuid(animal_lab, experiment_lab, "procedure animal")?;
            require_experiment_participation(
                &mut tx,
                procedure.experiment_id,
                animal_id,
                "procedure",
            )
            .await?;
        }
        sqlx::query("INSERT INTO procedures (id, experiment_id, animal_id, name, scheduled_at, performed_at, status, details_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(procedure.id.to_string()).bind(procedure.experiment_id.to_string()).bind(procedure.animal_id.map(|id| id.to_string())).bind(&procedure.name).bind(procedure.scheduled_at).bind(procedure.performed_at).bind(encode(&procedure.status)?).bind(procedure.details.to_string()).bind(procedure.meta.created_at).bind(procedure.meta.updated_at).bind(procedure.meta.deleted_at).bind(procedure.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            experiment_lab,
            Some(experiment_project),
            EntityType::Procedure,
            procedure.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(procedure)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            experiment_lab,
            Some(experiment_project),
            EntityType::Procedure,
            procedure.id,
            audit,
            procedure.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        let event = ExperimentEvent {
            id: Uuid::new_v4(),
            lab_id: experiment_lab,
            project_id: experiment_project,
            experiment_id: procedure.experiment_id,
            event_key: format!("procedure_{}", procedure.id),
            label: procedure.name.clone(),
            occurred_at: procedure
                .performed_at
                .or(procedure.scheduled_at)
                .unwrap_or(procedure.meta.created_at),
            details: serde_json::json!({
                "source": "procedure",
                "procedure_id": procedure.id,
                "procedure_status": procedure.status,
            }),
            meta: RecordMeta::new(procedure.meta.created_at),
        };
        event
            .validate()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        sqlx::query("INSERT INTO experiment_events (id, lab_id, project_id, experiment_id, event_key, label, occurred_at, details_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(event.id.to_string())
            .bind(event.lab_id.to_string())
            .bind(event.project_id.to_string())
            .bind(event.experiment_id.to_string())
            .bind(&event.event_key)
            .bind(&event.label)
            .bind(event.occurred_at)
            .bind(serde_json::to_string(&event.details).map_err(|error| StoreError::Serialization(error.to_string()))?)
            .bind(event.meta.created_at)
            .bind(event.meta.updated_at)
            .bind(event.meta.deleted_at)
            .bind(event.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            event.lab_id,
            Some(event.project_id),
            EntityType::ExperimentEvent,
            event.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(&event)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            event.lab_id,
            Some(event.project_id),
            EntityType::ExperimentEvent,
            event.id,
            audit,
            event.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        if procedure.status == ProcedureStatus::Completed
            && let Some(animal_id) = procedure.animal_id
        {
            let occurred_at = procedure.performed_at.ok_or_else(|| {
                StoreError::Validation(
                    "completed animal procedure must have performed_at".to_owned(),
                )
            })?;
            let mut event = AnimalEvent::new(
                experiment_lab,
                animal_id,
                AnimalEventKind::ProcedurePerformed {
                    procedure_id: procedure.id,
                },
                occurred_at,
                procedure.meta.created_at,
            );
            event.project_id = Some(experiment_project);
            event.recorded_by = audit.actor.user_id;
            append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        }
        tx.commit().await.map_err(map_sqlx)
    }
    async fn list_procedures(
        &self,
        experiment_id: Uuid,
        animal_id: Option<Uuid>,
    ) -> StoreResult<Vec<Procedure>> {
        let mut q = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {PROCEDURE_COLUMNS} FROM procedures WHERE experiment_id = "
        ));
        q.push_bind(experiment_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(animal_id) = animal_id {
            q.push(" AND animal_id = ").push_bind(animal_id.to_string());
        }
        q.push(" ORDER BY COALESCE(performed_at, scheduled_at), id");
        let rows = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(procedure_from_row).collect()
    }

    async fn create_attachment(
        &self,
        attachment: &Attachment,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        if attachment.size_bytes < 0 || attachment.version < 1 || attachment.sha256.len() != 64 {
            return Err(StoreError::Validation(
                "attachment size, version or SHA-256 is invalid".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        if let Some(project_id) = attachment.project_id {
            let project_lab = required_lab_id(&mut tx, "projects", project_id, "project").await?;
            require_same_uuid(project_lab, attachment.lab_id, "attachment project")?;
        }
        sqlx::query("INSERT INTO attachments (id, lab_id, project_id, entity_type, entity_id, file_name, media_type, relative_path, size_bytes, sha256, version, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(attachment.id.to_string()).bind(attachment.lab_id.to_string()).bind(attachment.project_id.map(|id| id.to_string())).bind(&attachment.entity_type).bind(attachment.entity_id.to_string()).bind(&attachment.file_name).bind(&attachment.media_type).bind(&attachment.relative_path).bind(attachment.size_bytes).bind(&attachment.sha256).bind(attachment.version).bind(attachment.meta.created_at).bind(attachment.meta.updated_at).bind(attachment.meta.deleted_at).bind(attachment.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(attachment)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            audit,
            attachment.meta.created_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn get_attachment(&self, id: Uuid) -> StoreResult<Attachment> {
        let row = sqlx::query(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "attachment",
            id,
        })?;
        attachment_from_row(&row)
    }

    async fn soft_delete_attachment(
        &self,
        id: Uuid,
        expected_revision: i64,
        deleted_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Attachment> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "attachment",
            id,
        })?;
        let mut attachment = attachment_from_row(&row)?;
        if attachment.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "attachment revision changed before deletion".to_owned(),
            ));
        }
        if attachment.entity_type != "project"
            || attachment.project_id != Some(attachment.entity_id)
        {
            return Err(StoreError::Conflict(
                "attachments linked to research records must be unlinked before deletion"
                    .to_owned(),
            ));
        }
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM attachment_links WHERE attachment_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if link_count > 0 {
            return Err(StoreError::Conflict(
                "linked attachments must be unlinked before deletion".to_owned(),
            ));
        }
        let private_image_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ai_private_images WHERE attachment_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let extraction_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ai_extraction_drafts WHERE attachment_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if private_image_count > 0 || extraction_count > 0 {
            return Err(StoreError::Conflict(
                "AI evidence attachments cannot be deleted from the project library".to_owned(),
            ));
        }
        let before = attachment.clone();
        attachment.meta.soft_delete(deleted_at);
        let updated = sqlx::query(
            "UPDATE attachments SET updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(attachment.meta.updated_at)
        .bind(attachment.meta.deleted_at)
        .bind(attachment.meta.revision)
        .bind(id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "attachment revision changed before deletion".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            AuditAction::SoftDelete,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&attachment)?),
        )
        .await?;
        let provenance = Provenance::from_audit(
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            audit,
            attachment.meta.updated_at,
        );
        insert_provenance_tx(&mut tx, &provenance).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(attachment)
    }

    async fn list_attachments(
        &self,
        lab_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
    ) -> StoreResult<Vec<Attachment>> {
        let rows = sqlx::query(&format!("SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE lab_id = ? AND entity_type = ? AND entity_id = ? AND deleted_at IS NULL ORDER BY file_name, version, id"))
            .bind(lab_id.to_string()).bind(entity_type).bind(entity_id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(attachment_from_row).collect()
    }

    async fn list_lab_attachments(&self, lab_id: Uuid) -> StoreResult<Vec<Attachment>> {
        let rows = sqlx::query(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE lab_id = ? AND deleted_at IS NULL ORDER BY id"
        ))
        .bind(lab_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(attachment_from_row).collect()
    }
}
fn locus_from_row(row: &SqliteRow) -> StoreResult<GeneLocus> {
    Ok(GeneLocus {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        symbol: row.try_get("symbol").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}
fn allele_from_row(row: &SqliteRow) -> StoreResult<Allele> {
    Ok(Allele {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        locus_id: uuid(row.try_get("locus_id").map_err(map_sqlx)?)?,
        symbol: row.try_get("symbol").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        is_wild_type: row.try_get::<i64, _>("is_wild_type").map_err(map_sqlx)? != 0,
        meta: meta(row)?,
    })
}
fn genotype_from_row(row: &SqliteRow) -> StoreResult<Genotype> {
    Ok(Genotype {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        locus_id: uuid(row.try_get("locus_id").map_err(map_sqlx)?)?,
        allele_1_id: optional_uuid(row.try_get("allele_1_id").map_err(map_sqlx)?)?,
        allele_2_id: optional_uuid(row.try_get("allele_2_id").map_err(map_sqlx)?)?,
        assessed_at: row.try_get("assessed_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}
fn genotype_definition_from_row(row: &SqliteRow) -> StoreResult<GenotypeDefinition> {
    Ok(GenotypeDefinition {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        components: Vec::new(),
        meta: meta(row)?,
    })
}

fn genotype_component_from_row(row: &SqliteRow) -> StoreResult<GenotypeComponent> {
    Ok(GenotypeComponent {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        genotype_definition_id: uuid(row.try_get("genotype_definition_id").map_err(map_sqlx)?)?,
        locus_id: uuid(row.try_get("locus_id").map_err(map_sqlx)?)?,
        allele_1_id: uuid(row.try_get("allele_1_id").map_err(map_sqlx)?)?,
        allele_2_id: optional_uuid(row.try_get("allele_2_id").map_err(map_sqlx)?)?,
        mode: decode(row.try_get("mode").map_err(map_sqlx)?)?,
        display_order: row.try_get("display_order").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

async fn load_genotype_components_sqlite(
    pool: &SqlitePool,
    definition_id: Uuid,
) -> StoreResult<Vec<GenotypeComponent>> {
    let rows = sqlx::query(&format!(
        "SELECT {GENOTYPE_COMPONENT_COLUMNS} FROM genotype_components WHERE genotype_definition_id = ? AND deleted_at IS NULL ORDER BY display_order, id"
    ))
    .bind(definition_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(genotype_component_from_row).collect()
}

fn genotyping_record_from_row(row: &SqliteRow) -> StoreResult<GenotypingRecord> {
    Ok(GenotypingRecord {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        genotype_definition_id: uuid(row.try_get("genotype_definition_id").map_err(map_sqlx)?)?,
        state: decode(row.try_get("state").map_err(map_sqlx)?)?,
        assessed_at: row.try_get("assessed_at").map_err(map_sqlx)?,
        method: row.try_get("method").map_err(map_sqlx)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
        supersedes_record_id: optional_uuid(
            row.try_get("supersedes_record_id").map_err(map_sqlx)?,
        )?,
        voided_at: row.try_get("voided_at").map_err(map_sqlx)?,
        void_reason: row.try_get("void_reason").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn breeding_line_from_row(row: &SqliteRow) -> StoreResult<BreedingLine> {
    Ok(BreedingLine {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        genotype_definition_ids: Vec::new(),
        meta: meta(row)?,
    })
}

fn colony_from_row(row: &SqliteRow) -> StoreResult<Colony> {
    Ok(Colony {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        breeding_line_id: uuid(row.try_get("breeding_line_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn breeding_pair_from_row(row: &SqliteRow) -> StoreResult<BreedingPair> {
    Ok(BreedingPair {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        colony_id: uuid(row.try_get("colony_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        started_at: row.try_get("started_at").map_err(map_sqlx)?,
        ended_at: row.try_get("ended_at").map_err(map_sqlx)?,
        members: Vec::new(),
        meta: meta(row)?,
    })
}

fn breeding_pair_member_from_row(row: &SqliteRow) -> StoreResult<BreedingPairMember> {
    Ok(BreedingPairMember {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        breeding_pair_id: uuid(row.try_get("breeding_pair_id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        role: decode(row.try_get("role").map_err(map_sqlx)?)?,
        joined_at: row.try_get("joined_at").map_err(map_sqlx)?,
        left_at: row.try_get("left_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn mating_event_from_row(row: &SqliteRow) -> StoreResult<MatingEvent> {
    Ok(MatingEvent {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        breeding_pair_id: uuid(row.try_get("breeding_pair_id").map_err(map_sqlx)?)?,
        male_animal_id: uuid(row.try_get("male_animal_id").map_err(map_sqlx)?)?,
        female_animal_id: uuid(row.try_get("female_animal_id").map_err(map_sqlx)?)?,
        occurred_at: row.try_get("occurred_at").map_err(map_sqlx)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn litter_from_row(row: &SqliteRow) -> StoreResult<Litter> {
    Ok(Litter {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        mating_event_id: uuid(row.try_get("mating_event_id").map_err(map_sqlx)?)?,
        born_on: row.try_get("born_on").map_err(map_sqlx)?,
        size_total: row.try_get("size_total").map_err(map_sqlx)?,
        size_alive: row.try_get("size_alive").map_err(map_sqlx)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn animal_draft_from_row(row: &SqliteRow) -> StoreResult<AnimalDraft> {
    Ok(AnimalDraft {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        litter_id: uuid(row.try_get("litter_id").map_err(map_sqlx)?)?,
        temporary_label: row.try_get("temporary_label").map_err(map_sqlx)?,
        sex: decode(row.try_get("sex").map_err(map_sqlx)?)?,
        birth_date: row.try_get("birth_date").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        registered_animal_id: optional_uuid(
            row.try_get("registered_animal_id").map_err(map_sqlx)?,
        )?,
        meta: meta(row)?,
    })
}

async fn load_breeding_line_definition_ids_sqlite(
    pool: &SqlitePool,
    line_id: Uuid,
) -> StoreResult<Vec<Uuid>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT genotype_definition_id FROM breeding_line_genotype_definitions WHERE breeding_line_id = ? ORDER BY display_order",
    )
    .bind(line_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(|value| uuid(value)).collect()
}

async fn load_breeding_pair_members_sqlite(
    pool: &SqlitePool,
    pair_id: Uuid,
) -> StoreResult<Vec<BreedingPairMember>> {
    let rows = sqlx::query(&format!(
        "SELECT {BREEDING_PAIR_MEMBER_COLUMNS} FROM breeding_pair_members WHERE breeding_pair_id = ? AND deleted_at IS NULL ORDER BY role, joined_at, id"
    ))
    .bind(pair_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(breeding_pair_member_from_row).collect()
}

fn experiment_event_from_row(row: &SqliteRow) -> StoreResult<ExperimentEvent> {
    Ok(ExperimentEvent {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        event_key: row.try_get("event_key").map_err(map_sqlx)?,
        label: row.try_get("label").map_err(map_sqlx)?,
        occurred_at: row.try_get("occurred_at").map_err(map_sqlx)?,
        details: serde_json::from_str(row.try_get("details_json").map_err(map_sqlx)?)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        meta: meta(row)?,
    })
}

fn observation_definition_from_row(row: &SqliteRow) -> StoreResult<ObservationDefinition> {
    Ok(ObservationDefinition {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        key: row.try_get("observation_key").map_err(map_sqlx)?,
        label: row.try_get("label").map_err(map_sqlx)?,
        value_type: decode(row.try_get("value_type").map_err(map_sqlx)?)?,
        unit: row.try_get("unit").map_err(map_sqlx)?,
        categories: serde_json::from_str(row.try_get("categories_json").map_err(map_sqlx)?)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        policy: decode(row.try_get("policy").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}

fn observation_from_row(row: &SqliteRow) -> StoreResult<Observation> {
    Ok(Observation {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        experiment_event_id: uuid(row.try_get("experiment_event_id").map_err(map_sqlx)?)?,
        definition_id: uuid(row.try_get("definition_id").map_err(map_sqlx)?)?,
        subject_type: decode(row.try_get("subject_type").map_err(map_sqlx)?)?,
        subject_id: uuid(row.try_get("subject_id").map_err(map_sqlx)?)?,
        context: serde_json::from_str(row.try_get("context_json").map_err(map_sqlx)?)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        current_value_version: row.try_get("current_value_version").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn observation_value_from_row(row: &SqliteRow) -> StoreResult<ObservationValueRecord> {
    Ok(ObservationValueRecord {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        observation_id: uuid(row.try_get("observation_id").map_err(map_sqlx)?)?,
        version: row.try_get("version").map_err(map_sqlx)?,
        value: serde_json::from_str(row.try_get("value_json").map_err(map_sqlx)?)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        recorded_at: row.try_get("recorded_at").map_err(map_sqlx)?,
        recorded_by: optional_uuid(row.try_get("recorded_by").map_err(map_sqlx)?)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

async fn validate_observation_scope_sqlite(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    lab_id: Uuid,
    project_id: Uuid,
    experiment_id: Uuid,
) -> StoreResult<()> {
    let project_lab = required_lab_id(tx, "projects", project_id, "project").await?;
    require_same_uuid(project_lab, lab_id, "observation project")?;
    let (experiment_lab, experiment_project) = required_experiment_scope(tx, experiment_id).await?;
    require_same_uuid(experiment_lab, lab_id, "observation experiment")?;
    require_same_uuid(experiment_project, project_id, "observation experiment")?;
    Ok(())
}

async fn validate_observation_subject_sqlite(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    observation: &Observation,
) -> StoreResult<()> {
    match observation.subject_type {
        ObservationSubjectType::Experiment => {
            require_same_uuid(
                observation.subject_id,
                observation.experiment_id,
                "observation experiment subject",
            )?;
        }
        ObservationSubjectType::Animal => {
            let lab = required_lab_id(tx, "animals", observation.subject_id, "animal").await?;
            require_same_uuid(lab, observation.lab_id, "observation animal subject")?;
            require_experiment_participation(
                tx,
                observation.experiment_id,
                observation.subject_id,
                "observation",
            )
            .await?;
        }
        ObservationSubjectType::Sample => {
            let row = sqlx::query(
                "SELECT lab_id, project_id, experiment_id FROM samples WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(observation.subject_id.to_string())
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "sample",
                id: observation.subject_id,
            })?;
            require_same_uuid(
                uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
                observation.lab_id,
                "observation sample",
            )?;
            require_same_uuid(
                uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
                observation.project_id,
                "observation sample",
            )?;
            let sample_experiment = optional_uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?;
            if sample_experiment != Some(observation.experiment_id) {
                return Err(StoreError::Validation(
                    "observation sample belongs to a different experiment".to_owned(),
                ));
            }
        }
        ObservationSubjectType::Artifact => {
            let row = sqlx::query(
                "SELECT lab_id, project_id FROM attachments WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(observation.subject_id.to_string())
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "attachment",
                id: observation.subject_id,
            })?;
            require_same_uuid(
                uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
                observation.lab_id,
                "observation artifact",
            )?;
            let project_id = optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?;
            if project_id != Some(observation.project_id) {
                return Err(StoreError::Validation(
                    "observation artifact belongs to a different project".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

async fn insert_observation_value_sqlite(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    value: &ObservationValueRecord,
) -> StoreResult<()> {
    value
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    sqlx::query("INSERT INTO observation_values (id, observation_id, version, value_json, recorded_at, recorded_by, notes, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(value.id.to_string())
        .bind(value.observation_id.to_string())
        .bind(value.version)
        .bind(serde_json::to_string(&value.value).map_err(|error| StoreError::Serialization(error.to_string()))?)
        .bind(value.recorded_at)
        .bind(value.recorded_by.map(|id| id.to_string()))
        .bind(&value.notes)
        .bind(value.meta.created_at)
        .bind(value.meta.updated_at)
        .bind(value.meta.deleted_at)
        .bind(value.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

fn pedigree_from_row(row: &SqliteRow) -> StoreResult<Pedigree> {
    Ok(Pedigree {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        animal_id: uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        parent_id: uuid(row.try_get("parent_id").map_err(map_sqlx)?)?,
        parent_type: decode(row.try_get("parent_type").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}
fn template_from_row(row: &SqliteRow) -> StoreResult<ExperimentTemplateVersion> {
    Ok(ExperimentTemplateVersion {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        template_key: row.try_get("template_key").map_err(map_sqlx)?,
        version: row.try_get("version").map_err(map_sqlx)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        fields: serde_json::from_str(row.try_get("fields_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        published_at: row.try_get("published_at").map_err(map_sqlx)?,
        published_by: optional_uuid(row.try_get("published_by").map_err(map_sqlx)?)?,
        meta: meta(row)?,
    })
}
fn cohort_from_row(row: &SqliteRow) -> StoreResult<Cohort> {
    Ok(Cohort {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        description: row.try_get("description").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}
fn procedure_from_row(row: &SqliteRow) -> StoreResult<Procedure> {
    Ok(Procedure {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        experiment_id: uuid(row.try_get("experiment_id").map_err(map_sqlx)?)?,
        animal_id: optional_uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        scheduled_at: row.try_get("scheduled_at").map_err(map_sqlx)?,
        performed_at: row.try_get("performed_at").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        details: serde_json::from_str(row.try_get("details_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        meta: meta(row)?,
    })
}
fn attachment_from_row(row: &SqliteRow) -> StoreResult<Attachment> {
    Ok(Attachment {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        entity_type: row.try_get("entity_type").map_err(map_sqlx)?,
        entity_id: uuid(row.try_get("entity_id").map_err(map_sqlx)?)?,
        file_name: row.try_get("file_name").map_err(map_sqlx)?,
        media_type: row.try_get("media_type").map_err(map_sqlx)?,
        relative_path: row.try_get("relative_path").map_err(map_sqlx)?,
        size_bytes: row.try_get("size_bytes").map_err(map_sqlx)?,
        sha256: row.try_get("sha256").map_err(map_sqlx)?,
        version: row.try_get("version").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

const LOCUS_COLUMNS: &str =
    "id, lab_id, symbol, description, created_at, updated_at, deleted_at, revision";
const ALLELE_COLUMNS: &str =
    "id, locus_id, symbol, description, is_wild_type, created_at, updated_at, deleted_at, revision";
const GENOTYPE_COLUMNS: &str = "id, animal_id, locus_id, allele_1_id, allele_2_id, assessed_at, created_at, updated_at, deleted_at, revision";
const GENOTYPE_DEFINITION_COLUMNS: &str =
    "id, lab_id, name, description, created_at, updated_at, deleted_at, revision";
const GENOTYPE_COMPONENT_COLUMNS: &str = "id, genotype_definition_id, locus_id, allele_1_id, allele_2_id, mode, display_order, created_at, updated_at, deleted_at, revision";
const GENOTYPING_RECORD_COLUMNS: &str = "id, lab_id, project_id, animal_id, genotype_definition_id, state, assessed_at, method, notes, supersedes_record_id, voided_at, void_reason, created_at, updated_at, deleted_at, revision";
const BREEDING_LINE_COLUMNS: &str =
    "id, lab_id, name, description, created_at, updated_at, deleted_at, revision";
const COLONY_COLUMNS: &str =
    "id, lab_id, breeding_line_id, name, description, created_at, updated_at, deleted_at, revision";
const BREEDING_PAIR_COLUMNS: &str = "id, lab_id, colony_id, name, status, started_at, ended_at, created_at, updated_at, deleted_at, revision";
const BREEDING_PAIR_MEMBER_COLUMNS: &str = "id, breeding_pair_id, animal_id, role, joined_at, left_at, created_at, updated_at, deleted_at, revision";
const MATING_EVENT_COLUMNS: &str = "id, lab_id, breeding_pair_id, male_animal_id, female_animal_id, occurred_at, notes, created_at, updated_at, deleted_at, revision";
const LITTER_COLUMNS: &str = "id, lab_id, mating_event_id, born_on, size_total, size_alive, notes, created_at, updated_at, deleted_at, revision";
const ANIMAL_DRAFT_COLUMNS: &str = "id, lab_id, litter_id, temporary_label, sex, birth_date, status, registered_animal_id, created_at, updated_at, deleted_at, revision";
const EXPERIMENT_EVENT_COLUMNS: &str = "id, lab_id, project_id, experiment_id, event_key, label, occurred_at, details_json, created_at, updated_at, deleted_at, revision";
const OBSERVATION_DEFINITION_COLUMNS: &str = "id, lab_id, project_id, experiment_id, observation_key, label, value_type, unit, categories_json, policy, created_at, updated_at, deleted_at, revision";
const OBSERVATION_COLUMNS: &str = "id, lab_id, project_id, experiment_id, experiment_event_id, definition_id, subject_type, subject_id, context_json, current_value_version, created_at, updated_at, deleted_at, revision";
const OBSERVATION_VALUE_COLUMNS: &str = "id, observation_id, version, value_json, recorded_at, recorded_by, notes, created_at, updated_at, deleted_at, revision";
const PEDIGREE_COLUMNS: &str =
    "id, animal_id, parent_id, parent_type, created_at, updated_at, deleted_at, revision";
const TEMPLATE_COLUMNS: &str = "id, lab_id, template_key, version, name, description, status, fields_json, published_at, published_by, created_at, updated_at, deleted_at, revision";
const COHORT_COLUMNS: &str =
    "id, experiment_id, name, description, created_at, updated_at, deleted_at, revision";
const PROCEDURE_COLUMNS: &str = "id, experiment_id, animal_id, name, scheduled_at, performed_at, status, details_json, created_at, updated_at, deleted_at, revision";
const ATTACHMENT_COLUMNS: &str = "id, lab_id, project_id, entity_type, entity_id, file_name, media_type, relative_path, size_bytes, sha256, version, created_at, updated_at, deleted_at, revision";
