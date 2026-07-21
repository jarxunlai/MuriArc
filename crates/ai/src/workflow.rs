use std::{collections::BTreeSet, sync::Arc};

use chrono::{Duration, Utc};
use muriarc_core::{
    Actor, ActorType, AiActionCategory, AiApprovalFilter, AiAutonomyGrant, AiAutonomyMode,
    AiConversation, AiConversationFilter, AiConversationMessage, AiConversationMessageRole,
    AiOperationStore, Approval, ApprovalDecision as StoredApprovalDecision, AuditContext,
    Measurement, MuriArcStore, RecordMeta, StoreError, ToolRun, ToolRunStatus, WriteSource,
};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiDataAccessContext, AiDataToolBackend,
    AiProvider, ApprovalDecision, ApprovalError, AssistantConversationDetail,
    AssistantConversationMessage, AssistantConversationSummary, AssistantError, AssistantRequest,
    AssistantService, AssistantTurnRequest, AssistantTurnResponse, ChatMessage,
    DraftDecisionRequest, DraftKind, DraftStatus, HumanApprover, ProposalActor,
    ProviderCredentials, StoreDomainToolExecutor, StoreToolAccessContext, ToolExecutionError,
    ToolName, WriteDraft, WriteDraftSummary,
};

const PROVIDER_HISTORY_LIMIT: u32 = 40;
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
    lab_import: bool,
    lab_registry_read: bool,
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
            lab_import: false,
            lab_registry_read,
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

    /// Adds live import/export authority without changing the constructor used
    /// by read-only integrations. Project sets are always intersected with the
    /// already-readable project boundary.
    pub fn with_data_access(
        mut self,
        importable_project_ids: impl IntoIterator<Item = Uuid>,
        exportable_project_ids: impl IntoIterator<Item = Uuid>,
        lab_import: bool,
    ) -> Self {
        self.importable_project_ids = importable_project_ids
            .into_iter()
            .filter(|project_id| self.allowed_project_ids.contains(project_id))
            .collect();
        self.exportable_project_ids = exportable_project_ids
            .into_iter()
            .filter(|project_id| self.allowed_project_ids.contains(project_id))
            .collect();
        self.lab_import = lab_import;
        self
    }

    fn data_access(&self) -> AiDataAccessContext {
        AiDataAccessContext::new(
            self.lab_id,
            self.user_id,
            self.importable_project_ids.iter().copied(),
            self.exportable_project_ids.iter().copied(),
            self.lab_import,
        )
    }

    fn data_access_for_conversation(&self, project_id: Option<Uuid>) -> AiDataAccessContext {
        match project_id {
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
            // A lab-wide conversation may confirm a lab-registry import and
            // create read-only project exports, but cannot create a
            // project-scoped import draft whose approval would have a
            // different persisted conversation scope.
            None => AiDataAccessContext::new(
                self.lab_id,
                self.user_id,
                std::iter::empty(),
                self.exportable_project_ids.iter().copied(),
                self.lab_import,
            ),
        }
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
        }
    }

    pub fn with_data_tools(mut self, data_tools: Arc<dyn AiDataToolBackend>) -> Self {
        self.data_tools = Some(data_tools);
        self
    }

    pub async fn run_turn<P: AiProvider>(
        &self,
        provider: P,
        api_key: Option<&str>,
        context: &AiExecutionContext,
        request: AssistantTurnRequest,
    ) -> Result<AssistantTurnResponse, AiWorkflowError> {
        let resolved = self.resolve_conversation(context, &request).await?;
        let conversation_id = resolved.conversation_id;
        let project_id = resolved.project_id;
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
            .with_writable_projects(writable_projects);
        let mut executor = StoreDomainToolExecutor::new(self.store.clone(), tool_access)
            .with_autonomy_mode(autonomy.effective_mode);
        if let Some(data_tools) = &self.data_tools {
            executor = executor.with_data_tools(
                context.data_access_for_conversation(project_id),
                data_tools.clone(),
            );
        }
        let credentials = match api_key {
            Some(api_key) => ProviderCredentials::bearer(api_key)?,
            None => ProviderCredentials::none(),
        };
        let assistant = AssistantService::new(provider, executor);
        let response = assistant
            .run(
                AssistantRequest::new(context.user_id, request.message.clone())
                    .with_history(resolved.history),
                &context.access_grant,
                credentials,
            )
            .await?;

        if let Some(conversation) = resolved.new_conversation.as_ref() {
            self.operations
                .create_ai_conversation(conversation, &ai_audit)
                .await?;
        }
        self.persist_tool_runs(context, conversation_id, project_id, &response, &ai_audit)
            .await?;
        let turn_response =
            AssistantTurnResponse::from_service(conversation_id, response, autonomy);
        self.persist_turn_messages(
            context,
            project_id,
            resolved.expected_last_sequence,
            &request.message,
            &turn_response,
            &ai_audit,
        )
        .await?;
        Ok(turn_response)
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
        authorize_conversation(context, &conversation)?;
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

    pub async fn list_conversations(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<AssistantConversationSummary>, AiWorkflowError> {
        if limit == 0 || limit > CONVERSATION_LIST_LIMIT {
            return Err(AiWorkflowError::InvalidConversationRequest);
        }
        if project_id.is_some_and(|id| !context.allows_project(id)) {
            return Err(AiWorkflowError::Forbidden);
        }
        let mut conversations = self
            .operations
            .list_ai_conversations(
                &AiConversationFilter {
                    lab_id: context.lab_id,
                    user_id: context.user_id,
                    project_id,
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
        let now = Utc::now();
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
                    tool_run.output =
                        Some(json!({"draft": draft, "measurement_id": measurement.id}));
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
                (DraftKind::BulkImport, ToolName::ImportCommitDraft) => {
                    let backend = self
                        .data_tools
                        .as_ref()
                        .ok_or(AiWorkflowError::UnsupportedDraftOperation)?;
                    let result = backend
                        .apply_import_draft(&context.data_access(), &draft, human_audit)
                        .await;
                    match result {
                        Ok(result) => {
                            draft.mark_applied(draft.revision())?;
                            tool_run.status = ToolRunStatus::Completed;
                            approval.requested_diff = json!({"draft": draft});
                            tool_run.output = Some(json!({
                                "draft": draft,
                                "job_id": result.job_id,
                                "result": result.result,
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
                            Ok(DraftDecisionResponse {
                                draft: WriteDraftSummary::from(&draft),
                                measurement_id: None,
                                job_id: Some(result.job_id),
                            })
                        }
                        Err(error) => {
                            // The human decision is final even when the bounded
                            // application operation fails. Persist an Approved
                            // (not Applied) draft and a failed tool run so the
                            // failure is auditable and never masquerades as a
                            // successful mutation.
                            tool_run.status = ToolRunStatus::Failed;
                            tool_run.error = Some(tool_error_code(&error));
                            approval.requested_diff = json!({"draft": draft});
                            tool_run.output = Some(json!({
                                "draft": draft,
                                "error": tool_error_code(&error),
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
            authorize_conversation(context, &conversation)?;
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
    async fn persist_turn_messages(
        &self,
        context: &AiExecutionContext,
        project_id: Option<Uuid>,
        expected_last_sequence: i64,
        user_content: &str,
        response: &AssistantTurnResponse,
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
            .append_ai_turn_messages(
                &user_message,
                &assistant_message,
                expected_last_sequence,
                audit,
            )
            .await?;
        Ok(())
    }

    async fn persist_tool_runs(
        &self,
        context: &AiExecutionContext,
        conversation_id: Uuid,
        project_id: Option<Uuid>,
        response: &crate::AssistantResponse,
        audit: &AuditContext,
    ) -> Result<(), AiWorkflowError> {
        let now = Utc::now();
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
            self.operations.create_tool_run(&tool_run, audit).await?;
            if let Some(draft) = draft {
                let approval = Approval {
                    id: draft.id(),
                    tool_run_id: tool_run.id,
                    requested_diff: json!({"draft": draft}),
                    decision: StoredApprovalDecision::Pending,
                    decided_by: None,
                    decided_at: None,
                    reason: None,
                    meta: RecordMeta::new(now),
                };
                self.operations.create_approval(&approval, audit).await?;
            }
        }
        Ok(())
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

fn tool_error_code(error: &ToolExecutionError) -> String {
    match error {
        ToolExecutionError::Rejected { code } => code.clone(),
        ToolExecutionError::Unavailable => "data_tool_unavailable".to_owned(),
    }
}

fn conversation_title(message: &str) -> String {
    let title = message.trim().chars().take(80).collect::<String>();
    if title.is_empty() {
        "MuriArc AI conversation".to_owned()
    } else {
        title
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
    Store(#[from] StoreError),
    #[error(transparent)]
    Approval(#[from] ApprovalError),
    #[error(transparent)]
    DataTool(#[from] ToolExecutionError),
    #[error(transparent)]
    Credential(#[from] crate::CredentialError),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiDataApplyResult, CompletionResponse, DomainToolOutput, DomainToolRequest, FieldChange,
        MockProvider, ProviderToolCall, ScopeSet, ToolScope,
    };
    use async_trait::async_trait;
    use muriarc_core::{Lab, Project, User};
    use muriarc_store_sqlite::SqliteStore;

    struct FakeImportBackend {
        job_id: Uuid,
        project_id: Uuid,
        fail_apply: bool,
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
            _audit: &AuditContext,
        ) -> Result<AiDataApplyResult, ToolExecutionError> {
            assert_eq!(draft.status(), DraftStatus::Approved);
            if self.fail_apply {
                Err(ToolExecutionError::Rejected {
                    code: "stale_import_fixture".to_owned(),
                })
            } else {
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
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read, ToolScope::WriteDraft])),
        )
    }

    #[test]
    fn lab_wide_turn_is_read_only_even_when_user_can_write_projects() {
        let projects = [Uuid::new_v4(), Uuid::new_v4()];
        let context = context(projects);

        let (readable, writable) = scoped_project_access(&context, None);

        assert_eq!(readable, BTreeSet::from(projects));
        assert!(writable.is_empty());
    }

    #[test]
    fn explicit_project_turn_keeps_only_that_projects_write_scope() {
        let projects = [Uuid::new_v4(), Uuid::new_v4()];
        let context = context(projects);

        let (readable, writable) = scoped_project_access(&context, Some(projects[1]));

        assert_eq!(readable, BTreeSet::from([projects[1]]));
        assert_eq!(writable, BTreeSet::from([projects[1]]));
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
            .list_conversations(&context, Some(project.id), 20)
            .await
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].revision, 3);
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
                },
            )
            .await;
        assert!(matches!(failed, Err(AiWorkflowError::Assistant(_))));
        assert_eq!(
            workflow
                .list_conversations(&context, Some(project.id), 20)
                .await
                .unwrap()
                .len(),
            1,
            "failed provider turns must not create resumable conversations"
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
