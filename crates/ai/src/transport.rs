use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use muriarc_core::{
    AiAutonomyMode, AiConversation, AiConversationMessageRole, AiModelProfileBinding,
};

use crate::{
    ApprovalRequirement, AssistantResponse, AssistantUsage, Citation, ContextManagementTrace,
    DraftKind, DraftStatus, FieldChange, ToolRunTrace, WriteDraft,
};

/// Stable request contract shared by Tauri commands and `/api/v1/ai/turns`.
/// Identity, lab and effective scopes are always supplied by the transport and
/// are deliberately absent from this untrusted client payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantTurnRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_model_profile_id: Option<Uuid>,
}

/// Starts an auditable empty conversation before the first Provider call.
///
/// The transport resolves `model_profile_id` to the profile's current
/// immutable version. Passwords, Desktop startup declarations and native
/// session identifiers deliberately live outside this shared payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationStartRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    pub requested_mode: AiAutonomyMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationStartResponse {
    pub conversation: AssistantConversationSummary,
    pub autonomy: AiAutonomyView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantTurnResponse {
    pub conversation_id: Uuid,
    pub content: String,
    pub citations: Vec<Citation>,
    pub tool_runs: Vec<ToolRunTrace>,
    pub drafts: Vec<WriteDraftSummary>,
    pub trace: AssistantTrace,
    #[serde(default)]
    pub autonomy: AiAutonomyView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiAutonomyView {
    pub mode: AiAutonomyMode,
    pub effective_mode: AiAutonomyMode,
    pub max_mode: AiAutonomyMode,
    pub batch_limit: u32,
    pub revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub requires_human_approval: Vec<String>,
}

impl Default for AiAutonomyView {
    fn default() -> Self {
        Self {
            mode: AiAutonomyMode::Ask,
            effective_mode: AiAutonomyMode::Ask,
            max_mode: AiAutonomyMode::Full,
            batch_limit: AiAutonomyMode::Ask.batch_limit(),
            revision: 0,
            expires_at: None,
            requires_human_approval: hard_boundaries(),
        }
    }
}

pub fn hard_boundaries() -> Vec<String> {
    [
        "research_signature",
        "animal_transfer_or_death",
        "delete_or_bulk_import",
        "permissions_and_accounts",
        "audit_or_log_cleanup",
        "breeding_scientific_facts",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiAutonomyUpdateRequest {
    pub mode: AiAutonomyMode,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationSummary {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<AiConversationReadOnlyReason>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiConversationReadOnlyReason {
    LegacyModelUnknown,
    ModelArchived,
    ModelUnavailable,
}

impl From<AiConversation> for AssistantConversationSummary {
    fn from(value: AiConversation) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            title: value.title,
            model_profile_id: value.model_profile.map(|binding| binding.profile_id),
            model_profile_version: value.model_profile.map(|binding| binding.profile_version),
            model_profile_name: None,
            model_id: None,
            read_only: value.legacy_read_only,
            read_only_reason: value
                .legacy_read_only
                .then_some(AiConversationReadOnlyReason::LegacyModelUnknown),
            created_at: value.meta.created_at,
            updated_at: value.meta.updated_at,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationMessage {
    pub id: Uuid,
    pub sequence: i64,
    pub role: AiConversationMessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<AssistantTurnResponse>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationDetail {
    pub conversation: AssistantConversationSummary,
    pub messages: Vec<AssistantConversationMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantTrace {
    pub provider_id: String,
    pub model: String,
    pub usage: AssistantUsage,
    pub context: ContextManagementTrace,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_calls: Vec<AssistantModelCallTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_evidence: Vec<AssistantImageEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantModelCallPurpose {
    FinalAnswer,
    VisionAndFinal,
    VisionObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantModelCallTrace {
    pub purpose: AssistantModelCallPurpose,
    pub model_profile_id: Uuid,
    pub model_profile_version: i64,
    pub provider_id: String,
    pub model: String,
    pub usage: AssistantUsage,
}

impl AssistantModelCallTrace {
    pub fn new(
        purpose: AssistantModelCallPurpose,
        binding: AiModelProfileBinding,
        provider_id: impl Into<String>,
        model: impl Into<String>,
        usage: AssistantUsage,
    ) -> Self {
        Self {
            purpose,
            model_profile_id: binding.profile_id,
            model_profile_version: binding.profile_version,
            provider_id: provider_id.into(),
            model: model.into(),
            usage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantImageEvidence {
    pub image_id: Uuid,
    /// SHA-256 of the verified, metadata-sanitized bytes sent to the Provider.
    pub sanitized_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteDraftSummary {
    pub id: Uuid,
    pub kind: DraftKind,
    pub project_id: Option<Uuid>,
    pub changes: Vec<FieldChange>,
    pub requirement: ApprovalRequirement,
    pub status: DraftStatus,
    pub revision: u64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl From<&WriteDraft> for WriteDraftSummary {
    fn from(draft: &WriteDraft) -> Self {
        Self {
            id: draft.id(),
            kind: draft.kind(),
            project_id: draft.project_id(),
            changes: draft.changes().to_vec(),
            requirement: draft.requirement(),
            status: draft.status(),
            revision: draft.revision(),
            created_at: draft.created_at(),
            expires_at: draft.expires_at(),
        }
    }
}

impl AssistantTurnResponse {
    pub fn from_service(
        conversation_id: Uuid,
        response: AssistantResponse,
        autonomy: AiAutonomyView,
        model_profile: AiModelProfileBinding,
        purpose: AssistantModelCallPurpose,
        mut prior_model_calls: Vec<AssistantModelCallTrace>,
        image_evidence: Vec<AssistantImageEvidence>,
    ) -> Self {
        let mut aggregate_usage = response.usage;
        for call in &prior_model_calls {
            aggregate_usage.provider_calls = aggregate_usage
                .provider_calls
                .saturating_add(call.usage.provider_calls);
            aggregate_usage.tool_calls = aggregate_usage
                .tool_calls
                .saturating_add(call.usage.tool_calls);
            aggregate_usage.input_tokens = aggregate_usage
                .input_tokens
                .saturating_add(call.usage.input_tokens);
            aggregate_usage.output_tokens = aggregate_usage
                .output_tokens
                .saturating_add(call.usage.output_tokens);
            aggregate_usage.total_tokens = aggregate_usage
                .total_tokens
                .saturating_add(call.usage.total_tokens);
        }
        prior_model_calls.push(AssistantModelCallTrace::new(
            purpose,
            model_profile,
            response.provider_id.clone(),
            response.model.clone(),
            response.usage,
        ));
        Self {
            conversation_id,
            content: response.content,
            citations: response.citations,
            tool_runs: response.tool_runs,
            drafts: response
                .drafts
                .iter()
                .map(WriteDraftSummary::from)
                .collect(),
            trace: AssistantTrace {
                provider_id: response.provider_id,
                model: response.model,
                usage: aggregate_usage,
                context: response.context,
                model_calls: prior_model_calls,
                image_evidence,
            },
            autonomy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftDecisionRequest {
    pub expected_revision: u64,
    pub decision: crate::ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
    #[serde(default)]
    pub step_up_verified: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_identity_and_raw_sql_fields() {
        let value = serde_json::json!({
            "message": "list animals",
            "userId": Uuid::new_v4(),
            "sql": "select * from animals"
        });
        assert!(serde_json::from_value::<AssistantTurnRequest>(value).is_err());
    }

    #[test]
    fn shared_conversation_start_rejects_transport_security_proofs() {
        for forbidden in [
            ("currentPassword", serde_json::json!("secret")),
            ("declared", serde_json::json!(true)),
            ("sessionId", serde_json::json!(Uuid::new_v4())),
            ("stepUpVerified", serde_json::json!(true)),
            ("modelProfileId", serde_json::json!(Uuid::new_v4())),
        ] {
            let mut value = serde_json::json!({
                "projectId": null,
                "requestedMode": "full",
            });
            value
                .as_object_mut()
                .unwrap()
                .insert(forbidden.0.to_owned(), forbidden.1);
            assert!(
                serde_json::from_value::<AssistantConversationStartRequest>(value).is_err(),
                "{} must remain transport-native",
                forbidden.0
            );
        }
    }
}
