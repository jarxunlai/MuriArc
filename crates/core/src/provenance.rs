use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{AiConversationSource, Attachment, EntityType};

/// Attachment targets whose lifecycle is managed by an AI workspace.
///
/// This classification does not make every matching attachment private:
/// callers must also inspect its lifecycle and project scope. In particular,
/// an archived conversation source is a formal project attachment.
pub fn is_ai_managed_attachment_entity_type(entity_type: &str) -> bool {
    matches!(entity_type, "ai_conversation_source" | "ai_private_image")
}

/// AI workspace and provider records that are not shared research history.
///
/// These rows can contain owner-private conversation bodies, tool input or
/// output, approval statements, private source metadata, or administrator-only
/// provider configuration. Dedicated owner/admin APIs expose their safe views;
/// shared snapshots and public audit projections must not expose their durable
/// audit/provenance records.
pub const fn is_ai_operational_or_configuration_entity_type(entity_type: EntityType) -> bool {
    matches!(
        entity_type,
        EntityType::AiPrivateImage
            | EntityType::AiConversationSource
            | EntityType::AiExtractionDraft
            | EntityType::AiConversation
            | EntityType::AiConversationMessage
            | EntityType::AiAutonomyGrant
            | EntityType::AiProviderSettings
            | EntityType::AiProviderEndpoint
            | EntityType::AiLabSettings
            | EntityType::ToolRun
            | EntityType::Approval
    )
}

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

/// Produces the durable audit projection for a private AI source attachment.
///
/// The attachment entity ID remains the audit subject, while immutable object
/// addressing details never enter `before_json` or `after_json`.
pub fn ai_source_attachment_audit_snapshot(
    attachment: &Attachment,
) -> Result<Value, serde_json::Error> {
    let mut snapshot = serde_json::to_value(attachment)?;
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("relative_path");
        object.remove("sha256");
    }
    Ok(snapshot)
}

/// Produces the durable audit projection for a conversation source without
/// persisting its internal attachment relationship.
pub fn ai_conversation_source_audit_snapshot(
    source: &AiConversationSource,
) -> Result<Value, serde_json::Error> {
    let mut snapshot = serde_json::to_value(source)?;
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("attachment_id");
    }
    Ok(snapshot)
}

/// Serializes an AI message or ToolRun for audit while retaining the public
/// source citation projection and removing internal attachment identities.
pub fn ai_source_ref_safe_audit_snapshot<T: Serialize>(
    value: &T,
) -> Result<Value, serde_json::Error> {
    let mut snapshot = serde_json::to_value(value)?;
    redact_attachment_ids_in_source_refs(&mut snapshot);
    Ok(snapshot)
}

/// Returns true for attachment audits created only to persist private AI
/// source/image storage. Public activity and audit routes omit these entries
/// so their `entity_id` cannot reveal the internal attachment identity.
pub fn is_private_ai_source_attachment_audit(entry: &AuditEntry) -> bool {
    entry.entity_type == EntityType::Attachment
        && entry
            .before
            .iter()
            .chain(entry.after.iter())
            .any(snapshot_is_private_ai_attachment)
}

/// Returns true for Job audits whose durable snapshots contain the private
/// binding used by AI source-backed imports.
///
/// The Job remains available to its owner through the dedicated Job API, but
/// its audit row is not a safe public activity item: `entity_id`,
/// `idempotency_key`, and `result` can otherwise correlate a conversation,
/// source, attachment, and preview.
pub fn is_private_ai_source_job_audit(entry: &AuditEntry) -> bool {
    entry.entity_type == EntityType::Job
        && entry
            .before
            .iter()
            .chain(entry.after.iter())
            .any(snapshot_is_private_ai_source_job)
}

/// Defense-in-depth for public audit projections, including historical rows
/// written before source-specific safe snapshots existed.
pub fn redact_public_audit_entry(entry: &mut AuditEntry) {
    for value in entry
        .before
        .iter_mut()
        .chain(entry.after.iter_mut())
        .chain(std::iter::once(&mut entry.operation_params))
    {
        redact_sensitive_audit_keys(value);
        if entry.entity_type == EntityType::Job {
            redact_job_technical_payload(value);
        }
    }
}

