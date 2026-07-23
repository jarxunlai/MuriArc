use std::{collections::BTreeSet, sync::Arc};

use chrono::{Duration, Utc};
use muriarc_application::{ExperimentGroupingPlan, RESEARCH_GROUPING_SCHEMA_VERSION};
use muriarc_core::{
    Actor, ActorType, AiActionCategory, AiApprovalFilter, AiAutonomyGrant, AiAutonomyMode,
    AiConversation, AiConversationArchiveFilter, AiConversationChange, AiConversationFilter,
    AiConversationMessage, AiConversationMessageRole, AiConversationSourceRef,
    AiConversationUpdate, AiExperimentGroupingApplication, AiImportResolution, AiOperationStore,
    Approval, ApprovalDecision as StoredApprovalDecision, AuditContext, Measurement, MuriArcStore,
    RecordMeta, StoreError, ToolRun, ToolRunStatus, WriteSource, portable_storage_timestamp,
};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiDataAccessContext, AiDataToolBackend,
    AiProvider, ApprovalDecision, ApprovalError, AssistantConfigError, AssistantConversationDetail,
    AssistantConversationMessage, AssistantConversationSummary, AssistantError, AssistantRequest,
    AssistantRuntimeConfig, AssistantService, AssistantSourceBundle, AssistantSourceError,
    AssistantSourceResolutionRequest, AssistantSourceResolver, AssistantTurnRequest,
    AssistantTurnResponse, ChatMessage, DraftDecisionRequest, DraftKind, DraftStatus,
    HumanApprover, MAX_ASSISTANT_SOURCES, ProposalActor, ProviderCredentials,
    StoreDomainToolExecutor, StoreToolAccessContext, ToolExecutionError, ToolName, WriteDraft,
    WriteDraftSummary,
};

const PROVIDER_HISTORY_LIMIT: u32 = 200;
const CONVERSATION_LIST_LIMIT: u32 = 100;
const CONVERSATION_DETAIL_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
pub struct AiExecutionContext {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub user_display_name: String,
    pub request_id: String,
    allowed_project_ids: BTreeSet<Uuid>,
    writable_project_ids: BTreeSet<Uuid>,
    importable_project_ids: BTreeSet<Uuid>,
    exportable_project_ids: BTreeSet<Uuid>,
    lab_registry_read: bool,
    read_activity: bool,
    read_audit: bool,
    access_grant: AccessGrant,
    session_id: Option<Uuid>,
    max_autonomy_mode: AiAutonomyMode,
}

impl AiExecutionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lab_id: Uuid,
        user_id: Uuid,
        user_display_name: impl Into<String>,
        request_id: impl Into<String>,
        allowed_project_ids: impl IntoIterator<Item = Uuid>,
        writable_project_ids: impl IntoIterator<Item = Uuid>,
        lab_registry_read: bool,
        access_grant: AccessGrant,
    ) -> Self {
        let allowed_project_ids = allowed_project_ids.into_iter().collect::<BTreeSet<_>>();
        let writable_project_ids = writable_project_ids
            .into_iter()
            .filter(|project_id| allowed_project_ids.contains(project_id))
            .collect();
        Self {
            lab_id,
            user_id,
            user_display_name: user_display_name.into(),
            request_id: request_id.into(),
            allowed_project_ids,
            writable_project_ids,
            importable_project_ids: BTreeSet::new(),
            exportable_project_ids: BTreeSet::new(),
            lab_registry_read,
            read_activity: false,
            read_audit: false,
            access_grant,
            session_id: None,
            max_autonomy_mode: AiAutonomyMode::Full,
        }
    }

    pub const fn with_autonomy_context(
        mut self,
        session_id: Option<Uuid>,
        max_mode: AiAutonomyMode,
    ) -> Self {
        self.session_id = session_id;
        self.max_autonomy_mode = max_mode;
        self
    }

    pub fn allows_project(&self, project_id: Uuid) -> bool {
        self.allowed_project_ids.contains(&project_id)
    }

    pub fn can_write_project(&self, project_id: Uuid) -> bool {
        self.writable_project_ids.contains(&project_id)
    }

    pub fn allowed_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.allowed_project_ids
    }

    /// Adds permissions resolved from the current human principal. Both flags
    /// default to false so integrations that do not explicitly opt in remain
    /// unable to expose governance records to a model.
    pub const fn with_governance_reads(mut self, read_activity: bool, read_audit: bool) -> Self {
        self.read_activity = read_activity;
        self.read_audit = read_audit;
        self
    }

    /// Adds live import/export authority without changing the constructor used
    /// by read-only integrations. Project sets are always intersected with the
    /// already-readable project boundary.
    pub fn with_data_access(
        mut self,
        importable_project_ids: impl IntoIterator<Item = Uuid>,
        exportable_project_ids: impl IntoIterator<Item = Uuid>,
        _lab_import: bool,
    ) -> Self {
        self.importable_project_ids = importable_project_ids
            .into_iter()
            .filter(|project_id| self.allowed_project_ids.contains(project_id))
            .collect();
        self.exportable_project_ids = exportable_project_ids
            .into_iter()
            .filter(|project_id| self.allowed_project_ids.contains(project_id))
            .collect();
        self
    }

    fn data_access_for_conversation(
        &self,
        conversation_id: Uuid,
        project_id: Option<Uuid>,
    ) -> AiDataAccessContext {
        let access = match project_id {
            Some(project_id) => AiDataAccessContext::new(
                self.lab_id,
                self.user_id,
                self.importable_project_ids
                    .contains(&project_id)
                    .then_some(project_id),
                self.exportable_project_ids
                    .contains(&project_id)
                    .then_some(project_id),
                false,
            ),
            // Lab-wide conversations are strictly read-only. Import, export,
            // and every write draft must first be anchored to an explicit
            // project conversation.
            None => AiDataAccessContext::none(self.lab_id, self.user_id),
        };
        access.with_conversation(conversation_id, project_id)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftDecisionResponse {
    pub draft: WriteDraftSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
}

#[derive(Clone)]
pub struct AiWorkflowService {
    store: Arc<dyn MuriArcStore>,
    operations: Arc<dyn AiOperationStore>,
    data_tools: Option<Arc<dyn AiDataToolBackend>>,
    source_resolver: Option<Arc<dyn AssistantSourceResolver>>,
}

impl std::fmt::Debug for AiWorkflowService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AiWorkflowService")
            .finish_non_exhaustive()
    }
}

impl AiWorkflowService {
    pub fn new(store: Arc<dyn MuriArcStore>, operations: Arc<dyn AiOperationStore>) -> Self {
        Self {
            store,
            operations,
            data_tools: None,
            source_resolver: None,
        }
    }

    pub fn with_data_tools(mut self, data_tools: Arc<dyn AiDataToolBackend>) -> Self {
        self.data_tools = Some(data_tools);
        self
    }

    pub fn with_source_resolver(
        mut self,
        source_resolver: Arc<dyn AssistantSourceResolver>,
    ) -> Self {
        self.source_resolver = Some(source_resolver);
        self
    }

    pub async fn run_turn<P: AiProvider>(
        &self,
        provider: P,
        api_key: Option<&str>,
        context: &AiExecutionContext,
        request: AssistantTurnRequest,
    ) -> Result<AssistantTurnResponse, AiWorkflowError> {
        self.run_turn_with_config(
            provider,
            api_key,
            context,
            request,
            AssistantRuntimeConfig::default(),
        )
        .await
    }

    pub async fn run_turn_with_config<P: AiProvider>(
        &self,
        provider: P,
        api_key: Option<&str>,
        context: &AiExecutionContext,
        request: AssistantTurnRequest,
        runtime: AssistantRuntimeConfig,
    ) -> Result<AssistantTurnResponse, AiWorkflowError> {
        let resolved = self.resolve_conversation(context, &request).await?;
        let conversation_id = resolved.conversation_id;
        let project_id = resolved.project_id;
        let source_bundle = self
            .resolve_sources(context, conversation_id, project_id, &request.source_refs)
            .await?;
        let source_refs = source_bundle.source_refs().to_vec();
        let ai_audit = ai_audit(context, "assistant_turn");
        let (mut stored_autonomy, mut autonomy) =
            self.effective_autonomy(context, conversation_id).await?;
        if autonomy.effective_mode == AiAutonomyMode::Full {
            if let Some(grant) = stored_autonomy.as_mut() {
                let expected_revision = grant.meta.revision;
                let now = Utc::now();
                grant.last_used_at = now;
                grant.expires_at = Some(now + Duration::minutes(30));
                grant.meta.touch(now);
                self.operations
                    .save_ai_autonomy_grant(grant, Some(expected_revision), &ai_audit)
                    .await?;
                autonomy.revision = grant.meta.revision;
                autonomy.expires_at = grant.expires_at;
            }
        }
        let (scoped_projects, writable_projects) = scoped_project_access(context, project_id);
        let tool_access = StoreToolAccessContext::new(context.lab_id, scoped_projects)
            .with_lab_registry_read(context.lab_registry_read && project_id.is_none())
            .with_activity_read(context.read_activity)
            .with_audit_read(context.read_audit)
            .with_current_user(context.user_id)
            .with_writable_projects(writable_projects);
        let mut executor = StoreDomainToolExecutor::new(self.store.clone(), tool_access)
            .with_autonomy_mode(autonomy.effective_mode);
        if let Some(data_tools) = &self.data_tools {
            executor = executor.with_data_tools(
                context.data_access_for_conversation(conversation_id, project_id),
                data_tools.clone(),
            );
        }
        let credentials = match api_key {
            Some(api_key) => ProviderCredentials::bearer(api_key)?,
            None => ProviderCredentials::none(),
        };
        let assistant = AssistantService::new(provider, executor).with_runtime_config(runtime)?;
        let response = assistant
            .run(
                AssistantRequest::new(context.user_id, request.message.clone())
                    .with_history(resolved.history)
                    .with_sources(source_bundle),
                &context.access_grant,
                credentials,
            )
            .await?;

        if let Some(conversation) = resolved.new_conversation.as_ref() {
            self.operations
                .create_ai_conversation(conversation, &ai_audit)
                .await?;
        }
        let (tool_runs, approvals) = self.build_turn_operation_records(
            context,
            conversation_id,
            project_id,
            &source_refs,
            &response,
        );
        let turn_response =
            AssistantTurnResponse::from_service(conversation_id, response, autonomy);
        self.persist_turn_records(
            context,
            project_id,
            resolved.expected_last_sequence,
            &request.message,
            &source_refs,
            &turn_response,
            &tool_runs,
            &approvals,
            &ai_audit,
        )
        .await?;
        Ok(turn_response)
    }

