use std::{path::Path, sync::Arc};

use muriarc_ai::{
    AccessGrant, AiExecutionContext, AiWorkflowError, AiWorkflowService, ApprovalDecision,
    ApprovalRequirement, AssistantConversationDetail, AssistantConversationSummary,
    AssistantTurnRequest, AssistantTurnResponse, DraftDecisionRequest, DraftDecisionResponse,
    DraftStatus, ScopeSet, ToolScope, WriteDraftSummary,
};
use muriarc_core::{
    Actor, AiOperationStore, AuditContext, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, StoreError,
    WriteSource,
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
}

impl DesktopAiState {
    pub(crate) async fn initialize(
        data: DesktopDataState,
        app_data_dir: &Path,
    ) -> Result<Self, DesktopAiError> {
        let store = Arc::new(data.store_ref().clone());
        let domain_store: Arc<dyn MuriArcStore> = store.clone();
        let operation_store: Arc<dyn AiOperationStore> = store.clone();
        let data_tools = Arc::new(DesktopAiDataTools::new(data));
        Ok(Self {
            store,
            workflow: AiWorkflowService::new(domain_store, operation_store)
                .with_data_tools(data_tools),
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
            .run_turn(
                resolved.provider,
                resolved.api_key.as_ref().map(|secret| secret.as_str()),
                &context,
                request,
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
        ))
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
}

impl DesktopAiError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidId | Self::InvalidApprovalStatement => "validation",
            Self::Forbidden | Self::Workflow(AiWorkflowError::Forbidden) => "forbidden",
            Self::Settings(SettingsError::Disabled | SettingsError::MissingCredential) => {
                "ai_not_configured"
            }
            Self::Settings(error) if error.is_validation() => "validation",
            Self::Workflow(AiWorkflowError::Assistant(_))
            | Self::Workflow(AiWorkflowError::DataTool(
                muriarc_ai::ToolExecutionError::Unavailable,
            )) => "ai_unavailable",
            Self::Store(StoreError::NotFound { .. }) => "not_found",
            Self::Store(StoreError::Conflict(_)) => "conflict",
            Self::Store(StoreError::Validation(_))
            | Self::Workflow(AiWorkflowError::Approval(_))
            | Self::Workflow(AiWorkflowError::InvalidStoredDraft)
            | Self::Workflow(AiWorkflowError::UnsupportedDraftOperation)
            | Self::Workflow(AiWorkflowError::InvalidConversationRequest)
            | Self::Workflow(AiWorkflowError::DataTool(
                muriarc_ai::ToolExecutionError::Rejected { .. },
            ))
            | Self::Workflow(AiWorkflowError::Credential(_)) => "validation",
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_))
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
