use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use muriarc_core::{
    AiAutonomyMode, AiConversation, AiConversationMessageRole, AiConversationSourceRef,
};

use crate::{
    ApprovalRequirement, AssistantIncompleteReason, AssistantResponse, AssistantUsage, Citation,
    ContextManagementTrace, DraftKind, DraftStatus, FieldChange, ImportCommitDraftPayload,
    ImportDraftPreviewSummary, ToolRunTrace, WriteDraft,
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
    pub source_refs: Vec<Uuid>,
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
    /// Absent for complete responses and for responses persisted by older
    /// MuriArc versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<AssistantIncompleteReason>,
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
    pub pinned_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
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
            pinned_at: value.pinned_at,
            archived_at: value.archived_at,
            created_at: value.meta.created_at,
            updated_at: value.meta.updated_at,
            revision: value.meta.revision,
        }
    }
}

/// Safe historical source metadata returned to renderers.
///
/// `attachment_id` remains part of the internal persisted source snapshot for
/// integrity checks, but it is deliberately omitted from this public DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConversationSourceRef {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub size_bytes: i64,
}

impl From<AiConversationSourceRef> for AssistantConversationSourceRef {
    fn from(value: AiConversationSourceRef) -> Self {
        Self {
            source_id: value.source_id,
            source_revision: value.source_revision,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<AssistantConversationSourceRef>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteDraftSummary {
    pub id: Uuid,
    pub kind: DraftKind,
    pub project_id: Option<Uuid>,
    pub changes: Vec<FieldChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_preview: Option<ImportDraftPreviewSummary>,
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
            import_preview: (draft.kind() == DraftKind::BulkImport)
                .then(|| {
                    serde_json::from_value::<ImportCommitDraftPayload>(draft.payload().clone())
                        .ok()
                        .map(|payload| payload.preview)
                })
                .flatten(),
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
    ) -> Self {
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
                context: response.context,
            },
            incomplete_reason: response.incomplete_reason,
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
    fn stored_turn_without_incomplete_reason_remains_deserializable() {
        let value = serde_json::json!({
            "conversationId": Uuid::new_v4(),
            "content": "complete legacy response",
            "citations": [],
            "toolRuns": [],
            "drafts": [],
            "trace": {
                "providerId": "legacy-provider",
                "model": "legacy-model",
                "usage": {
                    "provider_calls": 1,
                    "tool_calls": 0,
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15
                },
                "context": {
                    "estimatedInputTokens": 10,
                    "inputTokenCountIsEstimate": true,
                    "contextTrimmed": false,
                    "trimmedHistoryTurns": 0
                }
            }
        });

        let response: AssistantTurnResponse = serde_json::from_value(value).unwrap();

        assert_eq!(response.incomplete_reason, None);
        let serialized = serde_json::to_value(response).unwrap();
        assert!(serialized.get("incompleteReason").is_none());
        assert_eq!(
            serde_json::to_value(AssistantIncompleteReason::IterationLimitExceeded).unwrap(),
            "iteration_limit_exceeded"
        );
        assert_eq!(
            serde_json::to_value(AssistantIncompleteReason::ToolCallLimitExceeded).unwrap(),
            "tool_call_limit_exceeded"
        );
    }

    #[test]
    fn public_historical_source_refs_hide_internal_attachment_identity() {
        let source_id = Uuid::new_v4();
        let public = AssistantConversationSourceRef::from(AiConversationSourceRef {
            source_id,
            source_revision: 3,
            attachment_id: Uuid::new_v4(),
            file_name: "weights.csv".to_owned(),
            media_type: Some("text/csv".to_owned()),
            size_bytes: 42,
        });

        let serialized = serde_json::to_value(public).unwrap();
        assert_eq!(serialized["sourceId"], source_id.to_string());
        assert!(serialized.get("attachmentId").is_none());
        assert!(serialized.get("sha256").is_none());
    }

    #[test]
    fn bulk_import_summary_exposes_the_persisted_bounded_preview() {
        let now = Utc::now();
        let project_id = Uuid::new_v4();
        let preview = ImportDraftPreviewSummary {
            import_kind: crate::AiSourceImportKind::Measurement,
            project_id,
            experiment_id: Uuid::new_v4(),
            file_name: "measurements.csv".to_owned(),
            sheet_name: "Sheet1".to_owned(),
            total_rows: 1,
            accepted_rows: 1,
            issue_count: 0,
            issues_truncated: false,
            can_confirm: true,
            preview_rows: vec![crate::ImportDraftPreviewRow {
                row_number: 2,
                animal_id: Uuid::new_v4(),
                animal_display_id: "M-001".to_owned(),
                measurement_key: "body_weight".to_owned(),
                value: "22.4".to_owned(),
                unit: Some("g".to_owned()),
                measured_at: "2026-07-23T08:00:00Z".to_owned(),
            }],
            preview_rows_truncated: false,
            issues: Vec::new(),
        };
        let draft = WriteDraft::new(
            DraftKind::BulkImport,
            crate::ToolName::ImportCommitDraft,
            crate::ProposalActor::Ai {
                user_id: Uuid::new_v4(),
                tool_run_id: Uuid::new_v4(),
            },
            Some(project_id),
            vec![FieldChange {
                path: "/data/imports/test".to_owned(),
                before: Some(serde_json::json!({"status": "awaiting_confirmation"})),
                after: Some(serde_json::json!({"status": "completed"})),
            }],
            serde_json::to_value(ImportCommitDraftPayload {
                operation: ImportCommitDraftPayload::OPERATION.to_owned(),
                job_id: Uuid::new_v4(),
                preview_hash: "a".repeat(64),
                expected_revision: 2,
                preview: preview.clone(),
            })
            .unwrap(),
            now,
            now + chrono::Duration::hours(1),
        )
        .unwrap();

        let summary = WriteDraftSummary::from(&draft);
        assert_eq!(summary.import_preview, Some(preview));
        let serialized = serde_json::to_value(summary).unwrap();
        assert_eq!(
            serialized["importPreview"]["previewRows"][0]["measurementKey"],
            "body_weight"
        );
        assert!(serialized["importPreview"].get("preview_rows").is_none());
    }
}