    async fn resolve_sources(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        project_id: Option<Uuid>,
        source_ids: &[Uuid],
    ) -> Result<AssistantSourceBundle, AiWorkflowError> {
        if source_ids.is_empty() {
            return Ok(AssistantSourceBundle::empty());
        }
        if source_ids.len() > MAX_ASSISTANT_SOURCES
            || source_ids.iter().any(Uuid::is_nil)
            || source_ids.iter().copied().collect::<BTreeSet<_>>().len() != source_ids.len()
        {
            return Err(AssistantSourceError::InvalidSource.into());
        }
        let resolver = self
            .source_resolver
            .as_ref()
            .ok_or(AssistantSourceError::ResolutionRequired)?;
        let bundle = resolver
            .resolve(AssistantSourceResolutionRequest {
                lab_id: context.lab_id,
                user_id: context.user_id,
                conversation_id,
                project_id,
                source_ids: source_ids.to_vec(),
            })
            .await?;
        if bundle.source_ids() != source_ids {
            return Err(AssistantSourceError::InvalidSource.into());
        }
        Ok(bundle)
    }

    pub async fn get_autonomy(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
    ) -> Result<AiAutonomyView, AiWorkflowError> {
        let conversation = self.operations.get_ai_conversation(conversation_id).await?;
        authorize_conversation(context, &conversation)?;
        self.effective_autonomy(context, conversation_id)
            .await
            .map(|(_, view)| view)
    }

    pub async fn set_autonomy(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        request: AiAutonomyUpdateRequest,
        step_up_verified: bool,
        audit: &AuditContext,
    ) -> Result<AiAutonomyView, AiWorkflowError> {
        let conversation = self.operations.get_ai_conversation(conversation_id).await?;
        authorize_writable_conversation(context, &conversation)?;
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(context.user_id)
            || request.expected_revision < 0
            || request.mode > context.max_autonomy_mode
            || (request.mode == AiAutonomyMode::Full && !step_up_verified)
        {
            return Err(AiWorkflowError::Forbidden);
        }
        let now = Utc::now();
        let existing = self
            .operations
            .get_ai_autonomy_grant(conversation_id)
            .await?;
        if existing.as_ref().map_or(0, |value| value.meta.revision) != request.expected_revision {
            return Err(
                StoreError::Conflict("AI autonomy grant revision changed".to_owned()).into(),
            );
        }
        let expected_revision = existing.as_ref().map(|value| value.meta.revision);
        let mut grant = existing.unwrap_or_else(|| AiAutonomyGrant {
            id: Uuid::new_v4(),
            conversation_id,
            lab_id: conversation.lab_id,
            project_id: conversation.project_id,
            user_id: conversation.user_id,
            session_id: None,
            mode: AiAutonomyMode::Ask,
            allowed_categories: vec![AiActionCategory::Read],
            batch_limit: 1,
            step_up_verified_at: None,
            last_used_at: now,
            expires_at: None,
            revoked_at: None,
            meta: RecordMeta::new(now),
        });
        grant.mode = request.mode;
        grant.allowed_categories = match request.mode {
            AiAutonomyMode::Ask => vec![AiActionCategory::Read],
            AiAutonomyMode::Auto | AiAutonomyMode::Full => vec![
                AiActionCategory::Read,
                AiActionCategory::Artifact,
                AiActionCategory::ReversibleDraft,
            ],
        };
        grant.batch_limit = request.mode.batch_limit();
        grant.session_id = (request.mode == AiAutonomyMode::Full)
            .then_some(context.session_id)
            .flatten();
        grant.step_up_verified_at = (request.mode == AiAutonomyMode::Full).then_some(now);
        grant.last_used_at = now;
        grant.expires_at =
            (request.mode == AiAutonomyMode::Full).then_some(now + Duration::minutes(30));
        grant.revoked_at = None;
        if expected_revision.is_some() {
            grant.meta.touch(now);
        }
        self.operations
            .save_ai_autonomy_grant(&grant, expected_revision, audit)
            .await?;
        Ok(autonomy_view(
            Some(&grant),
            context.max_autonomy_mode,
            context.session_id,
            now,
        ))
    }

    async fn effective_autonomy(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
    ) -> Result<(Option<AiAutonomyGrant>, AiAutonomyView), AiWorkflowError> {
        let grant = self
            .operations
            .get_ai_autonomy_grant(conversation_id)
            .await?;
        let view = autonomy_view(
            grant.as_ref(),
            context.max_autonomy_mode,
            context.session_id,
            Utc::now(),
        );
        Ok((grant, view))
    }

    /// Creates an empty, owner-bound conversation before the first assistant
    /// turn. This is used by trusted transports when source files must be
    /// attached to a real conversation instead of a nullable staging scope.
    ///
    /// Only the authenticated human owner may call this entry point. Project
    /// scope comes from the already-resolved execution context; the caller
    /// cannot use conversation creation to gain access to another project.
    pub async fn create_conversation(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        title: String,
        audit: &AuditContext,
    ) -> Result<AssistantConversationSummary, AiWorkflowError> {
        if project_id.is_some_and(|id| !context.allows_project(id)) {
            return Err(AiWorkflowError::Forbidden);
        }
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(context.user_id)
        {
            return Err(AiWorkflowError::Forbidden);
        }
        let now = Utc::now();
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: context.lab_id,
            project_id,
            user_id: context.user_id,
            title: normalize_conversation_title(title)?,
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        self.operations
            .create_ai_conversation(&conversation, audit)
            .await?;
        Ok(conversation.into())
    }

    pub async fn list_conversations(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        title_query: Option<String>,
        archive: AiConversationArchiveFilter,
        limit: u32,
    ) -> Result<Vec<AssistantConversationSummary>, AiWorkflowError> {
        if limit == 0 || limit > CONVERSATION_LIST_LIMIT {
            return Err(AiWorkflowError::InvalidConversationRequest);
        }
        if project_id.is_some_and(|id| !context.allows_project(id)) {
            return Err(AiWorkflowError::Forbidden);
        }
        let title_query = normalize_conversation_title_query(title_query)?;
        let mut conversations = self
            .operations
            .list_ai_conversations(
                &AiConversationFilter {
                    lab_id: context.lab_id,
                    user_id: context.user_id,
                    project_id,
                    title_query,
                    archive,
                    ..AiConversationFilter::default()
                },
                0,
                CONVERSATION_LIST_LIMIT,
            )
            .await?;
        conversations.retain(|conversation| {
            conversation.project_id.is_none()
                || conversation
                    .project_id
                    .is_some_and(|id| context.allows_project(id))
        });
        conversations.truncate(limit as usize);
        Ok(conversations.into_iter().map(Into::into).collect())
    }

    /// Applies one human-requested metadata transition to a conversation owned
    /// by the authenticated user. The caller must supply a transport-created
    /// human audit context; model output cannot manufacture this authority.
    pub async fn update_conversation(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        expected_revision: i64,
        change: AiConversationChange,
        audit: &AuditContext,
    ) -> Result<AssistantConversationSummary, AiWorkflowError> {
        if conversation_id.is_nil() || expected_revision <= 0 {
            return Err(AiWorkflowError::InvalidConversationRequest);
        }
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(context.user_id)
        {
            return Err(AiWorkflowError::Forbidden);
        }
        let conversation = self.operations.get_ai_conversation(conversation_id).await?;
        authorize_conversation(context, &conversation)?;
        if conversation.archived_at.is_some() && !matches!(change, AiConversationChange::Unarchive)
        {
            return Err(archived_conversation_error());
        }
        let change = normalize_conversation_change(change)?;
        let updated = self
            .operations
            .update_ai_conversation(
                &AiConversationUpdate {
                    id: conversation_id,
                    expected_revision,
                    change,
                    updated_at: Utc::now(),
                },
                audit,
            )
            .await?;
        authorize_conversation(context, &updated)?;
        Ok(updated.into())
    }

    pub async fn get_conversation(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        limit: u32,
    ) -> Result<AssistantConversationDetail, AiWorkflowError> {
        if limit == 0 || limit > CONVERSATION_DETAIL_LIMIT {
            return Err(AiWorkflowError::InvalidConversationRequest);
        }
        let conversation = self.operations.get_ai_conversation(conversation_id).await?;
        authorize_conversation(context, &conversation)?;
        let stored_messages = self
            .operations
            .list_ai_conversation_messages(conversation.id, limit)
            .await?;
        let mut messages = Vec::with_capacity(stored_messages.len());
        for stored in stored_messages {
            let response = match stored.role {
                AiConversationMessageRole::User => {
                    if stored.response.is_some() {
                        return Err(AiWorkflowError::InvalidStoredConversation);
                    }
                    None
                }
                AiConversationMessageRole::Assistant => {
                    let mut response: AssistantTurnResponse = serde_json::from_value(
                        stored
                            .response
                            .clone()
                            .ok_or(AiWorkflowError::InvalidStoredConversation)?,
                    )
                    .map_err(|_| AiWorkflowError::InvalidStoredConversation)?;
                    if response.conversation_id != conversation.id
                        || response.content != stored.content
                    {
                        return Err(AiWorkflowError::InvalidStoredConversation);
                    }
                    for draft in &mut response.drafts {
                        *draft = self.get_draft(context, draft.id).await?;
                    }
                    Some(response)
                }
            };
            messages.push(AssistantConversationMessage {
                id: stored.id,
                sequence: stored.sequence,
                role: stored.role,
                content: stored.content,
                source_refs: stored.source_refs.into_iter().map(Into::into).collect(),
                response,
                created_at: stored.meta.created_at,
            });
        }
        Ok(AssistantConversationDetail {
            conversation: conversation.into(),
            messages,
        })
    }

    pub async fn list_drafts(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        status: Option<DraftStatus>,
    ) -> Result<Vec<WriteDraftSummary>, AiWorkflowError> {
        if project_id.is_some_and(|project_id| !context.allows_project(project_id)) {
            return Err(AiWorkflowError::Forbidden);
        }
        let stored_decision = status.and_then(|status| match status {
            DraftStatus::PendingApproval => Some(StoredApprovalDecision::Pending),
            DraftStatus::Rejected | DraftStatus::Cancelled => {
                Some(StoredApprovalDecision::Rejected)
            }
            DraftStatus::Approved | DraftStatus::Applied => Some(StoredApprovalDecision::Approved),
            DraftStatus::Expired => None,
        });
        let approvals = self
            .operations
            .list_approvals(&AiApprovalFilter {
                lab_id: context.lab_id,
                user_id: context.user_id,
                project_id,
                decision: stored_decision,
            })
            .await?;
        let mut drafts = Vec::with_capacity(approvals.len());
        for approval in approvals {
            let tool_run = self.operations.get_tool_run(approval.tool_run_id).await?;
            let draft = self.authorized_stored_draft(context, &approval, &tool_run)?;
            if status.is_none_or(|status| status == draft.status()) {
                drafts.push(WriteDraftSummary::from(&draft));
            }
        }
        Ok(drafts)
    }

    pub async fn get_draft(
        &self,
        context: &AiExecutionContext,
        draft_id: Uuid,
    ) -> Result<WriteDraftSummary, AiWorkflowError> {
        let approval = self.operations.get_approval(draft_id).await?;
        let tool_run = self.operations.get_tool_run(approval.tool_run_id).await?;
        let draft = self.authorized_stored_draft(context, &approval, &tool_run)?;
        Ok(WriteDraftSummary::from(&draft))
    }

