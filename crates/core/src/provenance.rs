use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::EntityType;

/// How a persisted scientific record entered MuriArc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    Human,
    Import,
    Ai,
    Migration,
}

/// Immutable origin metadata for one domain entity revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub source: ProvenanceSource,
    pub actor_user_id: Option<Uuid>,
    pub import_job_id: Option<Uuid>,
    pub import_commit_id: Option<Uuid>,
    pub tool_run_id: Option<Uuid>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub confidence: Option<f64>,
    pub request_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

impl Provenance {
    pub fn from_audit(
        lab_id: Uuid,
        project_id: Option<Uuid>,
        entity_type: EntityType,
        entity_id: Uuid,
        audit: &AuditContext,
        recorded_at: DateTime<Utc>,
    ) -> Self {
        let source = match (audit.actor.actor_type, audit.source) {
            (ActorType::Migration, _) | (_, WriteSource::Migration) => ProvenanceSource::Migration,
            (ActorType::Ai, _) | (_, WriteSource::Ai | WriteSource::Mcp) => ProvenanceSource::Ai,
            _ => ProvenanceSource::Human,
        };
        Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            entity_type,
            entity_id,
            source,
            actor_user_id: audit.actor.user_id,
            import_job_id: None,
            import_commit_id: None,
            tool_run_id: None,
            provider: None,
            model: None,
            confidence: None,
            request_id: audit.request_id.clone(),
            recorded_at,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.entity_id.is_nil()
            || self.project_id.is_some_and(|id| id.is_nil())
            || self.actor_user_id.is_some_and(|id| id.is_nil())
            || self.import_job_id.is_some_and(|id| id.is_nil())
            || self.import_commit_id.is_some_and(|id| id.is_nil())
            || self.tool_run_id.is_some_and(|id| id.is_nil())
        {
            return Err("provenance identifiers must not be nil");
        }
        if self
            .confidence
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err("provenance confidence must be between zero and one");
        }
        if self
            .provider
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 128)
            || self
                .model
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 256)
            || self
                .request_id
                .as_ref()
                .is_some_and(|value| value.len() > 256)
        {
            return Err("provenance text metadata is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Human,
    Ai,
    System,
    Migration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub actor_type: ActorType,
    pub user_id: Option<Uuid>,
    pub display_name: String,
}

impl Actor {
    pub fn human(user_id: Uuid, display_name: impl Into<String>) -> Self {
        Self {
            actor_type: ActorType::Human,
            user_id: Some(user_id),
            display_name: display_name.into(),
        }
    }

    pub fn system(display_name: impl Into<String>) -> Self {
        Self {
            actor_type: ActorType::System,
            user_id: None,
            display_name: display_name.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteSource {
    Desktop,
    Web,
    Api,
    Mcp,
    Ai,
    Migration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditContext {
    pub actor: Actor,
    pub source: WriteSource,
    pub request_id: Option<String>,
    pub reason: Option<String>,
}

impl AuditContext {
    pub fn system(source: WriteSource) -> Self {
        Self {
            actor: Actor::system("MuriArc"),
            source,
            request_id: None,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Create,
    Update,
    SoftDelete,
    Revoke,
    Publish,
    Sign,
    Import,
    Link,
    Archive,
    Process,
    Approve,
    Export,
    Cleanup,
    EnterAdminView,
}

impl AuditAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::SoftDelete => "soft_delete",
            Self::Revoke => "revoke",
            Self::Publish => "publish",
            Self::Sign => "sign",
            Self::Import => "import",
            Self::Link => "link",
            Self::Archive => "archive",
            Self::Process => "process",
            Self::Approve => "approve",
            Self::Export => "export",
            Self::Cleanup => "cleanup",
            Self::EnterAdminView => "enter_admin_view",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub action: AuditAction,
    pub actor: Actor,
    pub source: WriteSource,
    pub request_id: Option<String>,
    pub reason: Option<String>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub operation_code: String,
    pub operation_version: i32,
    pub operation_params: Value,
    pub entity_name_snapshot: Option<String>,
    pub entity_revision: Option<i64>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationMetadata {
    pub code: String,
    pub version: i32,
    pub params: Value,
    pub entity_name_snapshot: Option<String>,
    pub entity_revision: Option<i64>,
}

pub fn operation_metadata(
    entity_type: EntityType,
    action: AuditAction,
    before: Option<&Value>,
    after: Option<&Value>,
) -> OperationMetadata {
    let snapshot = after.or(before);
    let entity_name_snapshot = snapshot.and_then(snapshot_name);
    let entity_revision = snapshot.and_then(snapshot_revision);
    OperationMetadata {
        code: format!("{}.{}", entity_type.as_str(), action.as_str()),
        version: 1,
        params: json!({
            "entityType": entity_type.as_str(),
            "action": action.as_str(),
        }),
        entity_name_snapshot,
        entity_revision,
    }
}

fn snapshot_name(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["name", "display_id", "file_name", "label", "email", "title"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn snapshot_revision(value: &Value) -> Option<i64> {
    value
        .get("meta")
        .and_then(|meta| meta.get("revision"))
        .or_else(|| value.get("revision"))
        .and_then(Value::as_i64)
}
