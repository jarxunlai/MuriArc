use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use muriarc_ai::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiExecutionContext, AiWorkflowError,
    AiWorkflowService, ApprovalDecision, ApprovalRequirement, AssistantConversationDetail,
    AssistantConversationStartRequest, AssistantConversationStartResponse,
    AssistantConversationSummary, AssistantTurnRequest, AssistantTurnResponse,
    DraftDecisionRequest, DraftDecisionResponse, DraftStatus, ScopeSet, ToolScope,
    WriteDraftSummary,
};
use muriarc_core::{
    Actor, AiAutonomyMode, AiModelProfileBinding, AiModelProfileStore, AiOperationStore,
    AuditContext, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, StoreError, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ai_data_tools::DesktopAiDataTools,
    data::DesktopDataState,
    settings::{SettingsError, SettingsService},
};

#[derive(Clone)]
pub(crate) struct DesktopAiState {
    store: Arc<SqliteStore>,
    workflow: AiWorkflowService,
    settings: SettingsService,
    startup: DesktopStartupAuthorization,
}

#[derive(Clone)]
struct DesktopStartupAuthorization {
    session_id: Uuid,
    full_declared: Arc<AtomicBool>,
    declaration_lock: Arc<Mutex<()>>,
}

impl DesktopStartupAuthorization {
    fn new() -> Self {
        Self {
            session_id: Uuid::new_v4(),
            full_declared: Arc::new(AtomicBool::new(false)),
            declaration_lock: Arc::new(Mutex::new(())),
        }
    }

    fn confirm_full(&self, confirm: impl FnOnce() -> bool) -> bool {
        let Ok(_guard) = self.declaration_lock.lock() else {
            return false;
        };
        if self.full_declared() {
            return true;
        }
        let confirmed = confirm();
        if confirmed {
            self.full_declared.store(true, Ordering::Release);
        }
        confirmed
    }

    fn full_declared(&self) -> bool {
        self.full_declared.load(Ordering::Acquire)
    }
}

impl DesktopAiState {
    pub(crate) async fn initialize(
        data: DesktopDataState,
        settings: SettingsService,
    ) -> Result<Self, DesktopAiError> {
        let store = Arc::new(data.store_ref().clone());
        let domain_store: Arc<dyn MuriArcStore> = store.clone();
        let operation_store: Arc<dyn AiOperationStore> = store.clone();
        let model_profiles: Arc<dyn AiModelProfileStore> = store.clone();
        let data_tools = Arc::new(DesktopAiDataTools::new(data));
        Ok(Self {
            store,
            workflow: AiWorkflowService::new(domain_store, operation_store)
                .with_model_profiles(model_profiles)
                .with_data_tools(data_tools),
            settings,
            startup: DesktopStartupAuthorization::new(),
        })
    }

    pub(crate) fn confirm_full_startup(
        &self,
        confirm: impl FnOnce() -> bool,
    ) -> Result<(), DesktopAiError> {
        self.startup
            .confirm_full(confirm)
            .then_some(())
            .ok_or(DesktopAiError::AutonomyDeclarationRequired)
    }