    pub async fn decide_draft(
        &self,
        context: &AiExecutionContext,
        draft_id: Uuid,
        request: DraftDecisionRequest,
        human_audit: &AuditContext,
    ) -> Result<DraftDecisionResponse, AiWorkflowError> {
        if human_audit.actor.actor_type != ActorType::Human
            || human_audit.actor.user_id != Some(context.user_id)
        {
            return Err(AiWorkflowError::Forbidden);
        }
        let mut approval = self.operations.get_approval(draft_id).await?;
        let mut tool_run = self.operations.get_tool_run(approval.tool_run_id).await?;
        let mut draft = self.authorized_stored_draft(context, &approval, &tool_run)?;
        let decision_data_access = if request.decision == ApprovalDecision::Approve {
            let conversation_id = tool_run
                .conversation_id
                .ok_or(AiWorkflowError::InvalidStoredDraft)?;
            let conversation = self.operations.get_ai_conversation(conversation_id).await?;
            authorize_project_bound_draft(context, &conversation, &tool_run)?;
            Some(context.data_access_for_conversation(conversation.id, conversation.project_id))
        } else {
            None
        };
        let now = portable_storage_timestamp(Utc::now());
        draft.decide(
            request.expected_revision,
            request.decision,
            HumanApprover {
                user_id: context.user_id,
                display_name: context.user_display_name.clone(),
            },
            request.statement.clone(),
            request.step_up_verified,
            now,
        )?;

        let expected_tool_revision = tool_run.meta.revision;
        let expected_approval_revision = approval.meta.revision;
        approval.decision = match request.decision {
            ApprovalDecision::Approve => StoredApprovalDecision::Approved,
            ApprovalDecision::Reject => StoredApprovalDecision::Rejected,
        };
        approval.decided_by = Some(context.user_id);
        approval.decided_at = Some(now);
        approval.reason = request.statement;
        approval.meta.touch(now);
        tool_run.completed_at = Some(now);
        tool_run.meta.touch(now);

        match request.decision {
            ApprovalDecision::Reject => {
                tool_run.status = ToolRunStatus::Cancelled;
                approval.requested_diff = json!({"draft": draft});
                tool_run.output = Some(json!({"draft": draft}));
                self.operations
                    .finalize_ai_draft(
                        &tool_run,
                        expected_tool_revision,
                        &approval,
                        expected_approval_revision,
                        human_audit,
                    )
                    .await?;
                Ok(DraftDecisionResponse {
                    draft: WriteDraftSummary::from(&draft),
                    measurement_id: None,
                    job_id: None,
                })
            }
            ApprovalDecision::Approve => match (draft.kind(), draft.tool()) {
                (DraftKind::MeasurementResult, ToolName::MutationDraft) => {
                    let project_id = draft
                        .project_id()
                        .ok_or(AiWorkflowError::InvalidStoredDraft)?;
                    let (measurement, expected_animal_revision) = measurement_from_draft(&draft)?;
                    if measurement.lab_id != context.lab_id
                        || measurement.project_id != project_id
                        || !context.can_write_project(measurement.project_id)
                    {
                        return Err(AiWorkflowError::Forbidden);
                    }
                    draft.mark_applied(draft.revision())?;
                    tool_run.status = ToolRunStatus::Completed;
                    approval.requested_diff = json!({"draft": draft});
                    let (provider_id, model) = trusted_provider_metadata(tool_run.output.as_ref());
                    tool_run.output = Some(json!({
                        "draft": draft,
                        "measurement_id": measurement.id,
                        "provider_id": provider_id,
                        "model": model,
                    }));
                    self.operations
                        .apply_ai_measurement_draft(
                            &measurement,
                            expected_animal_revision,
                            &tool_run,
                            expected_tool_revision,
                            &approval,
                            expected_approval_revision,
                            human_audit,
                        )
                        .await?;
                    Ok(DraftDecisionResponse {
                        draft: WriteDraftSummary::from(&draft),
                        measurement_id: Some(measurement.id),
                        job_id: None,
                    })
                }
                (DraftKind::ResearchPlan, ToolName::ExperimentGroupingDraft) => {
                    let (plan, application) = grouping_application_from_draft(&draft)?;
                    if application.lab_id != context.lab_id
                        || draft.project_id() != Some(application.project_id)
                        || !context.can_write_project(application.project_id)
                    {
                        return Err(AiWorkflowError::Forbidden);
                    }
                    draft.mark_applied(draft.revision())?;
                    tool_run.status = ToolRunStatus::Completed;
                    let cohort_ids = application
                        .cohorts
                        .iter()
                        .map(|cohort| cohort.id)
                        .collect::<Vec<_>>();
                    let participation_ids = application
                        .participations
                        .iter()
                        .map(|participation| participation.id)
                        .collect::<Vec<_>>();
                    approval.requested_diff = json!({"draft": draft});
                    tool_run.output = Some(json!({
                        "draft": draft,
                        "input_snapshot_sha256": plan.input_snapshot_sha256,
                        "cohort_ids": cohort_ids,
                        "participation_ids": participation_ids,
                    }));
                    self.operations
                        .apply_ai_experiment_grouping_draft(
                            &application,
                            &tool_run,
                            expected_tool_revision,
                            &approval,
                            expected_approval_revision,
                            human_audit,
                        )
                        .await?;
                    Ok(DraftDecisionResponse {
                        draft: WriteDraftSummary::from(&draft),
                        measurement_id: None,
                        job_id: None,
                    })
                }
                (DraftKind::BulkImport, ToolName::ImportCommitDraft) => {
                    let backend = self
                        .data_tools
                        .as_ref()
                        .ok_or(AiWorkflowError::UnsupportedDraftOperation)?;
                    let binding: crate::ImportCommitDraftPayload =
                        serde_json::from_value(draft.payload().clone())
                            .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
                    binding.validate().map_err(AiWorkflowError::DataTool)?;
                    let mut applied_draft = draft.clone();
                    applied_draft.mark_applied(applied_draft.revision())?;
                    let mut applied_tool_run = tool_run.clone();
                    applied_tool_run.status = ToolRunStatus::Completed;
                    let (provider_id, model) =
                        trusted_provider_metadata(applied_tool_run.output.as_ref());
                    applied_tool_run.output = Some(json!({
                        "draft": applied_draft,
                        "provider_id": provider_id,
                        "model": model,
                    }));
                    let mut applied_approval = approval.clone();
                    applied_approval.requested_diff = json!({"draft": applied_draft});
                    let resolution = AiImportResolution {
                        expected_job_revision: binding.expected_revision,
                        tool_run: applied_tool_run,
                        expected_tool_run_revision: expected_tool_revision,
                        approval: applied_approval,
                        expected_approval_revision,
                    };
                    let result = backend
                        .apply_import_draft(
                            decision_data_access
                                .as_ref()
                                .ok_or(AiWorkflowError::Forbidden)?,
                            &draft,
                            &resolution,
                            human_audit,
                        )
                        .await;
                    match result {
                        Ok(result) => Ok(DraftDecisionResponse {
                            draft: WriteDraftSummary::from(&applied_draft),
                            measurement_id: None,
                            job_id: Some(result.job_id),
                        }),
                        Err(error) => {
                            // The human decision is final even when the bounded
                            // application operation fails. Persist an Approved
                            // (not Applied) draft and a failed tool run so the
                            // failure is auditable and never masquerades as a
                            // successful mutation.
                            tool_run.status = ToolRunStatus::Failed;
                            tool_run.error = Some(tool_error_code(&error));
                            approval.requested_diff = json!({"draft": draft});
                            let (provider_id, model) =
                                trusted_provider_metadata(tool_run.output.as_ref());
                            tool_run.output = Some(json!({
                                "draft": draft,
                                "error": tool_error_code(&error),
                                "provider_id": provider_id,
                                "model": model,
                            }));
                            self.operations
                                .finalize_ai_draft(
                                    &tool_run,
                                    expected_tool_revision,
                                    &approval,
                                    expected_approval_revision,
                                    human_audit,
                                )
                                .await?;
                            Err(AiWorkflowError::DataTool(error))
                        }
                    }
                }
                _ => Err(AiWorkflowError::UnsupportedDraftOperation),
            },
        }
    }

