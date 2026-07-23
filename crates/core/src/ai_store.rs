use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AiAutonomyGrant, AiConversation, AiConversationMessage, AiModelProfile,
    AiModelProfileSecretRef, AiModelProfileVersion, AiUserModelDefaults, Approval, AuditContext,
    Measurement, StoreResult, ToolRun,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversationFilter {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiApprovalFilter {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub project_id: Option<Uuid>,
    pub decision: Option<crate::ApprovalDecision>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModelProfileFilter {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub include_archived: bool,
}

#[async_trait]
pub trait AiModelProfileStore: Send + Sync {
    /// Creates the editable profile identity and immutable version 1 in one
    /// transaction, together with a redacted audit entry.
    async fn create_ai_model_profile(
        &self,
        profile: &AiModelProfile,
        version: &AiModelProfileVersion,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn get_ai_model_profile(&self, id: Uuid) -> StoreResult<AiModelProfile>;

    async fn list_ai_model_profiles(
        &self,
        filter: &AiModelProfileFilter,
    ) -> StoreResult<Vec<AiModelProfile>>;

    async fn get_ai_model_profile_version(
        &self,
        profile_id: Uuid,
        version: i64,
    ) -> StoreResult<AiModelProfileVersion>;

    /// Appends one immutable version and advances the profile projection using
    /// optimistic revision checking.
    async fn append_ai_model_profile_version(
        &self,
        profile: &AiModelProfile,
        version: &AiModelProfileVersion,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    /// Soft-archives one profile and atomically clears any conversation or
    /// vision default that still references it.
    async fn archive_ai_model_profile(
        &self,
        profile: &AiModelProfile,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn save_ai_user_model_defaults(
        &self,
        defaults: &AiUserModelDefaults,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn get_ai_user_model_defaults(
        &self,
        user_id: Uuid,
    ) -> StoreResult<Option<AiUserModelDefaults>>;
}

/// Desktop-only metadata boundary for exact immutable profile-version Keyring
/// bindings. Implementations persist only redacted state; secret bytes remain
/// exclusively in the operating-system credential store.
#[async_trait]
pub trait AiModelProfileSecretRefStore: Send + Sync {
    async fn get_ai_model_profile_secret_ref(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> StoreResult<Option<AiModelProfileSecretRef>>;

    async fn list_ai_model_profile_secret_refs(
        &self,
        profile_id: Uuid,
    ) -> StoreResult<Vec<AiModelProfileSecretRef>>;

    /// Creates or updates one exact-version binding with optimistic revision
    /// checking and writes its redacted audit entry in the same transaction.
    async fn save_ai_model_profile_secret_ref(
        &self,
        value: &AiModelProfileSecretRef,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    /// Atomically marks every existing exact-version binding for one profile
    /// as revoked. Implementations write one redacted audit entry per changed
    /// binding and return all bindings in their final state.
    async fn revoke_ai_model_profile_secret_refs(
        &self,
        profile_id: Uuid,
        revoked_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Vec<AiModelProfileSecretRef>>;
}

/// Persistence boundary for AI orchestration records.
///
/// It is separate from the ordinary domain Store so a model-facing executor
/// can remain read-only. Only application services receive this port. Every
/// method writes its audit record in the same transaction as the entity.
#[async_trait]
pub trait AiOperationStore: Send + Sync {
    async fn create_ai_conversation(
        &self,
        conversation: &AiConversation,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    /// Atomically creates one writable, immutable-model-bound conversation
    /// together with its initial conversation-scoped autonomy grant and both
    /// redacted audit entries.
    async fn create_ai_conversation_with_autonomy(
        &self,
        conversation: &AiConversation,
        grant: &AiAutonomyGrant,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn get_ai_conversation(&self, id: Uuid) -> StoreResult<AiConversation>;

    async fn list_ai_conversations(
        &self,
        filter: &AiConversationFilter,
        offset: u32,
        limit: u32,
    ) -> StoreResult<Vec<AiConversation>>;

    /// Atomically appends exactly one user/assistant pair and advances the
    /// parent conversation revision. `expected_last_sequence` prevents two
    /// concurrent turns from silently interleaving stale histories.
    async fn append_ai_turn_messages(
        &self,
        user_message: &AiConversationMessage,
        assistant_message: &AiConversationMessage,
        expected_last_sequence: i64,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation>;

    /// Returns the newest bounded slice in chronological order.
    async fn list_ai_conversation_messages(
        &self,
        conversation_id: Uuid,
        limit: u32,
    ) -> StoreResult<Vec<AiConversationMessage>>;

    async fn get_ai_autonomy_grant(
        &self,
        conversation_id: Uuid,
    ) -> StoreResult<Option<AiAutonomyGrant>>;

    /// Creates or replaces the conversation grant with optimistic revision
    /// checking and a formal audit entry in the same transaction.
    async fn save_ai_autonomy_grant(
        &self,
        grant: &AiAutonomyGrant,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn create_tool_run(&self, tool_run: &ToolRun, audit: &AuditContext) -> StoreResult<()>;

    async fn get_tool_run(&self, id: Uuid) -> StoreResult<ToolRun>;

    async fn create_approval(&self, approval: &Approval, audit: &AuditContext) -> StoreResult<()>;

    async fn get_approval(&self, id: Uuid) -> StoreResult<Approval>;

    async fn list_approvals(&self, filter: &AiApprovalFilter) -> StoreResult<Vec<Approval>>;

    /// Atomically records a resolved non-measurement draft and its tool-run
    /// state. Valid projections are rejected/cancelled, approved/completed,
    /// and approved/failed. The failed form records a human-approved operation
    /// that did not apply; its serialized draft must remain Approved rather
    /// than Applied.
    #[allow(clippy::too_many_arguments)]
    async fn finalize_ai_draft(
        &self,
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    /// Atomically applies one already human-approved AI measurement draft,
    /// updates the approval/tool-run projections, and writes all audit rows.
    /// The inserted Measurement must remain `draft`; signing is a later,
    /// explicit researcher action through the normal domain use case.
    #[allow(clippy::too_many_arguments)]
    async fn apply_ai_measurement_draft(
        &self,
        measurement: &Measurement,
        expected_animal_revision: i64,
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;
}
