use std::{path::Path, sync::Arc};

use muriarc_ai::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiExecutionContext, AiWorkflowError,
    AiWorkflowService, ApprovalDecision, ApprovalRequirement, AssistantConversationDetail,
    AssistantConversationSummary, AssistantTurnRequest, AssistantTurnResponse,
    DraftDecisionRequest, DraftDecisionResponse, DraftStatus, ScopeSet, ToolScope,
    WriteDraftSummary,
};
use muriarc_core::{
    Actor, AiAutonomyMode, AiConversationArchiveFilter, AiConversationChange, AiOperationStore,
    AuditContext, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, StoreError, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ai_data_tools::DesktopAiDataTools,
    ai_source_resolver::DesktopAiSourceResolver,
    data::DesktopDataState,
    settings::{SettingsError, SettingsService},
};

#[derive(Clone)]
pub(crate) struct DesktopAiState {
    store: Arc<SqliteStore>,
    workflow: AiWorkflowService,
    settings: SettingsService,
}

impl DesktopAiState {
    pub(crate) async fn initialize(
        data: DesktopDataState,
        app_data_dir: &Path,
    ) -> Result<Self, DesktopAiError> {
        let store = Arc::new(data.store_ref().clone());
        let domain_store: Arc<dyn MuriArcStore> = store.clone();
        let operation_store: Arc<dyn AiOperationStore> = store.clone();
        let source_resolver = Arc::new(DesktopAiSourceResolver::new(data.clone()));
        let data_tools = Arc::new(DesktopAiDataTools::new(data));
        Ok(Self {
            store,
            workflow: AiWorkflowService::new(domain_store, operation_store)
                .with_data_tools(data_tools)
                .with_source_resolver(source_resolver),
            settings: SettingsService::for_app_data(app_data_dir),
        })
    }

