use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AiAutonomyGrant, AiConversation, AiConversationMessage, Approval, AuditContext, Cohort,
    Measurement, Participation, StoreResult, ToolRun,
};

/// Fully normalized application payload for one approved experiment grouping
/// draft. IDs are generated when the draft is created; adapters must recheck
/// every optimistic revision and relationship before inserting any row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiExperimentGroupingApplication {
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub expected_project_revision: i64,
    pub experiment_id: Uuid,
    pub expected_experiment_revision: i64,
    pub input_snapshot_sha256: String,
    pub cohorts: Vec<Cohort>,
    pub participations: Vec<Participation>,
    pub expected_animal_revisions: Vec<AiGroupingAnimalRevision>,
    /// Present when the plan balanced by weight. One entry exists for every
    /// candidate, including animals that had no weight at preview time.
    #[serde(default)]
    pub expected_latest_weights: Vec<AiGroupingLatestWeightRevision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiGroupingAnimalRevision {
    pub animal_id: Uuid,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiGroupingLatestWeightRevision {
    pub animal_id: Uuid,
    pub measurement_id: Option<Uuid>,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiConversationArchiveFilter {
    #[default]
    Active,
    Archived,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversationFilter {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub project_id: Option<Uuid>,
    pub title_query: Option<String>,
    #[serde(default)]
    pub archive: AiConversationArchiveFilter,
    #[serde(default = "default_pinned_first")]
    pub pinned_first: bool,
}

const fn default_pinned_first() -> bool {
    true
}

impl Default for AiConversationFilter {
    fn default() -> Self {
        Self {
            lab_id: Uuid::nil(),
            user_id: Uuid::nil(),
            project_id: None,
            title_query: None,
            archive: AiConversationArchiveFilter::Active,
            pinned_first: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiConversationChange {
    Rename { title: String },
    Pin,
    Unpin,
    Archive,
    Unarchive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversationUpdate {
    pub id: Uuid,
    pub expected_revision: i64,
    pub change: AiConversationChange,
    pub updated_at: DateTime<Utc>,
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

    /// Applies exactly one user-visible metadata transition using optimistic
    /// revision checking and writes its formal audit row in the same
    /// transaction. Archiving is a reversible state and never hard-deletes
    /// the conversation or its messages.
    async fn update_ai_conversation(
        &self,
        update: &AiConversationUpdate,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation>;

    /// Atomically persists every durable record produced by one assistant
    /// turn: tool traces, pending approvals, the user/assistant message pair,
    /// and the parent conversation revision.
    ///
    /// A stale sequence, invalid approval binding, or any insert/audit failure
    /// rolls back the entire turn so a retry never observes orphan ToolRuns or
    /// approvals without their conversation messages.
    #[allow(clippy::too_many_arguments)]
    async fn append_ai_turn_records(
        &self,
        user_message: &AiConversationMessage,
        assistant_message: &AiConversationMessage,
        tool_runs: &[ToolRun],
        approvals: &[Approval],
        expected_last_sequence: i64,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation>;

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

    /// Atomically revalidates and applies one human-signed deterministic
    /// experiment grouping plan. Cohorts, participations with current Genetics
    /// v2 snapshots, animal events, audit/provenance, approval and tool-run
    /// resolution either all commit or all roll back.
    #[allow(clippy::too_many_arguments)]
    async fn apply_ai_experiment_grouping_draft(
        &self,
        application: &AiExperimentGroupingApplication,
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<Vec<Participation>>;
}