    async fn resolve_conversation(
        &self,
        context: &AiExecutionContext,
        request: &AssistantTurnRequest,
    ) -> Result<ResolvedConversation, AiWorkflowError> {
        if request
            .project_id
            .is_some_and(|project_id| !context.allows_project(project_id))
        {
            return Err(AiWorkflowError::Forbidden);
        }
        if let Some(conversation_id) = request.conversation_id {
            let conversation = self.operations.get_ai_conversation(conversation_id).await?;
            authorize_writable_conversation(context, &conversation)?;
            if request
                .project_id
                .is_some_and(|id| Some(id) != conversation.project_id)
            {
                return Err(AiWorkflowError::Forbidden);
            }
            let messages = self
                .operations
                .list_ai_conversation_messages(conversation.id, PROVIDER_HISTORY_LIMIT)
                .await?;
            let expected_last_sequence = messages.last().map_or(0, |message| message.sequence);
            let history = provider_history(&messages)?;
            Ok(ResolvedConversation {
                conversation_id: conversation.id,
                project_id: conversation.project_id,
                new_conversation: None,
                history,
                expected_last_sequence,
            })
        } else {
            let now = Utc::now();
            let conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: context.lab_id,
                project_id: request.project_id,
                user_id: context.user_id,
                title: conversation_title(&request.message),
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            Ok(ResolvedConversation {
                conversation_id: conversation.id,
                project_id: conversation.project_id,
                new_conversation: Some(conversation),
                history: Vec::new(),
                expected_last_sequence: 0,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_turn_records(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        expected_last_sequence: i64,
        user_content: &str,
        source_refs: &[AiConversationSourceRef],
        response: &AssistantTurnResponse,
        tool_runs: &[ToolRun],
        approvals: &[Approval],
        audit: &AuditContext,
    ) -> Result<(), AiWorkflowError> {
        let now = Utc::now();
        let user_message = AiConversationMessage::new(
            response.conversation_id,
            context.lab_id,
            project_id,
            context.user_id,
            expected_last_sequence + 1,
            AiConversationMessageRole::User,
            user_content,
            None,
            now,
        )
        .and_then(|message| message.with_source_refs(source_refs.to_vec()))
        .map_err(|error| StoreError::Validation(error.to_string()))?;
        let response_value = serde_json::to_value(response)
            .map_err(|error| StoreError::Serialization(error.to_string()))?;
        let assistant_message = AiConversationMessage::new(
            response.conversation_id,
            context.lab_id,
            project_id,
            context.user_id,
            expected_last_sequence + 2,
            AiConversationMessageRole::Assistant,
            response.content.clone(),
            Some(response_value),
            now,
        )
        .map_err(|error| StoreError::Validation(error.to_string()))?;
        self.operations
            .append_ai_turn_records(
                &user_message,
                &assistant_message,
                tool_runs,
                approvals,
                expected_last_sequence,
                audit,
            )
            .await?;
        Ok(())
    }

    fn build_turn_operation_records(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        project_id: Option<Uuid>,
        source_refs: &[AiConversationSourceRef],
        response: &crate::AssistantResponse,
    ) -> (Vec<ToolRun>, Vec<Approval>) {
        let now = Utc::now();
        let mut tool_runs = Vec::with_capacity(response.tool_runs.len());
        let mut approvals = Vec::with_capacity(response.drafts.len());
        for trace in &response.tool_runs {
            let draft = trace
                .draft_id
                .and_then(|draft_id| response.drafts.iter().find(|draft| draft.id() == draft_id));
            let status = if draft.is_some() {
                ToolRunStatus::AwaitingApproval
            } else {
                ToolRunStatus::Completed
            };
            let output = json!({
                "trace": trace,
                "draft": draft,
                "assistant_content": response.content,
                "provider_id": response.provider_id,
                "model": response.model,
                "incomplete_reason": response.incomplete_reason,
                "source_refs": source_refs,
            });
            let tool_run = ToolRun {
                id: trace.tool_run_id,
                conversation_id: Some(conversation_id),
                lab_id: context.lab_id,
                project_id,
                user_id: context.user_id,
                tool_name: trace.tool.as_str().to_owned(),
                input: trace.arguments.clone(),
                output: Some(output),
                status,
                source: WriteSource::Ai,
                started_at: Some(now),
                completed_at: (status == ToolRunStatus::Completed).then_some(now),
                error: None,
                meta: RecordMeta::new(now),
            };
            if let Some(draft) = draft {
                approvals.push(Approval {
                    id: draft.id(),
                    tool_run_id: tool_run.id,
                    requested_diff: json!({"draft": draft}),
                    decision: StoredApprovalDecision::Pending,
                    decided_by: None,
                    decided_at: None,
                    reason: None,
                    meta: RecordMeta::new(now),
                });
            }
            tool_runs.push(tool_run);
        }
        (tool_runs, approvals)
    }

    fn authorized_stored_draft(
        &self,
        context: &AiExecutionContext,
        approval: &Approval,
        tool_run: &ToolRun,
    ) -> Result<WriteDraft, AiWorkflowError> {
        if tool_run.lab_id != context.lab_id
            || tool_run.user_id != context.user_id
            || tool_run
                .project_id
                .is_some_and(|project_id| !context.allows_project(project_id))
            || approval.tool_run_id != tool_run.id
        {
            return Err(AiWorkflowError::Forbidden);
        }
        let draft_value = approval
            .requested_diff
            .get("draft")
            .cloned()
            .ok_or(AiWorkflowError::InvalidStoredDraft)?;
        let draft: WriteDraft =
            serde_json::from_value(draft_value).map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
        draft
            .validate_integrity()
            .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
        if draft.id() != approval.id
            || draft.project_id() != tool_run.project_id
            || !matches!(
                draft.proposed_by(),
                ProposalActor::Ai { user_id, tool_run_id }
                    if *user_id == context.user_id && *tool_run_id == tool_run.id
            )
        {
            return Err(AiWorkflowError::InvalidStoredDraft);
        }
        Ok(draft)
    }
}

struct ResolvedConversation {
    conversation_id: Uuid,
    project_id: Option<Uuid>,
    new_conversation: Option<AiConversation>,
    history: Vec<ChatMessage>,
    expected_last_sequence: i64,
}

fn authorize_conversation(
    context: &AiExecutionContext,
    conversation: &AiConversation,
) -> Result<(), AiWorkflowError> {
    if conversation.lab_id != context.lab_id
        || conversation.user_id != context.user_id
        || conversation
            .project_id
            .is_some_and(|project_id| !context.allows_project(project_id))
    {
        Err(AiWorkflowError::Forbidden)
    } else {
        Ok(())
    }
}

fn authorize_writable_conversation(
    context: &AiExecutionContext,
    conversation: &AiConversation,
) -> Result<(), AiWorkflowError> {
    authorize_conversation(context, conversation)?;
    if conversation.archived_at.is_some() {
        Err(archived_conversation_error())
    } else {
        Ok(())
    }
}

fn authorize_project_bound_draft(
    context: &AiExecutionContext,
    conversation: &AiConversation,
    tool_run: &ToolRun,
) -> Result<(), AiWorkflowError> {
    authorize_writable_conversation(context, conversation)?;
    let project_id = conversation.project_id.ok_or(AiWorkflowError::Forbidden)?;
    if tool_run.conversation_id != Some(conversation.id) || tool_run.project_id != Some(project_id)
    {
        Err(AiWorkflowError::InvalidStoredDraft)
    } else {
        Ok(())
    }
}

fn archived_conversation_error() -> AiWorkflowError {
    StoreError::Conflict("AI conversation is archived".to_owned()).into()
}

fn autonomy_view(
    grant: Option<&AiAutonomyGrant>,
    max_mode: AiAutonomyMode,
    session_id: Option<Uuid>,
    now: chrono::DateTime<Utc>,
) -> AiAutonomyView {
    let requested = grant.map_or(AiAutonomyMode::Ask, |value| value.mode);
    let effective = grant
        .map_or(AiAutonomyMode::Ask, |value| {
            value.effective_mode(now, session_id)
        })
        .min(max_mode);
    AiAutonomyView {
        mode: requested,
        effective_mode: effective,
        max_mode,
        batch_limit: effective.batch_limit(),
        revision: grant.map_or(0, |value| value.meta.revision),
        expires_at: (effective == AiAutonomyMode::Full)
            .then(|| grant.and_then(|value| value.expires_at))
            .flatten(),
        requires_human_approval: crate::transport::hard_boundaries(),
    }
}

fn provider_history(
    messages: &[AiConversationMessage],
) -> Result<Vec<ChatMessage>, AiWorkflowError> {
    if !messages.len().is_multiple_of(2) {
        return Err(AiWorkflowError::InvalidStoredConversation);
    }
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let expected = if index.is_multiple_of(2) {
                AiConversationMessageRole::User
            } else {
                AiConversationMessageRole::Assistant
            };
            if message.role != expected {
                return Err(AiWorkflowError::InvalidStoredConversation);
            }
            Ok(match message.role {
                AiConversationMessageRole::User => ChatMessage::user(message.content.clone()),
                AiConversationMessageRole::Assistant => {
                    ChatMessage::assistant(message.content.clone())
                }
            })
        })
        .collect()
}

fn scoped_project_access(
    context: &AiExecutionContext,
    project_id: Option<Uuid>,
) -> (BTreeSet<Uuid>, BTreeSet<Uuid>) {
    match project_id {
        Some(project_id) => {
            let writable_projects = if context.can_write_project(project_id) {
                BTreeSet::from([project_id])
            } else {
                BTreeSet::new()
            };
            (BTreeSet::from([project_id]), writable_projects)
        }
        // A lab-wide conversation may discover and compare projects, but a
        // mutation must be anchored to an explicitly selected project so the
        // persisted conversation, tool run and approval share one scope.
        None => (context.allowed_project_ids.clone(), BTreeSet::new()),
    }
}

fn measurement_from_draft(draft: &WriteDraft) -> Result<(Measurement, i64), AiWorkflowError> {
    if draft.tool() != crate::ToolName::MutationDraft {
        return Err(AiWorkflowError::UnsupportedDraftOperation);
    }
    let payload = draft.payload();
    if payload.get("operation").and_then(Value::as_str) != Some("create_measurement") {
        return Err(AiWorkflowError::UnsupportedDraftOperation);
    }
    let measurement: Measurement = serde_json::from_value(
        payload
            .get("measurement")
            .cloned()
            .ok_or(AiWorkflowError::InvalidStoredDraft)?,
    )
    .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
    let expected_animal_revision = payload
        .get("animal_revision")
        .and_then(Value::as_i64)
        .filter(|revision| *revision > 0)
        .ok_or(AiWorkflowError::InvalidStoredDraft)?;
    measurement
        .validate_record()
        .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
    Ok((measurement, expected_animal_revision))
}

fn grouping_application_from_draft(
    draft: &WriteDraft,
) -> Result<(ExperimentGroupingPlan, AiExperimentGroupingApplication), AiWorkflowError> {
    if draft.tool() != ToolName::ExperimentGroupingDraft
        || draft.kind() != DraftKind::ResearchPlan
        || draft.payload().get("operation").and_then(Value::as_str)
            != Some("apply_experiment_grouping")
    {
        return Err(AiWorkflowError::UnsupportedDraftOperation);
    }
    let plan: ExperimentGroupingPlan = serde_json::from_value(
        draft
            .payload()
            .get("plan")
            .cloned()
            .ok_or(AiWorkflowError::InvalidStoredDraft)?,
    )
    .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
    let application: AiExperimentGroupingApplication = serde_json::from_value(
        draft
            .payload()
            .get("application")
            .cloned()
            .ok_or(AiWorkflowError::InvalidStoredDraft)?,
    )
    .map_err(|_| AiWorkflowError::InvalidStoredDraft)?;
    if plan.schema_version != RESEARCH_GROUPING_SCHEMA_VERSION
        || !plan.requires_researcher_signature
        || plan.project_id != application.project_id
        || plan.expected_project_revision != application.expected_project_revision
        || plan.experiment_id != application.experiment_id
        || plan.expected_experiment_revision != application.expected_experiment_revision
        || plan.input_snapshot_sha256 != application.input_snapshot_sha256
        || plan.cohort_names.len() != application.cohorts.len()
        || plan.assignments.len() != application.participations.len()
        || plan.assignments.len() + plan.exclusions.len()
            != application.expected_animal_revisions.len()
        || draft.project_id() != Some(plan.project_id)
        || application
            .cohorts
            .iter()
            .enumerate()
            .any(|(index, cohort)| {
                cohort.experiment_id != plan.experiment_id
                    || cohort.name != plan.cohort_names[index]
                    || cohort.meta.revision != 1
                    || cohort.meta.deleted_at.is_some()
            })
    {
        return Err(AiWorkflowError::InvalidStoredDraft);
    }
    let revisions = application
        .expected_animal_revisions
        .iter()
        .map(|value| (value.animal_id, value.expected_revision))
        .collect::<std::collections::BTreeMap<_, _>>();
    if revisions.len() != application.expected_animal_revisions.len() {
        return Err(AiWorkflowError::InvalidStoredDraft);
    }
    let weight_revisions = application
        .expected_latest_weights
        .iter()
        .map(|value| (value.animal_id, value))
        .collect::<std::collections::BTreeMap<_, _>>();
    let balances_weight = plan
        .balance_by
        .iter()
        .any(|factor| factor == "weight_grams");
    if (balances_weight
        && (weight_revisions.len() != revisions.len()
            || weight_revisions
                .keys()
                .any(|id| !revisions.contains_key(id))))
        || (!balances_weight && !application.expected_latest_weights.is_empty())
        || application.expected_latest_weights.iter().any(|value| {
            value.animal_id.is_nil()
                || value.measurement_id.is_some() != value.expected_revision.is_some()
                || value.measurement_id.is_some_and(|id| id.is_nil())
                || value
                    .expected_revision
                    .is_some_and(|revision| revision <= 0)
        })
    {
        return Err(AiWorkflowError::InvalidStoredDraft);
    }
    for assignment in &plan.assignments {
        let participation = application
            .participations
            .iter()
            .find(|value| value.animal_id == assignment.animal_id)
            .ok_or(AiWorkflowError::InvalidStoredDraft)?;
        if participation.experiment_id != plan.experiment_id
            || participation.cohort_id
                != application
                    .cohorts
                    .get(assignment.cohort_index)
                    .map(|cohort| cohort.id)
            || !participation.genotype_snapshot.is_empty()
            || participation.meta.revision != 1
            || participation.meta.deleted_at.is_some()
            || revisions.get(&assignment.animal_id) != Some(&assignment.expected_revision)
        {
            return Err(AiWorkflowError::InvalidStoredDraft);
        }
    }
    for exclusion in &plan.exclusions {
        if revisions.get(&exclusion.animal_id) != Some(&exclusion.expected_revision) {
            return Err(AiWorkflowError::InvalidStoredDraft);
        }
    }
    if draft.changes().len() != application.cohorts.len() + application.participations.len() {
        return Err(AiWorkflowError::InvalidStoredDraft);
    }
    Ok((plan, application))
}

fn tool_error_code(error: &ToolExecutionError) -> String {
    match error {
        ToolExecutionError::Rejected { code } => code.clone(),
        ToolExecutionError::Unavailable => "data_tool_unavailable".to_owned(),
    }
}

fn trusted_provider_metadata(output: Option<&Value>) -> (Option<String>, Option<String>) {
    fn trusted_string(output: Option<&Value>, key: &str, max_len: usize) -> Option<String> {
        output
            .and_then(|value| value.as_object())
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.len() <= max_len)
            .map(str::to_owned)
    }

    (
        trusted_string(output, "provider_id", 128),
        trusted_string(output, "model", 256),
    )
}

fn conversation_title(message: &str) -> String {
    let title = message.trim().chars().take(80).collect::<String>();
    if title.is_empty() {
        "MuriArc AI conversation".to_owned()
    } else {
        title
    }
}

fn normalize_conversation_title_query(
    title_query: Option<String>,
) -> Result<Option<String>, AiWorkflowError> {
    let title_query = title_query
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if title_query
        .as_ref()
        .is_some_and(|value| value.chars().count() > 256 || value.chars().any(char::is_control))
    {
        return Err(AiWorkflowError::InvalidConversationRequest);
    }
    Ok(title_query)
}

fn normalize_conversation_change(
    change: AiConversationChange,
) -> Result<AiConversationChange, AiWorkflowError> {
    match change {
        AiConversationChange::Rename { title } => Ok(AiConversationChange::Rename {
            title: normalize_conversation_title(title)?,
        }),
        change => Ok(change),
    }
}

fn normalize_conversation_title(title: String) -> Result<String, AiWorkflowError> {
    let title = title.trim().to_owned();
    if title.is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        Err(AiWorkflowError::InvalidConversationRequest)
    } else {
        Ok(title)
    }
}