    pub(crate) async fn start_conversation(
        &self,
        input: DesktopConversationStartInput,
    ) -> Result<AssistantConversationStartResponse, DesktopAiError> {
        let context = self.context().await?;
        let full = input.requested_mode == AiAutonomyMode::Full;
        if full && !self.startup.full_declared() {
            return Err(DesktopAiError::AutonomyDeclarationRequired);
        }
        let model_profile = {
            let _profile_operation = self.settings.profile_coordinator().lock().await;
            let (profile_id, explicitly_selected) = match input.model_profile_id {
                Some(profile_id) if !profile_id.is_nil() => (profile_id, true),
                Some(_) => return Err(start_profile_unavailable(true, false)),
                None => (
                    self.store
                        .get_ai_user_model_defaults(LOCAL_USER_ID)
                        .await?
                        .and_then(|defaults| defaults.default_conversation_profile_id)
                        .ok_or(DesktopAiError::ModelSelectionRequired)?,
                    false,
                ),
            };
            let profile = match self.store.get_ai_model_profile(profile_id).await {
                Ok(profile) => profile,
                Err(StoreError::NotFound { .. }) if explicitly_selected => {
                    return Err(start_profile_unavailable(true, false));
                }
                Err(StoreError::NotFound { .. }) => {
                    return Err(start_profile_unavailable(false, false));
                }
                Err(error) => return Err(error.into()),
            };
            if profile.lab_id != LOCAL_LAB_ID
                || profile.user_id != LOCAL_USER_ID
                || profile.meta.deleted_at.is_some()
            {
                return Err(start_profile_unavailable(explicitly_selected, false));
            }
            if profile.archived_at.is_some() {
                return Err(start_profile_unavailable(explicitly_selected, true));
            }
            let binding = AiModelProfileBinding {
                profile_id,
                profile_version: profile.current_version,
            };
            // Resolve the exact immutable version and credential before the
            // conversation is persisted. This performs no Provider request.
            if let Err(error) = self
                .settings
                .resolve_provider_for_profile(self.store.as_ref(), binding)
                .await
            {
                return Err(start_model_resolution_error(error, explicitly_selected));
            }
            binding
        };
        let audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("start_ai_conversation".to_owned()),
        };
        self.workflow
            .start_conversation(
                &context,
                model_profile,
                AssistantConversationStartRequest {
                    project_id: input.project_id,
                    requested_mode: input.requested_mode,
                },
                full && self.startup.full_declared(),
                &audit,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn turn(
        &self,
        request: AssistantTurnRequest,
    ) -> Result<AssistantTurnResponse, DesktopAiError> {
        let context = self.context().await?;
        let (model_profile, resolved) = {
            let _profile_operation = self.settings.profile_coordinator().lock().await;
            let model_profile = match request.conversation_id {
                Some(conversation_id) => {
                    self.workflow
                        .conversation_model_profile(&context, conversation_id)
                        .await?
                }
                None => {
                    let defaults = self
                        .store
                        .get_ai_user_model_defaults(LOCAL_USER_ID)
                        .await?
                        .ok_or(SettingsError::DefaultModelNotConfigured)?;
                    let profile_id = defaults
                        .default_conversation_profile_id
                        .ok_or(SettingsError::DefaultModelNotConfigured)?;
                    let profile = self.store.get_ai_model_profile(profile_id).await?;
                    if profile.lab_id != LOCAL_LAB_ID
                        || profile.user_id != LOCAL_USER_ID
                        || profile.archived_at.is_some()
                        || profile.meta.deleted_at.is_some()
                    {
                        return Err(SettingsError::DefaultModelNotConfigured.into());
                    }
                    AiModelProfileBinding {
                        profile_id,
                        profile_version: profile.current_version,
                    }
                }
            };
            let resolved = self
                .settings
                .resolve_provider_for_profile(self.store.as_ref(), model_profile)
                .await?;
            (model_profile, resolved)
        };
        self.workflow
            .run_turn_with_config(
                resolved.provider,
                resolved.api_key.as_ref().map(|secret| secret.as_str()),
                &context,
                model_profile,
                request,
                resolved.runtime,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn list_conversations(
        &self,
        project_id: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<AssistantConversationSummary>, DesktopAiError> {
        self.workflow
            .list_conversations(&self.context().await?, project_id, limit)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn get_conversation(
        &self,
        conversation_id: Uuid,
        limit: u32,
    ) -> Result<AssistantConversationDetail, DesktopAiError> {
        self.workflow
            .get_conversation(&self.context().await?, conversation_id, limit)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn get_autonomy(
        &self,
        conversation_id: Uuid,
    ) -> Result<AiAutonomyView, DesktopAiError> {
        self.workflow
            .get_autonomy(&self.context().await?, conversation_id)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn set_autonomy(
        &self,
        conversation_id: Uuid,
        input: DesktopAutonomyInput,
    ) -> Result<AiAutonomyView, DesktopAiError> {
        let context = self.context().await?;
        if input.mode == AiAutonomyMode::Full && !self.startup.full_declared() {
            return Err(DesktopAiError::AutonomyDeclarationRequired);
        }
        let audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("update_ai_autonomy".to_owned()),
        };
        self.workflow
            .set_autonomy(
                &context,
                conversation_id,
                AiAutonomyUpdateRequest {
                    mode: input.mode,
                    expected_revision: input.expected_revision,
                },
                input.mode == AiAutonomyMode::Full && self.startup.full_declared(),
                &audit,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn list_drafts(
        &self,
        project_id: Option<Uuid>,
        status: Option<DraftStatus>,
    ) -> Result<Vec<WriteDraftSummary>, DesktopAiError> {
        self.workflow
            .list_drafts(&self.context().await?, project_id, status)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn get_draft(
        &self,
        draft_id: Uuid,
    ) -> Result<WriteDraftSummary, DesktopAiError> {
        self.workflow
            .get_draft(&self.context().await?, draft_id)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn decide_draft(
        &self,
        draft_id: Uuid,
        input: DesktopDraftDecisionInput,
    ) -> Result<DraftDecisionResponse, DesktopAiError> {
        let context = self.context().await?;
        // Re-read the persisted draft inside the trusted Tauri state. The webview
        // cannot assert step_up_verified or provide a Server password in local mode.
        let draft = self.workflow.get_draft(&context, draft_id).await?;
        let request = trusted_desktop_decision(draft.requirement, input)?;
        let audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("review_ai_write_draft".to_owned()),
        };
        self.workflow
            .decide_draft(&context, draft_id, request, &audit)
            .await
            .map_err(Into::into)
    }

    async fn context(&self) -> Result<AiExecutionContext, DesktopAiError> {
        let user = self.store.get_user(LOCAL_USER_ID).await?;
        if user.lab_id != LOCAL_LAB_ID || user.meta.deleted_at.is_some() {
            return Err(DesktopAiError::Forbidden);
        }
        let project_ids = self
            .store
            .list_projects(LOCAL_LAB_ID)
            .await?
            .into_iter()
            .map(|project| project.id)
            .collect::<Vec<_>>();
        Ok(AiExecutionContext::new(
            LOCAL_LAB_ID,
            LOCAL_USER_ID,
            user.display_name,
            Uuid::new_v4().to_string(),
            project_ids.iter().copied(),
            project_ids.iter().copied(),
            true,
            AccessGrant::local_user(ScopeSet::new([
                ToolScope::Read,
                ToolScope::WriteDraft,
                ToolScope::Import,
                ToolScope::Export,
            ])),
        )
        .with_data_access(
            project_ids.iter().copied(),
            project_ids.iter().copied(),
            true,
        )
        .with_autonomy_context(Some(self.startup.session_id), AiAutonomyMode::Full))
    }
}

/// Desktop-only approval input. Unknown transport fields are rejected, so a
/// renderer/model cannot smuggle `stepUpVerified` or a Server password into IPC.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopDraftDecisionInput {
    pub expected_revision: u64,
    pub decision: ApprovalDecision,
    pub statement: Option<String>,
}

/// The renderer may select only an editable profile identity and requested
/// mode. Exact versions, startup proof and the process Session UUID are native.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopConversationStartInput {
    pub project_id: Option<Uuid>,
    pub model_profile_id: Option<Uuid>,
    pub requested_mode: AiAutonomyMode,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopAutonomyInput {
    pub mode: AiAutonomyMode,
    pub expected_revision: i64,
}

fn trusted_desktop_decision(
    requirement: ApprovalRequirement,
    input: DesktopDraftDecisionInput,
) -> Result<DraftDecisionRequest, DesktopAiError> {
    let statement = input
        .statement
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if statement
        .as_ref()
        .is_some_and(|value| value.chars().count() > 500)
    {
        return Err(DesktopAiError::InvalidApprovalStatement);
    }
    let reinforced_approval = input.decision == ApprovalDecision::Approve
        && requirement == ApprovalRequirement::ReinforcedConfirmation;
    if reinforced_approval && statement.is_none() {
        return Err(DesktopAiError::InvalidApprovalStatement);
    }
    Ok(DraftDecisionRequest {
        expected_revision: input.expected_revision,
        decision: input.decision,
        statement,
        // Only this native adapter constructs the proof after re-reading the
        // draft and validating an explicit local researcher declaration.
        step_up_verified: reinforced_approval,
    })
}

#[derive(Debug, Error)]
pub(crate) enum DesktopAiError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Workflow(#[from] AiWorkflowError),
    #[error("invalid AI draft identifier")]
    InvalidId,
    #[error("AI operation is forbidden")]
    Forbidden,
    #[error("加强确认必须填写不超过 500 字的人工声明")]
    InvalidApprovalStatement,
    #[error("Full 模式需要明确确认其仅适用于当前会话，且不会绕过人工审批边界")]
    AutonomyDeclarationRequired,
    #[error("请选择一个可用的对话模型")]
    ModelSelectionRequired,
    #[error("所选模型已停用")]
    SelectedModelArchived,
    #[error("所选模型已停用或不可用")]
    SelectedModelUnavailable,
}

fn start_model_resolution_error(error: SettingsError, explicitly_selected: bool) -> DesktopAiError {
    if matches!(
        &error,
        SettingsError::Storage
            | SettingsError::CredentialStore
            | SettingsError::ModelProfileStore(
                StoreError::Database(_) | StoreError::Serialization(_)
            )
    ) {
        DesktopAiError::Settings(error)
    } else if explicitly_selected {
        DesktopAiError::SelectedModelUnavailable
    } else {
        DesktopAiError::ModelSelectionRequired
    }
}

fn start_profile_unavailable(explicitly_selected: bool, archived: bool) -> DesktopAiError {
    match (explicitly_selected, archived) {
        (false, _) => DesktopAiError::ModelSelectionRequired,
        (true, true) => DesktopAiError::SelectedModelArchived,
        (true, false) => DesktopAiError::SelectedModelUnavailable,
    }
}

impl DesktopAiError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidId | Self::InvalidApprovalStatement => "validation",
            Self::AutonomyDeclarationRequired => "autonomy_declaration_required",
            Self::ModelSelectionRequired => "model_selection_required",
            Self::SelectedModelArchived
            | Self::Workflow(AiWorkflowError::ConversationModelArchived) => "model_archived",
            Self::SelectedModelUnavailable
            | Self::Workflow(AiWorkflowError::ConversationModelUnavailable) => "model_unavailable",
            Self::Forbidden | Self::Workflow(AiWorkflowError::Forbidden) => "forbidden",
            Self::Settings(
                SettingsError::MissingCredential | SettingsError::DefaultModelNotConfigured,
            ) => "ai_not_configured",
            Self::Settings(error) if error.is_validation() => "validation",
            Self::Workflow(AiWorkflowError::Assistant(_))
            | Self::Workflow(AiWorkflowError::DataTool(
                muriarc_ai::ToolExecutionError::Unavailable,
            )) => "ai_unavailable",
            Self::Store(StoreError::NotFound { .. })
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::NotFound { .. })) => {
                "not_found"
            }
            Self::Store(StoreError::Conflict(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Conflict(_))) => {
                "conflict"
            }
            Self::Store(StoreError::Validation(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Validation(_)))
            | Self::Workflow(AiWorkflowError::Config(_))
            | Self::Workflow(AiWorkflowError::Approval(_))
            | Self::Workflow(AiWorkflowError::InvalidStoredDraft)
            | Self::Workflow(AiWorkflowError::UnsupportedDraftOperation)
            | Self::Workflow(AiWorkflowError::InvalidConversationRequest)
            | Self::Workflow(AiWorkflowError::LegacyConversationReadOnly)
            | Self::Workflow(AiWorkflowError::ConversationModelProfileMismatch)
            | Self::Workflow(AiWorkflowError::DataTool(
                muriarc_ai::ToolExecutionError::Rejected { .. },
            ))
            | Self::Workflow(AiWorkflowError::Credential(_)) => "validation",
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_))
            | Self::Settings(SettingsError::ModelProfileStore(
                StoreError::Database(_) | StoreError::Serialization(_),
            ))
            | Self::Settings(_)
            | Self::Workflow(AiWorkflowError::Store(_))
            | Self::Workflow(AiWorkflowError::InvalidStoredConversation) => "storage_error",
        }
    }

    pub(crate) fn safe_message(&self) -> String {
        match self.code() {
            "storage_error" => "本地 AI 数据或安全存储操作失败".to_owned(),
            "ai_unavailable" => "AI Provider 暂时不可用，请检查配置后重试".to_owned(),
            "ai_not_configured" => "请先启用 AI 并配置所需密钥".to_owned(),
            "model_selection_required" => "请选择一个可用的对话模型".to_owned(),
            "model_archived" => "该会话绑定的模型已停用，只能查看历史记录".to_owned(),
            "model_unavailable" => "该会话绑定的模型当前不可用，只能查看历史记录".to_owned(),
            _ => self.to_string(),
        }
    }
}

pub(crate) fn parse_uuid(value: &str) -> Result<Uuid, DesktopAiError> {
    Uuid::parse_str(value).map_err(|_| DesktopAiError::InvalidId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failures_do_not_echo_secrets() {
        let error = DesktopAiError::Settings(SettingsError::CredentialStore);
        assert_eq!(error.code(), "storage_error");
        assert!(!error.safe_message().to_ascii_lowercase().contains("key"));
    }

    #[test]
    fn ai_actor_is_never_used_as_human_approver() {
        let actor = Actor {
            actor_type: muriarc_core::ActorType::Ai,
            user_id: Some(LOCAL_USER_ID),
            display_name: "AI".to_owned(),
        };
        assert_ne!(actor.actor_type, muriarc_core::ActorType::Human);
    }

    #[test]
    fn conversation_errors_have_stable_safe_codes() {
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::InvalidConversationRequest).code(),
            "validation"
        );
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::InvalidStoredConversation).code(),
            "storage_error"
        );
        assert_eq!(
            DesktopAiError::Settings(SettingsError::ModelProfileStore(StoreError::NotFound {
                entity: "ai_model_profile",
                id: Uuid::nil(),
            }))
            .code(),
            "not_found"
        );
        assert_eq!(
            DesktopAiError::Settings(SettingsError::ModelProfileStore(StoreError::Conflict(
                "stale model profile".to_owned(),
            )))
            .code(),
            "conflict"
        );
        assert_eq!(
            DesktopAiError::Settings(SettingsError::ModelProfileStore(StoreError::Validation(
                "invalid model profile".to_owned(),
            )))
            .code(),
            "validation"
        );
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::ConversationModelArchived).code(),
            "model_archived"
        );
        assert_eq!(
            DesktopAiError::ModelSelectionRequired.code(),
            "model_selection_required"
        );
        assert_eq!(
            DesktopAiError::SelectedModelArchived.code(),
            "model_archived"
        );
        assert_eq!(
            start_model_resolution_error(SettingsError::MissingCredential, false).code(),
            "model_selection_required"
        );
        assert_eq!(
            start_model_resolution_error(SettingsError::MissingCredential, true).code(),
            "model_unavailable"
        );
        assert_eq!(
            start_profile_unavailable(false, true).code(),
            "model_selection_required"
        );
        assert_eq!(
            start_profile_unavailable(true, true).code(),
            "model_archived"
        );
        assert_eq!(
            start_profile_unavailable(true, false).code(),
            "model_unavailable"
        );
    }

    #[test]
    fn desktop_startup_authorization_is_process_local_and_dtos_reject_proof_fields() {
        let first = DesktopStartupAuthorization::new();
        let second = DesktopStartupAuthorization::new();
        assert_ne!(first.session_id, second.session_id);
        assert!(!first.full_declared());
        assert!(!second.full_declared());
        assert!(!first.confirm_full(|| false));
        assert!(!first.full_declared());
        assert!(first.confirm_full(|| true));
        assert!(first.full_declared());
        assert!(first.confirm_full(|| panic!("confirmed startup must not prompt twice")));
        assert!(!second.full_declared());

        let valid = serde_json::json!({
            "projectId": null,
            "modelProfileId": null,
            "requestedMode": "full",
        });
        assert!(serde_json::from_value::<DesktopConversationStartInput>(valid).is_ok());
        for forbidden in ["declared", "sessionId", "currentPassword", "stepUpVerified"] {
            let mut value = serde_json::json!({
                "projectId": null,
                "modelProfileId": null,
                "requestedMode": "full",
            });
            value
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_owned(), serde_json::json!(true));
            assert!(
                serde_json::from_value::<DesktopConversationStartInput>(value).is_err(),
                "{forbidden} must not cross the Tauri start contract"
            );
        }

        let forged_autonomy = serde_json::json!({
            "mode": "full",
            "expectedRevision": 0,
            "declared": true,
        });
        assert!(serde_json::from_value::<DesktopAutonomyInput>(forged_autonomy).is_err());
    }

    #[test]
    fn desktop_approval_dto_rejects_client_step_up_and_constructs_it_natively() {
        let unsafe_input = serde_json::json!({
            "expectedRevision": 0,
            "decision": "approve",
            "statement": "I reviewed the import",
            "stepUpVerified": true,
        });
        assert!(serde_json::from_value::<DesktopDraftDecisionInput>(unsafe_input).is_err());
        let unsafe_password = serde_json::json!({
            "expectedRevision": 0,
            "decision": "approve",
            "statement": "I reviewed the import",
            "currentPassword": "secret",
        });
        assert!(serde_json::from_value::<DesktopDraftDecisionInput>(unsafe_password).is_err());

        let request = trusted_desktop_decision(
            ApprovalRequirement::ReinforcedConfirmation,
            DesktopDraftDecisionInput {
                expected_revision: 2,
                decision: ApprovalDecision::Approve,
                statement: Some("  I reviewed the import diff  ".to_owned()),
            },
        )
        .unwrap();
        assert!(request.step_up_verified);
        assert_eq!(
            request.statement.as_deref(),
            Some("I reviewed the import diff")
        );

        let ordinary = trusted_desktop_decision(
            ApprovalRequirement::PreviewConfirmation,
            DesktopDraftDecisionInput {
                expected_revision: 1,
                decision: ApprovalDecision::Approve,
                statement: None,
            },
        )
        .unwrap();
        assert!(!ordinary.step_up_verified);
        assert!(matches!(
            trusted_desktop_decision(
                ApprovalRequirement::ReinforcedConfirmation,
                DesktopDraftDecisionInput {
                    expected_revision: 2,
                    decision: ApprovalDecision::Approve,
                    statement: Some("   ".to_owned()),
                },
            ),
            Err(DesktopAiError::InvalidApprovalStatement)
        ));
    }
}