/// Applies the shared public/model audit boundary before any projection exposes
/// audit entity identifiers or snapshot-derived metadata.
pub fn protect_public_audit_entries(entries: &mut Vec<AuditEntry>) {
    entries.retain(|entry| {
        !is_ai_operational_or_configuration_entity_type(entry.entity_type)
            && !is_private_ai_source_attachment_audit(entry)
            && !is_private_ai_source_job_audit(entry)
    });
    for entry in entries {
        redact_public_audit_entry(entry);
    }
}

fn redact_attachment_ids_in_source_refs(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                redact_attachment_ids_in_source_refs(value);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                if normalized_key(key) == "sourcerefs" {
                    redact_attachment_ids(value);
                } else {
                    redact_attachment_ids_in_source_refs(value);
                }
            }
        }
        _ => {}
    }
}

fn redact_attachment_ids(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                redact_attachment_ids(value);
            }
        }
        Value::Object(object) => {
            object.retain(|key, _| normalized_key(key) != "attachmentid");
            for value in object.values_mut() {
                redact_attachment_ids(value);
            }
        }
        _ => {}
    }
}

fn redact_sensitive_audit_keys(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                redact_sensitive_audit_keys(value);
            }
        }
        Value::Object(object) => {
            object.retain(|key, _| {
                !matches!(
                    normalized_key(key).as_str(),
                    "attachmentid" | "relativepath" | "sha256"
                )
            });
            for value in object.values_mut() {
                redact_sensitive_audit_keys(value);
            }
        }
        _ => {}
    }
}

fn snapshot_is_private_ai_attachment(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.iter().any(|(key, value)| {
            normalized_key(key) == "entitytype"
                && value
                    .as_str()
                    .is_some_and(is_ai_managed_attachment_entity_type)
        })
    })
}

fn snapshot_is_private_ai_source_job(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.iter().any(|(key, value)| {
        normalized_key(key) == "idempotencykey"
            && value
                .as_str()
                .is_some_and(|key| key.starts_with("ai-source-import:"))
    }) || object.iter().any(|(key, value)| {
        normalized_key(key) == "result" && contains_normalized_key(value, "muriarcaisourcebinding")
    })
}

fn contains_normalized_key(value: &Value, expected: &str) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| contains_normalized_key(value, expected)),
        Value::Object(object) => object.iter().any(|(key, value)| {
            normalized_key(key) == expected || contains_normalized_key(value, expected)
        }),
        _ => false,
    }
}

