use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{DomainError, RecordMeta, WriteSource};

pub const MAX_AI_CONVERSATION_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_AI_CONVERSATION_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversation {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub title: String,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiConversationMessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConversationMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub sequence: i64,
    pub role: AiConversationMessageRole,
    pub content: String,
    pub response: Option<Value>,
    pub meta: RecordMeta,
}

impl AiConversationMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: Uuid,
        lab_id: Uuid,
        project_id: Option<Uuid>,
        user_id: Uuid,
        sequence: i64,
        role: AiConversationMessageRole,
        content: impl Into<String>,
        response: Option<Value>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let message = Self {
            id: Uuid::new_v4(),
            conversation_id,
            lab_id,
            project_id,
            user_id,
            sequence,
            role,
            content: content.into(),
            response,
            meta: RecordMeta::new(now),
        };
        message.validate()?;
        Ok(message)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let response_valid = match self.role {
            AiConversationMessageRole::User => self.response.is_none(),
            AiConversationMessageRole::Assistant => self
                .response
                .as_ref()
                .is_some_and(serde_json::Value::is_object),
        };
        let payload_size = self
            .response
            .as_ref()
            .and_then(|value| serde_json::to_vec(value).ok())
            .map_or(0, |bytes| bytes.len());
        if self.id.is_nil()
            || self.conversation_id.is_nil()
            || self.lab_id.is_nil()
            || self.user_id.is_nil()
            || self.sequence <= 0
            || self.content.trim().is_empty()
            || self.content.len() > MAX_AI_CONVERSATION_MESSAGE_BYTES
            || !response_valid
            || payload_size > MAX_AI_CONVERSATION_PAYLOAD_BYTES
        {
            Err(DomainError::InvalidAiConversationMessage)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRunStatus {
    Pending,
    AwaitingApproval,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRun {
    pub id: Uuid,
    pub conversation_id: Option<Uuid>,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub tool_name: String,
    pub input: Value,
    pub output: Option<Value>,
    pub status: ToolRunStatus,
    pub source: WriteSource,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub id: Uuid,
    pub tool_run_id: Uuid,
    pub requested_diff: Value,
    pub decision: ApprovalDecision,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Import,
    Export,
    Snapshot,
    BulkOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Parsing,
    Validating,
    AwaitingConfirmation,
    Writing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub created_by: Uuid,
    pub kind: JobKind,
    pub status: JobStatus,
    pub idempotency_key: String,
    pub progress_current: i64,
    pub progress_total: Option<i64>,
    pub result: Option<Value>,
    pub error_report: Option<Value>,
    pub cancellation_requested: bool,
    pub meta: RecordMeta,
}