fn ai_audit(context: &AiExecutionContext, reason: &'static str) -> AuditContext {
    AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(context.user_id),
            display_name: format!("MuriArc AI for {}", context.user_display_name),
        },
        source: WriteSource::Ai,
        request_id: Some(context.request_id.clone()),
        reason: Some(reason.to_owned()),
    }
}

#[derive(Debug, Error)]
pub enum AiWorkflowError {
    #[error(transparent)]
    Assistant(#[from] AssistantError),
    #[error(transparent)]
    Config(#[from] AssistantConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Approval(#[from] ApprovalError),
    #[error(transparent)]
    DataTool(#[from] ToolExecutionError),
    #[error(transparent)]
    Credential(#[from] crate::CredentialError),
    #[error(transparent)]
    Source(#[from] AssistantSourceError),
    #[error("AI operation is outside the authenticated lab/project scope")]
    Forbidden,
    #[error("stored AI draft is invalid")]
    InvalidStoredDraft,
    #[error("stored AI draft operation is unsupported")]
    UnsupportedDraftOperation,
    #[error("AI conversation request is outside bounded limits")]
    InvalidConversationRequest,
    #[error("stored AI conversation is invalid")]
    InvalidStoredConversation,
}

impl AiWorkflowError {
    /// Stable product-level error code shared by Server and Desktop.
    ///
    /// HTTP status selection and localized user messages remain transport
    /// concerns, but the underlying failure classification must not drift.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Store(StoreError::NotFound { .. }) => "not_found",
            Self::Store(StoreError::Conflict(_)) => "conflict",
            Self::Store(StoreError::Validation(_)) => "validation_error",
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_)) => "storage_error",
            Self::Approval(ApprovalError::RevisionConflict { .. }) => "conflict",
            Self::Approval(_) | Self::Credential(_) | Self::Config(_) => "validation_error",
            Self::Assistant(error) => error.code(),
            Self::Source(_) => "invalid_ai_source",
            Self::DataTool(ToolExecutionError::Rejected { .. }) => "conflict",
            Self::DataTool(ToolExecutionError::Unavailable) => "ai_data_unavailable",
            Self::InvalidStoredDraft
            | Self::UnsupportedDraftOperation
            | Self::InvalidStoredConversation => "storage_error",
            Self::InvalidConversationRequest => "validation_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiDataApplyResult, AiSourceImportKind, AssistantIncompleteReason, AssistantLimits,
        CompletionResponse, DomainToolOutput, DomainToolRequest, FieldChange,
        ImportDraftPreviewRow, ImportDraftPreviewSummary, MockProvider, ProviderToolCall,
        ResolvedAssistantSource, ScopeSet, ToolScope,
    };
    use async_trait::async_trait;
    use muriarc_core::{
        Animal, AuditFilter, EntityType, Experiment, Lab, MeasurementValue, Participation, Project,
        ProvenanceFilter, ProvenanceSource, Sex, User,
    };
    use muriarc_store_sqlite::SqliteStore;
    use std::sync::Mutex;

    struct FakeImportBackend {
        job_id: Uuid,
        project_id: Uuid,
        fail_apply: bool,
        operations: Arc<SqliteStore>,
    }

    struct FakeSourceResolver {
        requests: Mutex<Vec<AssistantSourceResolutionRequest>>,
    }

    fn import_preview(project_id: Uuid) -> ImportDraftPreviewSummary {
        ImportDraftPreviewSummary {
            import_kind: AiSourceImportKind::Measurement,
            project_id,
            experiment_id: Uuid::new_v4(),
            file_name: "measurements.csv".to_owned(),
            sheet_name: "Sheet1".to_owned(),
            total_rows: 1,
            accepted_rows: 1,
            issue_count: 0,
            issues_truncated: false,
            can_confirm: true,
            preview_rows: vec![ImportDraftPreviewRow {
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
        }
    }

    #[async_trait]
    impl AssistantSourceResolver for FakeSourceResolver {
        async fn resolve(
            &self,
            request: AssistantSourceResolutionRequest,
        ) -> Result<AssistantSourceBundle, AssistantSourceError> {
            self.requests.lock().unwrap().push(request.clone());
            AssistantSourceBundle::try_from_sources(
                request
                    .source_ids
                    .iter()
                    .map(|source_id| ResolvedAssistantSource {
                        source_id: *source_id,
                        source_revision: 3,
                        attachment_id: Uuid::from_u128(source_id.as_u128() ^ (1_u128 << 127)),
                        file_name: format!("{source_id}.csv"),
                        media_type: "text/csv".to_owned(),
                        size_bytes: 42,
                        material: json!({
                            "kind": "table",
                            "headers": ["animal_code"],
                            "rows": [[format!("M-{source_id}")]]
                        }),
                        images: Vec::new(),
                    })
                    .collect(),
            )
        }
    }

    #[async_trait]
    impl AiDataToolBackend for FakeImportBackend {
        fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
            access
                .can_import_project(self.project_id)
                .then_some(ToolName::ImportCommitDraft)
                .into_iter()
                .collect()
        }

        async fn execute(
            &self,
            _access: &AiDataAccessContext,
            request: DomainToolRequest,
        ) -> Result<DomainToolOutput, ToolExecutionError> {
            let now = Utc::now();
            let draft = WriteDraft::new(
                DraftKind::BulkImport,
                ToolName::ImportCommitDraft,
                ProposalActor::Ai {
                    user_id: request.user_id,
                    tool_run_id: request.tool_run_id,
                },
                Some(self.project_id),
                vec![FieldChange {
                    path: format!("/data/imports/{}", self.job_id),
                    before: Some(json!({"status": "awaiting_confirmation"})),
                    after: Some(json!({"status": "completed"})),
                }],
                json!({
                    "operation": "confirm_import",
                    "job_id": self.job_id,
                    "preview_hash": "a".repeat(64),
                    "expected_revision": 2,
                    "preview": import_preview(self.project_id),
                }),
                now,
                now + chrono::Duration::hours(1),
            )
            .unwrap();
            Ok(DomainToolOutput::write_draft(draft, Vec::new()))
        }

        async fn apply_import_draft(
            &self,
            _access: &AiDataAccessContext,
            draft: &WriteDraft,
            resolution: &AiImportResolution,
            audit: &AuditContext,
        ) -> Result<AiDataApplyResult, ToolExecutionError> {
            assert_eq!(draft.status(), DraftStatus::Approved);
            assert_eq!(resolution.approval.id, draft.id());
            assert_eq!(resolution.expected_job_revision, 2);
            assert!(
                resolution
                    .tool_run
                    .completed_at
                    .is_some_and(muriarc_core::has_portable_storage_precision)
            );
            assert!(muriarc_core::has_portable_storage_precision(
                resolution.tool_run.meta.updated_at
            ));
            assert!(
                resolution
                    .approval
                    .decided_at
                    .is_some_and(muriarc_core::has_portable_storage_precision)
            );
            assert!(muriarc_core::has_portable_storage_precision(
                resolution.approval.meta.updated_at
            ));
            if self.fail_apply {
                Err(ToolExecutionError::Rejected {
                    code: "stale_import_fixture".to_owned(),
                })
            } else {
                self.operations
                    .finalize_ai_draft(
                        &resolution.tool_run,
                        resolution.expected_tool_run_revision,
                        &resolution.approval,
                        resolution.expected_approval_revision,
                        audit,
                    )
                    .await
                    .map_err(|_| ToolExecutionError::Unavailable)?;
                Ok(AiDataApplyResult {
                    job_id: self.job_id,
                    result: json!({"committed": true}),
                })
            }
        }
    }

    fn context(projects: [Uuid; 2]) -> AiExecutionContext {
        AiExecutionContext::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Researcher",
            "request-1",
            projects,
            projects,
            true,
            AccessGrant::local_user(ScopeSet::new([
                ToolScope::Read,
                ToolScope::WriteDraft,
                ToolScope::Import,
                ToolScope::Export,
            ])),
        )
        .with_data_access(projects, projects, true)
    }

    #[test]
    fn lab_wide_turn_is_read_only_even_when_user_can_write_projects() {
        let projects = [Uuid::new_v4(), Uuid::new_v4()];
        let context = context(projects);

        let (readable, writable) = scoped_project_access(&context, None);
        let data_access = context.data_access_for_conversation(Uuid::new_v4(), None);

        assert_eq!(readable, BTreeSet::from(projects));
        assert!(writable.is_empty());
        assert!(!data_access.can_import_anything());
        assert!(!data_access.can_export_anything());
        assert!(!data_access.can_import_lab());
        assert!(data_access.importable_project_ids().is_empty());
        assert!(data_access.exportable_project_ids().is_empty());
    }

    #[test]
    fn explicit_project_turn_keeps_only_that_projects_write_scope() {
        let projects = [Uuid::new_v4(), Uuid::new_v4()];
        let context = context(projects);

        let (readable, writable) = scoped_project_access(&context, Some(projects[1]));
        let data_access = context.data_access_for_conversation(Uuid::new_v4(), Some(projects[1]));

        assert_eq!(readable, BTreeSet::from([projects[1]]));
        assert_eq!(writable, BTreeSet::from([projects[1]]));
        assert!(data_access.can_import_project(projects[1]));
        assert!(data_access.can_export_project(projects[1]));
        assert!(!data_access.can_import_project(projects[0]));
        assert!(!data_access.can_export_project(projects[0]));
        assert!(!data_access.can_import_lab());
    }

    #[tokio::test]
    async fn lab_wide_stored_write_draft_can_be_rejected_but_not_applied() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Lab-wide approval boundary", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "lab-wide-approval@example.test",
            "Lab-wide reviewer",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: None,
            user_id: user.id,
            title: "Legacy lab-wide draft".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let ai_audit = AuditContext {
            actor: Actor {
                actor_type: ActorType::Ai,
                user_id: Some(user.id),
                display_name: "MuriArc AI".to_owned(),
            },
            source: WriteSource::Ai,
            request_id: Some("legacy-lab-wide-draft".to_owned()),
            reason: Some("persist a legacy lab-wide draft fixture".to_owned()),
        };
        store
            .create_ai_conversation(&conversation, &ai_audit)
            .await
            .unwrap();
        let tool_run_id = Uuid::new_v4();
        let draft = WriteDraft::new(
            DraftKind::BulkImport,
            ToolName::ImportCommitDraft,
            ProposalActor::Ai {
                user_id: user.id,
                tool_run_id,
            },
            None,
            vec![FieldChange {
                path: format!("/data/imports/{}", Uuid::new_v4()),
                before: Some(json!({"status": "awaiting_confirmation"})),
                after: Some(json!({"status": "completed"})),
            }],
            json!({
                "operation": "confirm_import",
                "job_id": Uuid::new_v4(),
                "preview_hash": "a".repeat(64),
                "expected_revision": 2,
                "preview": import_preview(Uuid::new_v4()),
            }),
            now,
            now + Duration::hours(1),
        )
        .unwrap();
        let tool_run = ToolRun {
            id: tool_run_id,
            conversation_id: Some(conversation.id),
            lab_id: lab.id,
            project_id: None,
            user_id: user.id,
            tool_name: ToolName::ImportCommitDraft.as_str().to_owned(),
            input: json!({"job_id": Uuid::new_v4()}),
            output: Some(json!({"draft": &draft})),
            status: ToolRunStatus::AwaitingApproval,
            source: WriteSource::Ai,
            started_at: Some(now),
            completed_at: None,
            error: None,
            meta: RecordMeta::new(now),
        };
        let approval = Approval {
            id: draft.id(),
            tool_run_id,
            requested_diff: json!({"draft": &draft}),
            decision: StoredApprovalDecision::Pending,
            decided_by: None,
            decided_at: None,
            reason: None,
            meta: RecordMeta::new(now),
        };
        store.create_tool_run(&tool_run, &ai_audit).await.unwrap();
        store.create_approval(&approval, &ai_audit).await.unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "lab-wide-decision",
            std::iter::empty(),
            std::iter::empty(),
            true,
            AccessGrant::local_user(ScopeSet::new([
                ToolScope::Read,
                ToolScope::Import,
                ToolScope::WriteDraft,
            ])),
        )
        .with_data_access(std::iter::empty(), std::iter::empty(), true);
        let human_audit = AuditContext {
            actor: Actor::human(user.id, user.display_name.clone()),
            source: WriteSource::Web,
            request_id: Some("lab-wide-decision".to_owned()),
            reason: Some("review legacy draft".to_owned()),
        };

        let blocked = workflow
            .decide_draft(
                &context,
                draft.id(),
                DraftDecisionRequest {
                    expected_revision: 1,
                    decision: ApprovalDecision::Approve,
                    statement: Some("I reviewed the preview".to_owned()),
                    step_up_verified: true,
                },
                &human_audit,
            )
            .await;
        assert!(matches!(blocked, Err(AiWorkflowError::Forbidden)));
        assert_eq!(
            store.get_approval(draft.id()).await.unwrap().decision,
            StoredApprovalDecision::Pending
        );
        assert_eq!(
            store.get_tool_run(tool_run_id).await.unwrap().status,
            ToolRunStatus::AwaitingApproval
        );

        let rejected = workflow
            .decide_draft(
                &context,
                draft.id(),
                DraftDecisionRequest {
                    expected_revision: 1,
                    decision: ApprovalDecision::Reject,
                    statement: Some("This draft has no project scope".to_owned()),
                    step_up_verified: false,
                },
                &human_audit,
            )
            .await
            .unwrap();
        assert_eq!(rejected.draft.status, DraftStatus::Rejected);
        assert_eq!(
            store.get_approval(draft.id()).await.unwrap().decision,
            StoredApprovalDecision::Rejected
        );
    }

