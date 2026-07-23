use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::{DomainError, RecordMeta, WriteSource};

pub const MAX_AI_CONVERSATION_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_AI_CONVERSATION_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_AI_CONVERSATION_SOURCE_REFS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversation {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub pinned_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

impl AiConversation {
    pub fn apply_change(
        &mut self,
        change: &crate::AiConversationChange,
        updated_at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if updated_at < self.meta.updated_at {
            return Err(DomainError::InvalidAiConversation);
        }
        match change {
            crate::AiConversationChange::Rename { title } => {
                let title = title.trim();
                if title.is_empty()
                    || title.chars().count() > 256
                    || title.chars().any(char::is_control)
                {
                    return Err(DomainError::InvalidAiConversation);
                }
                self.title = title.to_owned();
            }
            crate::AiConversationChange::Pin => {
                if self.pinned_at.is_some() {
                    return Err(DomainError::InvalidAiConversation);
                }
                self.pinned_at = Some(updated_at);
            }
            crate::AiConversationChange::Unpin => {
                if self.pinned_at.is_none() {
                    return Err(DomainError::InvalidAiConversation);
                }
                self.pinned_at = None;
            }
            crate::AiConversationChange::Archive => {
                if self.archived_at.is_some() {
                    return Err(DomainError::InvalidAiConversation);
                }
                self.archived_at = Some(updated_at);
            }
            crate::AiConversationChange::Unarchive => {
                if self.archived_at.is_none() {
                    return Err(DomainError::InvalidAiConversation);
                }
                self.archived_at = None;
            }
        }
        self.meta.touch(updated_at);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiAutonomyMode {
    #[default]
    Ask,
    Auto,
    Full,
}

impl AiAutonomyMode {
    pub const fn batch_limit(self) -> u32 {
        match self {
            Self::Ask => 1,
            Self::Auto => 20,
            Self::Full => 100,
        }
    }

    pub const fn min(self, ceiling: Self) -> Self {
        if self as u8 <= ceiling as u8 {
            self
        } else {
            ceiling
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiActionCategory {
    Read,
    Artifact,
    ReversibleDraft,
}

/// A revocable, conversation-scoped delegation grant. This is not a role and
/// never expands the holder's ordinary project or laboratory permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiAutonomyGrant {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub mode: AiAutonomyMode,
    pub allowed_categories: Vec<AiActionCategory>,
    pub batch_limit: u32,
    pub step_up_verified_at: Option<DateTime<Utc>>,
    pub last_used_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

impl AiAutonomyGrant {
    pub fn effective_mode(&self, now: DateTime<Utc>, session_id: Option<Uuid>) -> AiAutonomyMode {
        if self.revoked_at.is_some()
            || self.expires_at.is_some_and(|expires_at| expires_at <= now)
            || (self.mode == AiAutonomyMode::Full
                && self.session_id.is_some()
                && self.session_id != session_id)
        {
            AiAutonomyMode::Ask
        } else {
            self.mode
        }
    }
}

#[cfg(test)]
mod autonomy_tests {
    use chrono::Duration;

    use super::*;

    fn grant(now: DateTime<Utc>, session_id: Option<Uuid>) -> AiAutonomyGrant {
        AiAutonomyGrant {
            id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            project_id: Some(Uuid::new_v4()),
            user_id: Uuid::new_v4(),
            session_id,
            mode: AiAutonomyMode::Full,
            allowed_categories: vec![AiActionCategory::Read],
            batch_limit: 100,
            step_up_verified_at: Some(now),
            last_used_at: now,
            expires_at: Some(now + Duration::minutes(30)),
            revoked_at: None,
            meta: RecordMeta::new(now),
        }
    }

    #[test]
    fn full_autonomy_is_bound_to_its_session_and_idle_window() {
        let now = Utc::now();
        let session_id = Uuid::new_v4();
        let grant = grant(now, Some(session_id));

        assert_eq!(
            grant.effective_mode(now, Some(session_id)),
            AiAutonomyMode::Full
        );
        assert_eq!(
            grant.effective_mode(now, Some(Uuid::new_v4())),
            AiAutonomyMode::Ask
        );
        assert_eq!(
            grant.effective_mode(now + Duration::minutes(30), Some(session_id)),
            AiAutonomyMode::Ask
        );
    }

    #[test]
    fn revoked_autonomy_always_falls_back_to_ask() {
        let now = Utc::now();
        let mut grant = grant(now, None);
        grant.revoked_at = Some(now);

        assert_eq!(grant.effective_mode(now, None), AiAutonomyMode::Ask);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiConversationMessageRole {
    User,
    Assistant,
}

/// Immutable, display-safe metadata captured from a trusted source resolver for
/// one user turn. Object paths and content hashes deliberately stay behind the
/// storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConversationSourceRef {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub attachment_id: Uuid,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
}

impl AiConversationSourceRef {
    pub fn validate(&self) -> Result<(), DomainError> {
        let file_name = self.file_name.as_str();
        let media_type_valid = self.media_type.as_ref().is_none_or(|media_type| {
            media_type == media_type.trim()
                && !media_type.is_empty()
                && media_type.len() <= 127
                && media_type.is_ascii()
                && media_type.bytes().all(|byte| byte.is_ascii_graphic())
                && media_type.contains('/')
        });
        if self.source_id.is_nil()
            || self.source_revision <= 0
            || self.attachment_id.is_nil()
            || file_name != file_name.trim()
            || file_name.is_empty()
            || file_name.len() > 255
            || file_name.chars().any(char::is_control)
            || file_name.contains('/')
            || file_name.contains('\\')
            || !media_type_valid
            || self.size_bytes < 0
        {
            Err(DomainError::InvalidAiConversationMessage)
        } else {
            Ok(())
        }
    }
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
    #[serde(default)]
    pub source_refs: Vec<AiConversationSourceRef>,
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
            source_refs: Vec::new(),
            meta: RecordMeta::new(now),
        };
        message.validate()?;
        Ok(message)
    }

    pub fn with_source_refs(
        mut self,
        source_refs: Vec<AiConversationSourceRef>,
    ) -> Result<Self, DomainError> {
        self.source_refs = source_refs;
        self.validate()?;
        Ok(self)
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
        let source_refs_valid = self.source_refs.len() <= MAX_AI_CONVERSATION_SOURCE_REFS
            && self
                .source_refs
                .iter()
                .all(|source_ref| source_ref.validate().is_ok())
            && self
                .source_refs
                .iter()
                .map(|source_ref| source_ref.source_id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.source_refs.len()
            && self
                .source_refs
                .iter()
                .map(|source_ref| source_ref.attachment_id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.source_refs.len()
            && (self.role == AiConversationMessageRole::User || self.source_refs.is_empty());
        if self.id.is_nil()
            || self.conversation_id.is_nil()
            || self.lab_id.is_nil()
            || self.user_id.is_nil()
            || self.sequence <= 0
            || self.content.trim().is_empty()
            || self.content.len() > MAX_AI_CONVERSATION_MESSAGE_BYTES
            || !response_valid
            || payload_size > MAX_AI_CONVERSATION_PAYLOAD_BYTES
            || !source_refs_valid
        {
            Err(DomainError::InvalidAiConversationMessage)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod conversation_message_tests {
    use super::*;

    fn source_ref(id: u128) -> AiConversationSourceRef {
        AiConversationSourceRef {
            source_id: Uuid::from_u128(id),
            source_revision: 1,
            attachment_id: Uuid::from_u128(id + 100),
            file_name: format!("source-{id}.csv"),
            media_type: Some("text/csv".to_owned()),
            size_bytes: 42,
        }
    }

    fn message(role: AiConversationMessageRole) -> AiConversationMessage {
        AiConversationMessage::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            Uuid::new_v4(),
            1,
            role,
            "inspect sources",
            (role == AiConversationMessageRole::Assistant)
                .then(|| serde_json::json!({"content": "done"})),
            Utc::now(),
        )
        .unwrap()
    }

    #[test]
    fn user_source_refs_are_validated_and_keep_selection_order() {
        let refs = vec![source_ref(2), source_ref(1)];
        let user_message = message(AiConversationMessageRole::User)
            .with_source_refs(refs.clone())
            .unwrap();
        assert_eq!(user_message.source_refs, refs);

        let mut unsafe_name = source_ref(3);
        unsafe_name.file_name = "../animals.csv".to_owned();
        assert!(
            message(AiConversationMessageRole::User)
                .with_source_refs(vec![unsafe_name])
                .is_err()
        );

        let duplicate_source = vec![source_ref(4), {
            let mut duplicate = source_ref(5);
            duplicate.source_id = Uuid::from_u128(4);
            duplicate
        }];
        assert!(
            message(AiConversationMessageRole::User)
                .with_source_refs(duplicate_source)
                .is_err()
        );
    }

    #[test]
    fn assistant_messages_cannot_claim_source_refs() {
        assert!(
            message(AiConversationMessageRole::Assistant)
                .with_source_refs(vec![source_ref(1)])
                .is_err()
        );
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