    pub(crate) async fn turn(
        &self,
        request: AssistantTurnRequest,
    ) -> Result<AssistantTurnResponse, DesktopAiError> {
        let resolved = self.settings.resolve_provider()?;
        let context = self.context().await?;
        self.workflow
            .run_turn_with_config(
                resolved.provider,
                resolved.api_key.as_ref().map(|secret| secret.as_str()),
                &context,
                request,
                resolved.runtime,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn list_conversations(
        &self,
        project_id: Option<Uuid>,
        title_query: Option<String>,
        archive: AiConversationArchiveFilter,
        limit: u32,
    ) -> Result<Vec<AssistantConversationSummary>, DesktopAiError> {
        self.workflow
            .list_conversations(
                &self.context().await?,
                project_id,
                title_query,
                archive,
                limit,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn create_conversation(
        &self,
        input: DesktopConversationCreateInput,
    ) -> Result<AssistantConversationSummary, DesktopAiError> {
        let context = self.context().await?;
        let project_id = input.project_id.as_deref().map(parse_uuid).transpose()?;
        let audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("create_ai_conversation".to_owned()),
        };
        self.workflow
            .create_conversation(&context, project_id, input.title, &audit)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn update_conversation(
        &self,
        conversation_id: Uuid,
        input: DesktopConversationUpdateInput,
    ) -> Result<AssistantConversationSummary, DesktopAiError> {
        let context = self.context().await?;
        let audit = AuditContext {
            actor: Actor::human(context.user_id, context.user_display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("update_ai_conversation".to_owned()),
        };
        let (expected_revision, change) = input.into_change()?;
        self.workflow
            .update_conversation(&context, conversation_id, expected_revision, change, &audit)
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
        if input.mode == AiAutonomyMode::Full && !input.declared {
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
                input.mode == AiAutonomyMode::Full && input.declared,
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
        .with_governance_reads(true, true))
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopAutonomyInput {
    pub mode: AiAutonomyMode,
    pub expected_revision: i64,
    #[serde(default)]
    pub declared: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopConversationUpdateAction {
    Rename,
    Pin,
    Unpin,
    Archive,
    Unarchive,
}

/// Renderer input contains only the requested scope and display title. Actor,
/// source, lab and owner are always supplied by the trusted native state.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopConversationCreateInput {
    pub project_id: Option<String>,
    pub title: String,
}

/// Renderer input is deliberately smaller than the core tagged enum. It can
/// request one metadata action but cannot provide actor, source or audit data.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopConversationUpdateInput {
    pub action: DesktopConversationUpdateAction,
    pub title: Option<String>,
    pub expected_revision: i64,
}

impl DesktopConversationUpdateInput {
    fn into_change(self) -> Result<(i64, AiConversationChange), DesktopAiError> {
        let change = match (self.action, self.title) {
            (DesktopConversationUpdateAction::Rename, Some(title)) => {
                AiConversationChange::Rename { title }
            }
            (DesktopConversationUpdateAction::Rename, None) => {
                return Err(DesktopAiError::InvalidConversationUpdate);
            }
            (DesktopConversationUpdateAction::Pin, None) => AiConversationChange::Pin,
            (DesktopConversationUpdateAction::Unpin, None) => AiConversationChange::Unpin,
            (DesktopConversationUpdateAction::Archive, None) => AiConversationChange::Archive,
            (DesktopConversationUpdateAction::Unarchive, None) => AiConversationChange::Unarchive,
            (_, Some(_)) => return Err(DesktopAiError::InvalidConversationUpdate),
        };
        Ok((self.expected_revision, change))
    }
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
    #[error("invalid AI conversation update")]
    InvalidConversationUpdate,
    #[error("AI operation is forbidden")]
    Forbidden,
    #[error("加强确认必须填写不超过 500 字的人工声明")]
    InvalidApprovalStatement,
    #[error("Full 模式需要明确确认其仅适用于当前会话，且不会绕过人工审批边界")]
    AutonomyDeclarationRequired,
}

impl DesktopAiError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidId
            | Self::InvalidConversationUpdate
            | Self::InvalidApprovalStatement
            | Self::AutonomyDeclarationRequired => "validation_error",
            Self::Forbidden => "forbidden",
            Self::Settings(SettingsError::Disabled | SettingsError::MissingCredential) => {
                "ai_not_configured"
            }
            Self::Settings(error) if error.is_validation() => "validation_error",
            Self::Workflow(error) => error.code(),
            Self::Store(StoreError::NotFound { .. }) => "not_found",
            Self::Store(StoreError::Conflict(_)) => "conflict",
            Self::Store(StoreError::Validation(_)) => "validation_error",
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_))
            | Self::Settings(_) => "storage_error",
        }
    }

    pub(crate) fn safe_message(&self) -> String {
        match self.code() {
            "storage_error" => "本地 AI 数据或安全存储操作失败".to_owned(),
            "request_timeout" => "AI 请求已超过受控执行时限，请稍后重试".to_owned(),
            "context_exceeded" => "当前问题与必要上下文超过模型输入上限，请缩小问题范围".to_owned(),
            "ai_unavailable"
            | "ai_data_unavailable"
            | "provider_unreachable"
            | "provider_transport_error"
            | "provider_http_error"
            | "provider_unavailable"
            | "response_format_incompatible"
            | "response_too_large"
            | "output_budget_exhausted" => "AI Provider 暂时不可用，请检查配置后重试".to_owned(),
            "api_key_rejected" => "AI Provider 拒绝了当前凭据，请重新配置密钥".to_owned(),
            "model_not_found" => "当前配置的 AI 模型不可用，请检查模型设置".to_owned(),
            "invalid_provider" => "AI Provider 配置无效，请检查设置".to_owned(),
            "ai_not_configured" => "请先启用 AI 并配置所需密钥".to_owned(),
            "invalid_ai_source" => {
                "所选文件已过期、范围不匹配，或无法通过完整性校验，请重新上传".to_owned()
            }
            "iteration_limit_exceeded" => {
                "AI 在完成任何工具结果前达到了受控迭代上限，请缩小问题范围后重试".to_owned()
            }
            "tool_call_limit_exceeded" => {
                "AI 在完成任何工具结果前达到了受控工具调用上限，请缩小问题范围后重试".to_owned()
            }
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
    use muriarc_ai::AssistantError;

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
            "validation_error"
        );
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::InvalidStoredConversation).code(),
            "storage_error"
        );
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::Assistant(
                AssistantError::TotalTimeoutExceeded
            ))
            .code(),
            "request_timeout"
        );
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::DataTool(
                muriarc_ai::ToolExecutionError::Unavailable
            ))
            .code(),
            "ai_data_unavailable"
        );
    }

    #[test]
    fn desktop_conversation_update_input_cannot_smuggle_audit_authority() {
        let input: DesktopConversationUpdateInput = serde_json::from_value(serde_json::json!({
            "action": "rename",
            "title": "  Updated title  ",
            "expectedRevision": 3
        }))
        .unwrap();
        let (revision, change) = input.into_change().unwrap();
        assert_eq!(revision, 3);
        assert_eq!(
            change,
            AiConversationChange::Rename {
                title: "  Updated title  ".to_owned()
            }
        );

        for unsafe_input in [
            serde_json::json!({
                "action": "pin",
                "expectedRevision": 3,
                "actor": {"actorType": "human"}
            }),
            serde_json::json!({
                "action": "pin",
                "expected_revision": 3
            }),
        ] {
            assert!(
                serde_json::from_value::<DesktopConversationUpdateInput>(unsafe_input).is_err()
            );
        }
        let unexpected_title: DesktopConversationUpdateInput =
            serde_json::from_value(serde_json::json!({
                "action": "archive",
                "title": "not accepted",
                "expectedRevision": 3
            }))
            .unwrap();
        assert!(matches!(
            unexpected_title.into_change(),
            Err(DesktopAiError::InvalidConversationUpdate)
        ));
        assert_eq!(
            serde_json::from_value::<AiConversationArchiveFilter>(serde_json::json!("archived"))
                .unwrap(),
            AiConversationArchiveFilter::Archived
        );
    }

    #[test]
    fn desktop_conversation_create_input_cannot_smuggle_identity_or_audit() {
        let project_id = Uuid::new_v4();
        let input: DesktopConversationCreateInput = serde_json::from_value(serde_json::json!({
            "projectId": project_id,
            "title": "Source review"
        }))
        .unwrap();
        assert_eq!(input.project_id, Some(project_id.to_string()));
        assert_eq!(input.title, "Source review");

        for unsafe_input in [
            serde_json::json!({
                "projectId": project_id,
                "title": "Source review",
                "userId": Uuid::new_v4()
            }),
            serde_json::json!({
                "projectId": project_id,
                "title": "Source review",
                "audit": {"source": "desktop"}
            }),
        ] {
            assert!(
                serde_json::from_value::<DesktopConversationCreateInput>(unsafe_input).is_err()
            );
        }
    }

    #[test]
    fn zero_progress_assistant_limits_have_stable_safe_codes() {
        let cases = [
            (
                AssistantError::IterationLimitExceeded,
                "iteration_limit_exceeded",
            ),
            (
                AssistantError::ToolCallLimitExceeded,
                "tool_call_limit_exceeded",
            ),
        ];

        for (error, expected) in cases {
            let error = DesktopAiError::Workflow(AiWorkflowError::Assistant(error));
            assert_eq!(error.code(), expected);
            assert!(!error.safe_message().contains("Provider"));
        }
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
