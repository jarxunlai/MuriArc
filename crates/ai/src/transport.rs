use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use muriarc_core::{AiConversation, AiConversationMessageRole};

use crate::{
    ApprovalRequirement, AssistantResponse, AssistantUsage, Citation, DraftKind, DraftStatus,
    FieldChange, ToolRunTrace, WriteDraft,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationSummary {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: i64,
}

impl From<AiConversation> for AssistantConversationSummary {
    fn from(value: AiConversation) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            title: value.title,
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
    pub fn from_service(conversation_id: Uuid, response: AssistantResponse) -> Self {
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
                usage: response.usage,
            },
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
}
