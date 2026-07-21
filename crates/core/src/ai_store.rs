use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AiConversation, AiConversationMessage, Approval, AuditContext, Measurement, StoreResult,
    ToolRun,
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