fn redact_job_technical_payload(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                redact_job_technical_payload(value);
            }
        }
        Value::Object(object) => {
            object.retain(|key, _| {
                !matches!(
                    normalized_key(key).as_str(),
                    "idempotencykey" | "result" | "errorreport"
                )
            });
            for value in object.values_mut() {
                redact_job_technical_payload(value);
            }
        }
        _ => {}
    }
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn job_audit(after: Value) -> AuditEntry {
        AuditEntry {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            project_id: Some(Uuid::new_v4()),
            entity_type: EntityType::Job,
            entity_id: Uuid::new_v4(),
            action: AuditAction::Create,
            actor: Actor::system("MuriArc"),
            source: WriteSource::Ai,
            request_id: None,
            reason: None,
            before: None,
            after: Some(after),
            operation_code: "job.create".to_owned(),
            operation_version: 1,
            operation_params: json!({
                "result": {"must_not": "survive"},
                "entityType": "job",
            }),
            entity_name_snapshot: None,
            entity_revision: Some(1),
            occurred_at: Utc::now(),
        }
    }

    #[test]
    fn private_ai_source_jobs_are_detected_from_key_or_binding() {
        let prefixed = job_audit(json!({
            "idempotency_key": "ai-source-import:preview",
            "status": "awaiting_confirmation",
        }));
        assert!(is_private_ai_source_job_audit(&prefixed));

        let bound = job_audit(json!({
            "idempotencyKey": "legacy-key",
            "result": {
                "nested": {
                    "_muriarc_ai_source_binding": {"source_id": Uuid::new_v4()},
                },
            },
        }));
        assert!(is_private_ai_source_job_audit(&bound));

        let ordinary = job_audit(json!({
            "idempotency_key": "ordinary-export",
            "result": {"note": "_muriarc_ai_source_binding is only text"},
        }));
        assert!(!is_private_ai_source_job_audit(&ordinary));
    }

    #[test]
    fn public_job_audit_keeps_status_and_removes_technical_payloads() {
        let mut entry = job_audit(json!({
            "id": Uuid::new_v4(),
            "status": "failed",
            "progress_current": 1,
            "progress_total": 2,
            "idempotency_key": "ordinary-secret",
            "result": {"private": true},
            "error_report": {"message": "private"},
            "meta": {"revision": 1},
        }));

        redact_public_audit_entry(&mut entry);

        let after = entry.after.unwrap();
        assert_eq!(after["status"], "failed");
        assert_eq!(after["progress_current"], 1);
        assert_eq!(after["meta"]["revision"], 1);
        assert!(after.get("idempotency_key").is_none());
        assert!(after.get("result").is_none());
        assert!(after.get("error_report").is_none());
        assert!(entry.operation_params.get("result").is_none());
        assert_eq!(entry.operation_params["entityType"], "job");
    }

    #[test]
    fn ai_attachment_classifier_reports_management_not_privacy() {
        assert!(is_ai_managed_attachment_entity_type(
            "ai_conversation_source"
        ));
        assert!(is_ai_managed_attachment_entity_type("ai_private_image"));
        assert!(!is_ai_managed_attachment_entity_type("animal"));
    }

    #[test]
    fn ai_operational_and_configuration_entities_share_one_boundary() {
        for entity_type in [
            EntityType::AiPrivateImage,
            EntityType::AiConversationSource,
            EntityType::AiExtractionDraft,
            EntityType::AiConversation,
            EntityType::AiConversationMessage,
            EntityType::AiAutonomyGrant,
            EntityType::AiProviderSettings,
            EntityType::AiProviderEndpoint,
            EntityType::AiLabSettings,
            EntityType::ToolRun,
            EntityType::Approval,
        ] {
            assert!(
                is_ai_operational_or_configuration_entity_type(entity_type),
                "{} must stay behind the dedicated AI boundary",
                entity_type.as_str()
            );
        }
        for entity_type in [
            EntityType::Animal,
            EntityType::Attachment,
            EntityType::Measurement,
            EntityType::Job,
            EntityType::Provenance,
        ] {
            assert!(
                !is_ai_operational_or_configuration_entity_type(entity_type),
                "{} is formal business history",
                entity_type.as_str()
            );
        }
    }

    #[test]
    fn public_audit_boundary_filters_private_ai_source_and_operational_entities() {
        let mut private_attachment = job_audit(json!({
            "entity_type": "ai_conversation_source",
            "file_name": "private.csv",
        }));
        private_attachment.entity_type = EntityType::Attachment;
        let private_attachment_id = private_attachment.entity_id;
        let private_job = job_audit(json!({
            "idempotency_key": "ai-source-import:private",
            "status": "awaiting_confirmation",
        }));
        let private_job_id = private_job.entity_id;
        let ordinary = job_audit(json!({
            "idempotency_key": "ordinary-export",
            "status": "completed",
            "result": {"relative_path": "private/path"},
        }));
        let ordinary_id = ordinary.entity_id;
        let mut message = job_audit(json!({"content": "OWNER_CHAT_SENTINEL"}));
        message.entity_type = EntityType::AiConversationMessage;
        let message_id = message.entity_id;
        let mut tool_run = job_audit(json!({"output": "TOOL_OUTPUT_SENTINEL"}));
        tool_run.entity_type = EntityType::ToolRun;
        let tool_run_id = tool_run.entity_id;
        let mut approval = job_audit(json!({
            "requested_diff": {"statement": "APPROVAL_STATEMENT_SENTINEL"},
        }));
        approval.entity_type = EntityType::Approval;
        let approval_id = approval.entity_id;
        let mut provider = job_audit(json!({"api_key": "PROVIDER_CONFIG_SENTINEL"}));
        provider.entity_type = EntityType::AiProviderEndpoint;
        let provider_id = provider.entity_id;
        let mut entries = vec![
            private_attachment,
            private_job,
            message,
            tool_run,
            approval,
            provider,
            ordinary,
        ];

        protect_public_audit_entries(&mut entries);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entity_id, ordinary_id);
        assert_ne!(entries[0].entity_id, private_attachment_id);
        assert_ne!(entries[0].entity_id, private_job_id);
        assert_ne!(entries[0].entity_id, message_id);
        assert_ne!(entries[0].entity_id, tool_run_id);
        assert_ne!(entries[0].entity_id, approval_id);
        assert_ne!(entries[0].entity_id, provider_id);
        assert!(entries[0].after.as_ref().unwrap().get("result").is_none());
    }
}