    #[tokio::test]
    async fn measurement_approval_preserves_provider_metadata_for_provenance() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Measurement provenance workflow", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "measurement-provenance@example.test",
            "Measurement reviewer",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Measurement provenance", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let animal = Animal::new_mouse(lab.id, "PROVENANCE-001", Sex::Female, now).unwrap();
        store.create_animal(&animal, &bootstrap).await.unwrap();
        let experiment = Experiment::new(lab.id, project.id, "Weight study", now).unwrap();
        store
            .create_experiment(&experiment, &bootstrap)
            .await
            .unwrap();
        let participation = Participation::enroll(experiment.id, animal.id, now);
        store
            .create_participation(&participation, &bootstrap)
            .await
            .unwrap();
        let animal = store.get_animal(animal.id).await.unwrap();
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            title: "Record a weight".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let ai_audit = AuditContext {
            actor: Actor {
                actor_type: ActorType::Ai,
                user_id: Some(user.id),
                display_name: "MuriArc AI".to_owned(),
            },
            source: WriteSource::Ai,
            request_id: Some("measurement-draft".to_owned()),
            reason: Some("AI proposed a measurement".to_owned()),
        };
        store
            .create_ai_conversation(&conversation, &ai_audit)
            .await
            .unwrap();
        let mut measurement = Measurement::draft(
            lab.id,
            project.id,
            animal.id,
            "body_weight",
            "Body weight",
            MeasurementValue::Number(22.4),
            now,
            now,
        )
        .unwrap();
        measurement.experiment_id = Some(experiment.id);
        measurement.unit = Some("g".to_owned());
        let measurement_value = serde_json::to_value(&measurement).unwrap();
        let tool_run_id = Uuid::new_v4();
        let draft = WriteDraft::new(
            DraftKind::MeasurementResult,
            ToolName::MutationDraft,
            ProposalActor::Ai {
                user_id: user.id,
                tool_run_id,
            },
            Some(project.id),
            vec![FieldChange {
                path: format!("/measurements/{}", measurement.id),
                before: None,
                after: Some(measurement_value.clone()),
            }],
            json!({
                "operation": "create_measurement",
                "measurement": measurement_value,
                "animal_revision": animal.meta.revision,
            }),
            now,
            now + Duration::hours(1),
        )
        .unwrap();
        let tool_run = ToolRun {
            id: tool_run_id,
            conversation_id: Some(conversation.id),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            tool_name: ToolName::MutationDraft.as_str().to_owned(),
            input: json!({"operation": "create_measurement"}),
            output: Some(json!({
                "draft": &draft,
                "provider_id": "provider-under-test",
                "model": "model-under-test",
            })),
            status: ToolRunStatus::AwaitingApproval,
            source: WriteSource::Ai,
            started_at: Some(now),
            completed_at: None,
            error: None,
            meta: RecordMeta::new(now),
        };
        let approval = Approval {
            id: draft.id(),
            tool_run_id,
            requested_diff: json!({"draft": &draft}),
            decision: StoredApprovalDecision::Pending,
            decided_by: None,
            decided_at: None,
            reason: None,
            meta: RecordMeta::new(now),
        };
        store.create_tool_run(&tool_run, &ai_audit).await.unwrap();
        store.create_approval(&approval, &ai_audit).await.unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "measurement-approval",
            [project.id],
            [project.id],
            false,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read, ToolScope::WriteDraft])),
        );
        let human_audit = AuditContext {
            actor: Actor::human(user.id, user.display_name.clone()),
            source: WriteSource::Web,
            request_id: Some("measurement-approval".to_owned()),
            reason: Some("reviewed measurement source".to_owned()),
        };

        let applied = workflow
            .decide_draft(
                &context,
                draft.id(),
                DraftDecisionRequest {
                    expected_revision: 1,
                    decision: ApprovalDecision::Approve,
                    statement: Some("Verified source value".to_owned()),
                    step_up_verified: false,
                },
                &human_audit,
            )
            .await
            .unwrap();
        assert_eq!(applied.measurement_id, Some(measurement.id));
        let stored_tool = store.get_tool_run(tool_run_id).await.unwrap();
        assert_eq!(
            stored_tool
                .output
                .as_ref()
                .and_then(|value| value.get("provider_id"))
                .and_then(Value::as_str),
            Some("provider-under-test")
        );
        assert_eq!(
            stored_tool
                .output
                .as_ref()
                .and_then(|value| value.get("model"))
                .and_then(Value::as_str),
            Some("model-under-test")
        );
        let provenance = store
            .list_provenance(&ProvenanceFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                entity_type: Some(EntityType::Measurement),
                entity_id: Some(measurement.id),
                source: Some(ProvenanceSource::Ai),
            })
            .await
            .unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(
            provenance[0].provider.as_deref(),
            Some("provider-under-test")
        );
        assert_eq!(provenance[0].model.as_deref(), Some("model-under-test"));
    }

    #[tokio::test]
    async fn human_owner_creates_an_empty_conversation_only_in_authorized_scope() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Conversation creation", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "conversation-create@example.test",
            "Conversation owner",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Allowed project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let forbidden_project = Project::new(lab.id, "Forbidden project", now).unwrap();
        store
            .create_project(&forbidden_project, &bootstrap)
            .await
            .unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "create-conversation-request",
            [project.id],
            [project.id],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let audit = AuditContext {
            actor: Actor::human(user.id, user.display_name.clone()),
            source: WriteSource::Web,
            request_id: Some("create-conversation-request".to_owned()),
            reason: Some("prepare a conversation for source uploads".to_owned()),
        };

        let created = workflow
            .create_conversation(
                &context,
                Some(project.id),
                "  New source conversation  ".to_owned(),
                &audit,
            )
            .await
            .unwrap();
        assert_eq!(created.project_id, Some(project.id));
        assert_eq!(created.title, "New source conversation");
        assert_eq!(created.revision, 1);
        assert!(
            store
                .list_ai_conversation_messages(created.id, 20)
                .await
                .unwrap()
                .is_empty()
        );
        let persisted = store.get_ai_conversation(created.id).await.unwrap();
        assert_eq!(persisted.user_id, user.id);
        assert_eq!(persisted.lab_id, lab.id);
        let audits = store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                entity_id: Some(created.id),
            })
            .await
            .unwrap();
        let creation = audits
            .iter()
            .find(|entry| entry.entity_type == EntityType::AiConversation)
            .unwrap();
        assert_eq!(creation.actor.actor_type, ActorType::Human);
        assert_eq!(creation.actor.user_id, Some(user.id));
        assert_eq!(creation.source, WriteSource::Web);
        assert_eq!(
            creation.request_id.as_deref(),
            Some("create-conversation-request")
        );

        assert!(matches!(
            workflow
                .create_conversation(
                    &context,
                    Some(forbidden_project.id),
                    "Out of scope".to_owned(),
                    &audit,
                )
                .await,
            Err(AiWorkflowError::Forbidden)
        ));
        assert!(matches!(
            workflow
                .create_conversation(
                    &context,
                    None,
                    "AI-owned attempt".to_owned(),
                    &ai_audit(&context, "model_must_not_create_conversation"),
                )
                .await,
            Err(AiWorkflowError::Forbidden)
        ));
        assert!(matches!(
            workflow
                .create_conversation(&context, None, " \n ".to_owned(), &audit)
                .await,
            Err(AiWorkflowError::InvalidConversationRequest)
        ));
        let wrong_owner_audit = AuditContext {
            actor: Actor::human(Uuid::new_v4(), "Other user"),
            ..audit
        };
        assert!(matches!(
            workflow
                .create_conversation(&context, None, "Wrong owner".to_owned(), &wrong_owner_audit,)
                .await,
            Err(AiWorkflowError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn successful_turns_persist_and_restore_bounded_multiturn_history() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Workflow history", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(lab.id, "history@example.test", "History researcher", now).unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "History project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "history-request",
            [project.id],
            [project.id],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read, ToolScope::WriteDraft])),
        );
        let completion = |content: &str| CompletionResponse {
            id: Some(Uuid::new_v4().to_string()),
            model: Some("history-model".to_owned()),
            content: Some(content.to_owned()),
            tool_calls: Vec::new(),
            finish_reason: Some("stop".to_owned()),
            usage: None,
        };

        let first = workflow
            .run_turn(
                MockProvider::new("history", "history-model", [Ok(completion("First answer"))]),
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "First question".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await
            .unwrap();
        let second_provider = MockProvider::new(
            "history",
            "history-model",
            [Ok(completion("Second answer"))],
        );
        let provider_probe = second_provider.clone();
        let second = workflow
            .run_turn(
                second_provider,
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: Some(first.conversation_id),
                    project_id: Some(project.id),
                    message: "Second question".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(second.conversation_id, first.conversation_id);

        let provider_messages = &provider_probe.requests().unwrap()[0].messages;
        assert_eq!(provider_messages[1].content, "First question");
        assert_eq!(provider_messages[2].content, "First answer");
        assert_eq!(provider_messages[3].content, "Second question");

        let summaries = workflow
            .list_conversations(
                &context,
                Some(project.id),
                None,
                AiConversationArchiveFilter::Active,
                20,
            )
            .await
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].revision, 3);
        let human_audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some("conversation-management".to_owned()),
            reason: Some("manage AI conversation".to_owned()),
        };
        let renamed = workflow
            .update_conversation(
                &context,
                first.conversation_id,
                summaries[0].revision,
                AiConversationChange::Rename {
                    title: "  Longitudinal study  ".to_owned(),
                },
                &human_audit,
            )
            .await
            .unwrap();
        assert_eq!(renamed.title, "Longitudinal study");
        assert_eq!(
            workflow
                .list_conversations(
                    &context,
                    Some(project.id),
                    Some("  STUDY ".to_owned()),
                    AiConversationArchiveFilter::Active,
                    20,
                )
                .await
                .unwrap(),
            vec![renamed.clone()]
        );
        let pinned = workflow
            .update_conversation(
                &context,
                renamed.id,
                renamed.revision,
                AiConversationChange::Pin,
                &human_audit,
            )
            .await
            .unwrap();
        assert!(pinned.pinned_at.is_some());
        let archived = workflow
            .update_conversation(
                &context,
                pinned.id,
                pinned.revision,
                AiConversationChange::Archive,
                &human_audit,
            )
            .await
            .unwrap();
        assert!(archived.archived_at.is_some());
        assert!(
            workflow
                .get_conversation(&context, archived.id, 20)
                .await
                .is_ok()
        );
        assert!(workflow.get_autonomy(&context, archived.id).await.is_ok());
        assert!(matches!(
            workflow
                .set_autonomy(
                    &context,
                    archived.id,
                    AiAutonomyUpdateRequest {
                        mode: AiAutonomyMode::Ask,
                        expected_revision: 0,
                    },
                    false,
                    &human_audit,
                )
                .await,
            Err(AiWorkflowError::Store(StoreError::Conflict(_)))
        ));
        assert!(matches!(
            workflow
                .run_turn(
                    MockProvider::new(
                        "history",
                        "history-model",
                        [Ok(completion("Must not run"))],
                    ),
                    None,
                    &context,
                    AssistantTurnRequest {
                        conversation_id: Some(archived.id),
                        project_id: Some(project.id),
                        message: "Continue archived conversation".to_owned(),
                        source_refs: Vec::new(),
                    },
                )
                .await,
            Err(AiWorkflowError::Store(StoreError::Conflict(_)))
        ));
        assert!(matches!(
            workflow
                .update_conversation(
                    &context,
                    archived.id,
                    archived.revision,
                    AiConversationChange::Rename {
                        title: "Must unarchive first".to_owned(),
                    },
                    &human_audit,
                )
                .await,
            Err(AiWorkflowError::Store(StoreError::Conflict(_)))
        ));
        assert!(
            workflow
                .list_conversations(
                    &context,
                    Some(project.id),
                    None,
                    AiConversationArchiveFilter::Active,
                    20,
                )
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            workflow
                .list_conversations(
                    &context,
                    Some(project.id),
                    None,
                    AiConversationArchiveFilter::Archived,
                    20,
                )
                .await
                .unwrap(),
            vec![archived.clone()]
        );
        let restored = workflow
            .update_conversation(
                &context,
                archived.id,
                archived.revision,
                AiConversationChange::Unarchive,
                &human_audit,
            )
            .await
            .unwrap();
        assert!(restored.archived_at.is_none());
        assert!(matches!(
            workflow
                .update_conversation(
                    &context,
                    restored.id,
                    restored.revision,
                    AiConversationChange::Unpin,
                    &ai_audit(&context, "model_must_not_manage_conversation"),
                )
                .await,
            Err(AiWorkflowError::Forbidden)
        ));
        assert!(matches!(
            workflow
                .list_conversations(
                    &context,
                    Some(project.id),
                    Some("invalid\nquery".to_owned()),
                    AiConversationArchiveFilter::Active,
                    20,
                )
                .await,
            Err(AiWorkflowError::InvalidConversationRequest)
        ));
        let detail = workflow
            .get_conversation(&context, first.conversation_id, 200)
            .await
            .unwrap();
        assert_eq!(detail.messages.len(), 4);
        assert_eq!(detail.messages[0].content, "First question");
        assert_eq!(
            detail.messages[1].response.as_ref().unwrap().content,
            "First answer"
        );
        assert_eq!(detail.messages[2].content, "Second question");
        assert_eq!(
            detail.messages[3].response.as_ref().unwrap().content,
            "Second answer"
        );

        let failed = workflow
            .run_turn(
                MockProvider::new("history", "history-model", []),
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "This provider call fails".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await;
        assert!(matches!(failed, Err(AiWorkflowError::Assistant(_))));
        assert_eq!(
            workflow
                .list_conversations(
                    &context,
                    Some(project.id),
                    None,
                    AiConversationArchiveFilter::Active,
                    20,
                )
                .await
                .unwrap()
                .len(),
            1,
            "failed provider turns must not create resumable conversations"
        );
    }

    #[tokio::test]
    async fn opaque_source_refs_are_resolved_after_scope_and_only_original_question_is_persisted() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Workflow sources", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(lab.id, "sources@example.test", "Source researcher", now).unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Source project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let source_id = Uuid::new_v4();
        let resolver = Arc::new(FakeSourceResolver {
            requests: Mutex::new(Vec::new()),
        });
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow =
            AiWorkflowService::new(domain, operations).with_source_resolver(resolver.clone());
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "source-request",
            [project.id],
            [project.id],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let provider = MockProvider::new(
            "source",
            "source-model",
            [
                Ok(CompletionResponse {
                    id: Some("source-tool-response".to_owned()),
                    model: Some("source-model".to_owned()),
                    content: None,
                    tool_calls: vec![ProviderToolCall {
                        id: "source-project-search".to_owned(),
                        name: ToolName::ResourceSearch.as_str().to_owned(),
                        arguments: json!({"resource": "projects"}),
                    }],
                    finish_reason: Some("tool_calls".to_owned()),
                    usage: None,
                }),
                Ok(CompletionResponse {
                    id: Some("source-response".to_owned()),
                    model: Some("source-model".to_owned()),
                    content: Some("I found the selected animal.".to_owned()),
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_owned()),
                    usage: None,
                }),
            ],
        );
        let probe = provider.clone();

        let turn = workflow
            .run_turn(
                provider,
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "Summarize this source".to_owned(),
                    source_refs: vec![source_id],
                },
            )
            .await
            .unwrap();

        {
            let resolution = resolver.requests.lock().unwrap();
            assert_eq!(resolution.len(), 1);
            assert_eq!(resolution[0].conversation_id, turn.conversation_id);
            assert_eq!(resolution[0].project_id, Some(project.id));
        }
        let provider_messages = probe.requests().unwrap()[0].messages.clone();
        assert!(
            provider_messages[1]
                .content
                .contains(&source_id.to_string())
        );
        let detail = workflow
            .get_conversation(&context, turn.conversation_id, 20)
            .await
            .unwrap();
        assert_eq!(detail.messages[0].content, "Summarize this source");
        assert!(!detail.messages[0].content.contains(&source_id.to_string()));
        assert_eq!(detail.messages[0].source_refs.len(), 1);
        assert_eq!(detail.messages[0].source_refs[0].source_id, source_id);
        assert_eq!(detail.messages[0].source_refs[0].source_revision, 3);
        assert_eq!(
            detail.messages[0].source_refs[0].file_name,
            format!("{source_id}.csv")
        );
        assert!(detail.messages[1].source_refs.is_empty());

        let persisted_tool = store
            .get_tool_run(turn.tool_runs[0].tool_run_id)
            .await
            .unwrap();
        let tool_output = persisted_tool.output.unwrap();
        assert_eq!(
            tool_output["source_refs"][0]["sourceId"],
            serde_json::Value::String(source_id.to_string())
        );
        let serialized_tool_output = tool_output.to_string();
        assert!(!serialized_tool_output.contains("relative_path"));
        assert!(!serialized_tool_output.contains("sha256"));

        let second_source_id = Uuid::new_v4();
        let second_provider = MockProvider::new(
            "source",
            "source-model",
            [Ok(CompletionResponse {
                id: Some("second-source-response".to_owned()),
                model: Some("source-model".to_owned()),
                content: Some("I reviewed the second source.".to_owned()),
                tool_calls: Vec::new(),
                finish_reason: Some("stop".to_owned()),
                usage: None,
            })],
        );
        let second_probe = second_provider.clone();
        workflow
            .run_turn(
                second_provider,
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: Some(turn.conversation_id),
                    project_id: Some(project.id),
                    message: "Review a different source".to_owned(),
                    source_refs: vec![second_source_id],
                },
            )
            .await
            .unwrap();

        let reloaded = workflow
            .get_conversation(&context, turn.conversation_id, 20)
            .await
            .unwrap();
        assert_eq!(reloaded.messages.len(), 4);
        assert_eq!(reloaded.messages[0].source_refs[0].source_id, source_id);
        assert_eq!(
            reloaded.messages[2].source_refs[0].source_id,
            second_source_id
        );
        assert!(reloaded.messages[3].source_refs.is_empty());
        let second_request = &second_probe.requests().unwrap()[0];
        let second_context = second_request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(second_context.contains(&second_source_id.to_string()));
        assert!(!second_context.contains(&source_id.to_string()));
    }

    #[tokio::test]
    async fn incomplete_turn_persists_completed_tool_runs_and_both_messages() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Workflow incomplete", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "incomplete@example.test",
            "Incomplete researcher",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Incomplete project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "incomplete-request",
            [project.id],
            [project.id],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let completions = (0..AssistantLimits::default().max_iterations)
            .map(|index| {
                Ok(CompletionResponse {
                    id: Some(format!("incomplete-response-{index}")),
                    model: Some("incomplete-model".to_owned()),
                    content: None,
                    tool_calls: vec![ProviderToolCall {
                        id: format!("resource-search-{index}"),
                        name: ToolName::ResourceSearch.as_str().to_owned(),
                        arguments: json!({"resource": "projects"}),
                    }],
                    finish_reason: Some("tool_calls".to_owned()),
                    usage: None,
                })
            })
            .collect::<Vec<_>>();

        let turn = workflow
            .run_turn(
                MockProvider::new("incomplete", "incomplete-model", completions),
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "Repeatedly inspect this project".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            turn.incomplete_reason,
            Some(AssistantIncompleteReason::IterationLimitExceeded)
        );
        assert_eq!(
            turn.tool_runs.len(),
            AssistantLimits::default().max_iterations - 1
        );
        for trace in &turn.tool_runs {
            let persisted = store.get_tool_run(trace.tool_run_id).await.unwrap();
            assert_eq!(persisted.status, ToolRunStatus::Completed);
            assert_eq!(persisted.conversation_id, Some(turn.conversation_id));
        }
        let detail = workflow
            .get_conversation(&context, turn.conversation_id, 20)
            .await
            .unwrap();
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(
            detail.messages[1]
                .response
                .as_ref()
                .unwrap()
                .incomplete_reason,
            turn.incomplete_reason
        );
        assert_eq!(
            detail.messages[1].response.as_ref().unwrap().tool_runs,
            turn.tool_runs
        );
    }

    #[tokio::test]
    async fn zero_progress_tool_call_limit_is_persisted_with_stable_feedback() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Workflow zero progress", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "zero-progress@example.test",
            "Zero progress researcher",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Zero progress project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "zero-progress-request",
            [project.id],
            [project.id],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let tool_calls = (0..=AssistantLimits::default().max_tool_calls)
            .map(|index| ProviderToolCall {
                id: format!("zero-progress-{index}"),
                name: ToolName::ResourceSearch.as_str().to_owned(),
                arguments: json!({"resource": "projects"}),
            })
            .collect();

        let turn = workflow
            .run_turn(
                MockProvider::new(
                    "zero-progress",
                    "zero-progress-model",
                    [Ok(CompletionResponse {
                        id: Some("zero-progress-response".to_owned()),
                        model: Some("zero-progress-model".to_owned()),
                        content: None,
                        tool_calls,
                        finish_reason: Some("tool_calls".to_owned()),
                        usage: None,
                    })],
                ),
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "Inspect too many resources at once".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            turn.incomplete_reason,
            Some(AssistantIncompleteReason::ToolCallLimitExceeded)
        );
        assert!(turn.tool_runs.is_empty());
        assert!(turn.content.contains("No data was changed"));
        let detail = workflow
            .get_conversation(&context, turn.conversation_id, 20)
            .await
            .unwrap();
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(
            detail.messages[1]
                .response
                .as_ref()
                .and_then(|response| response.incomplete_reason),
            Some(AssistantIncompleteReason::ToolCallLimitExceeded)
        );
    }

    async fn import_draft_fixture(
        fail_apply: bool,
    ) -> (
        Arc<SqliteStore>,
        AiWorkflowService,
        AiExecutionContext,
        Uuid,
        Uuid,
    ) {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Workflow import", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(lab.id, "import@example.test", "Importer", now).unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let project = Project::new(lab.id, "Import project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let job_id = Uuid::new_v4();
        let backend = Arc::new(FakeImportBackend {
            job_id,
            project_id: project.id,
            fail_apply,
            operations: store.clone(),
        });
        let domain: Arc<dyn MuriArcStore> = store.clone();
        let operations: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain, operations).with_data_tools(backend);
        let context = AiExecutionContext::new(
            lab.id,
            user.id,
            user.display_name.clone(),
            "import-workflow-request",
            [project.id],
            [project.id],
            false,
            AccessGrant::local_user(ScopeSet::new([
                ToolScope::Read,
                ToolScope::Import,
                ToolScope::WriteDraft,
            ])),
        )
        .with_data_access([project.id], std::iter::empty(), false);
        let provider = MockProvider::new(
            "workflow-import",
            "workflow-import-model",
            [
                Ok(CompletionResponse {
                    id: Some("import-call-response".to_owned()),
                    model: Some("workflow-import-model".to_owned()),
                    content: None,
                    tool_calls: vec![ProviderToolCall {
                        id: "import-call".to_owned(),
                        name: ToolName::ImportCommitDraft.as_str().to_owned(),
                        arguments: json!({
                            "job_id": job_id,
                            "preview_hash": "a".repeat(64),
                            "expected_revision": 2,
                        }),
                    }],
                    finish_reason: Some("tool_calls".to_owned()),
                    usage: None,
                }),
                Ok(CompletionResponse {
                    id: Some("import-final-response".to_owned()),
                    model: Some("workflow-import-model".to_owned()),
                    content: Some("The import is ready for review.".to_owned()),
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_owned()),
                    usage: None,
                }),
            ],
        );
        let turn = workflow
            .run_turn(
                provider,
                None,
                &context,
                AssistantTurnRequest {
                    conversation_id: None,
                    project_id: Some(project.id),
                    message: "Prepare this existing import for confirmation".to_owned(),
                    source_refs: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(turn.drafts.len(), 1);
        (store, workflow, context, turn.drafts[0].id, job_id)
    }

    #[tokio::test]
    async fn bulk_import_is_applied_only_after_reinforced_approval_and_failures_stay_auditable() {
        let (store, workflow, context, draft_id, job_id) = import_draft_fixture(false).await;
        let audit = AuditContext {
            actor: Actor::human(context.user_id, "Importer"),
            source: WriteSource::Web,
            request_id: Some("approve-import".to_owned()),
            reason: Some("reviewed import preview".to_owned()),
        };
        let applied = workflow
            .decide_draft(
                &context,
                draft_id,
                DraftDecisionRequest {
                    expected_revision: 1,
                    decision: ApprovalDecision::Approve,
                    statement: Some("I reviewed the complete import diff".to_owned()),
                    step_up_verified: true,
                },
                &audit,
            )
            .await
            .unwrap();
        assert_eq!(applied.job_id, Some(job_id));
        assert_eq!(applied.draft.status, DraftStatus::Applied);
        let approval = store.get_approval(draft_id).await.unwrap();
        let tool_run = store.get_tool_run(approval.tool_run_id).await.unwrap();
        assert_eq!(approval.decision, StoredApprovalDecision::Approved);
        assert_eq!(tool_run.status, ToolRunStatus::Completed);

        let (store, workflow, context, draft_id, _) = import_draft_fixture(true).await;
        let audit = AuditContext {
            actor: Actor::human(context.user_id, "Importer"),
            source: WriteSource::Web,
            request_id: Some("reject-stale-import".to_owned()),
            reason: Some("stale import fixture".to_owned()),
        };
        let failed = workflow
            .decide_draft(
                &context,
                draft_id,
                DraftDecisionRequest {
                    expected_revision: 1,
                    decision: ApprovalDecision::Approve,
                    statement: Some("I reviewed the complete import diff".to_owned()),
                    step_up_verified: true,
                },
                &audit,
            )
            .await;
        assert!(matches!(failed, Err(AiWorkflowError::DataTool(_))));
        let approval = store.get_approval(draft_id).await.unwrap();
        let tool_run = store.get_tool_run(approval.tool_run_id).await.unwrap();
        let stored = workflow.get_draft(&context, draft_id).await.unwrap();
        assert_eq!(approval.decision, StoredApprovalDecision::Approved);
        assert_eq!(tool_run.status, ToolRunStatus::Failed);
        assert_eq!(tool_run.error.as_deref(), Some("stale_import_fixture"));
        assert_eq!(stored.status, DraftStatus::Approved);
    }
}
