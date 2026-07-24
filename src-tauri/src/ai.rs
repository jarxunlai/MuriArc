use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use muriarc_ai::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiExecutionContext, AiWorkflowError,
    AiWorkflowService, ApprovalDecision, ApprovalRequirement, AssistantConversationDetail,
    AssistantConversationStartRequest, AssistantConversationStartResponse,
    AssistantConversationSummary, AssistantTurnMedia, AssistantTurnRequest, AssistantTurnResponse,
    DraftDecisionRequest, DraftDecisionResponse, DraftStatus, ScopeSet, ToolScope,
    WriteDraftSummary,
};
use muriarc_core::{
    Actor, AiAutonomyMode, AiConversationArchiveFilter, AiConversationChange, AiExtractionDraft,
    AiModelProfileBinding, AiModelProfileStore, AiOperationStore, AppliedAiExtraction,
    AuditContext, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, StoreError, WriteSource,
};
use muriarc_data::AttachmentFileError;
use muriarc_store_sqlite::SqliteStore;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ai_data_tools::DesktopAiDataTools,
    ai_images::{
        ApproveAiExtractionInput, ArchivePrivateAiImageInput, CreateAiExtractionInput,
        DesktopAiImages, PrivateImageContent, PrivateImageView, RejectAiExtractionInput,
        UploadPrivateAiImageInput,
    },
    ai_source_resolver::DesktopAiSourceResolver,
    data::DesktopDataState,
    settings::{SettingsError, SettingsService},
};

#[derive(Clone)]
pub(crate) struct DesktopAiState {
    store: Arc<SqliteStore>,
    workflow: AiWorkflowService,
    settings: SettingsService,
    images: DesktopAiImages,
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
        let source_resolver = Arc::new(DesktopAiSourceResolver::new(data.clone()));
        let model_profiles: Arc<dyn AiModelProfileStore> = store.clone();
        let data_tools = Arc::new(DesktopAiDataTools::new(data.clone()));
        let images = DesktopAiImages::new(
            store.clone(),
            data.attachments_ref().clone(),
            settings.clone(),
        );
        Ok(Self {
            store,
            workflow: AiWorkflowService::new(domain_store, operation_store)
                .with_model_profiles(model_profiles)
                .with_data_tools(data_tools)
                .with_source_resolver(source_resolver),
            settings,
            images,
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
                    title: input.title,
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
        let model_profile = {
            let _profile_operation = self.settings.profile_coordinator().lock().await;
            self.workflow
                .conversation_model_profile(&context, request.conversation_id)
                .await?
        };
        let resolved = preflight_then_resolve(
            &self.workflow,
            &context,
            model_profile,
            &request,
            || async {
                let _profile_operation = self.settings.profile_coordinator().lock().await;
                self.settings
                    .resolve_provider_for_profile(self.store.as_ref(), model_profile)
                    .await
                    .map_err(Into::into)
            },
        )
        .await?;
        let source_bundle = self
            .workflow
            .resolve_turn_sources(&context, model_profile, &request)
            .await?;
        let source_images = source_bundle.images().to_vec();
        let images = if request.image_ids.is_empty() {
            Vec::new()
        } else {
            self.images
                .prepare_assistant_images(
                    &context,
                    Some(request.conversation_id),
                    request.project_id,
                    &request.image_ids,
                )
                .await?
        };
        let vision_route = plan_desktop_vision_route(
            !images.is_empty() || !source_images.is_empty(),
            resolved.supports_vision,
            request.vision_model_profile_id,
        )?;
        let relay_provider = match vision_route {
            DesktopVisionRoute::Relay(profile_id) => {
                Some(self.images.resolve_vision_provider(profile_id).await?)
            }
            DesktopVisionRoute::None | DesktopVisionRoute::Direct => None,
        };
        let media = match vision_route {
            DesktopVisionRoute::None => AssistantTurnMedia::default(),
            DesktopVisionRoute::Direct => {
                AssistantTurnMedia::direct_with_sources(images, source_images)?
            }
            DesktopVisionRoute::Relay(_) => {
                let (vision_binding, vision) =
                    relay_provider.ok_or(DesktopAiError::VisionModelSelectionRequired)?;
                let observation = self
                    .workflow
                    .observe_images_with_sources(
                        vision.provider,
                        vision.api_key.as_ref().map(|secret| secret.as_str()),
                        vision_binding,
                        &images,
                        &source_images,
                        vision.runtime,
                    )
                    .await?;
                AssistantTurnMedia::relayed_with_sources(images, source_images, observation)?
            }
        };
        self.workflow
            .run_turn_with_resolved_sources_config(
                resolved.provider,
                resolved.api_key.as_ref().map(|secret| secret.as_str()),
                &context,
                model_profile,
                request,
                resolved.runtime,
                media,
                source_bundle,
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn list_private_images(
        &self,
        conversation_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> Result<Vec<PrivateImageView>, DesktopAiError> {
        self.images.list(conversation_id, project_id).await
    }

    pub(crate) async fn upload_private_image(
        &self,
        input: UploadPrivateAiImageInput,
    ) -> Result<PrivateImageView, DesktopAiError> {
        let context = self.context().await?;
        self.images.upload(&self.workflow, &context, input).await
    }

    pub(crate) async fn read_private_image(
        &self,
        id: Uuid,
    ) -> Result<PrivateImageContent, DesktopAiError> {
        self.images.read(id).await
    }

    pub(crate) async fn archive_private_image(
        &self,
        id: Uuid,
        input: ArchivePrivateAiImageInput,
    ) -> Result<PrivateImageView, DesktopAiError> {
        let context = self.context().await?;
        self.images
            .archive(&self.workflow, &context, id, input)
            .await
    }

    pub(crate) async fn list_ai_extractions(
        &self,
        project_id: Option<Uuid>,
    ) -> Result<Vec<AiExtractionDraft>, DesktopAiError> {
        self.images.list_extractions(project_id).await
    }

    pub(crate) async fn create_ai_extraction(
        &self,
        input: CreateAiExtractionInput,
    ) -> Result<AiExtractionDraft, DesktopAiError> {
        let context = self.context().await?;
        self.images.create_extraction(&context, input).await
    }

    pub(crate) async fn approve_ai_extraction(
        &self,
        id: Uuid,
        input: ApproveAiExtractionInput,
    ) -> Result<AppliedAiExtraction, DesktopAiError> {
        let context = self.context().await?;
        self.images.approve_extraction(&context, id, input).await
    }

    pub(crate) async fn reject_ai_extraction(
        &self,
        id: Uuid,
        input: RejectAiExtractionInput,
    ) -> Result<AiExtractionDraft, DesktopAiError> {
        let context = self.context().await?;
        self.images.reject_extraction(&context, id, input).await
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
        .with_governance_reads(true, true)
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

/// The renderer may select only a bounded display title, editable profile
/// identity, project and requested mode. Exact versions, startup proof and the
/// process Session UUID are native.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopConversationStartInput {
    pub project_id: Option<Uuid>,
    pub title: Option<String>,
    pub model_profile_id: Option<Uuid>,
    pub requested_mode: AiAutonomyMode,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopAutonomyInput {
    pub mode: AiAutonomyMode,
    pub expected_revision: i64,
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
    #[error("请选择一个可用的对话模型")]
    ModelSelectionRequired,
    #[error("所选模型已停用")]
    SelectedModelArchived,
    #[error("所选模型已停用或不可用")]
    SelectedModelUnavailable,
    #[error("当前对话模型不支持视觉，请明确选择一个可用的视觉模型")]
    VisionModelSelectionRequired,
    #[error("所选视觉模型已停用、不支持视觉或不可用")]
    VisionModelUnavailable,
    #[error("图片证据不可用、不安全或不属于当前会话")]
    InvalidImageEvidence,
    #[error("AI 图片候选不符合当前数据单元约束")]
    InvalidExtraction,
    #[error("AI Provider request failed")]
    ProviderUnavailable,
    #[error(transparent)]
    ImageStorage(AttachmentFileError),
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

async fn preflight_then_resolve<T, Resolve, ResolveFuture>(
    workflow: &AiWorkflowService,
    context: &AiExecutionContext,
    model_profile: AiModelProfileBinding,
    request: &AssistantTurnRequest,
    resolve: Resolve,
) -> Result<T, DesktopAiError>
where
    Resolve: FnOnce() -> ResolveFuture,
    ResolveFuture: Future<Output = Result<T, DesktopAiError>>,
{
    workflow
        .preflight_turn_request(context, model_profile, request)
        .await?;
    resolve().await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopVisionRoute {
    None,
    Direct,
    Relay(Option<Uuid>),
}

fn plan_desktop_vision_route(
    has_images: bool,
    final_model_supports_vision: bool,
    requested_vision_profile_id: Option<Uuid>,
) -> Result<DesktopVisionRoute, DesktopAiError> {
    match (
        has_images,
        final_model_supports_vision,
        requested_vision_profile_id,
    ) {
        (false, _, None) => Ok(DesktopVisionRoute::None),
        (false, _, Some(_)) | (true, true, Some(_)) => Err(DesktopAiError::InvalidImageEvidence),
        (true, true, None) => Ok(DesktopVisionRoute::Direct),
        (true, false, profile_id) => Ok(DesktopVisionRoute::Relay(profile_id)),
    }
}

impl DesktopAiError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidId
            | Self::InvalidConversationUpdate
            | Self::InvalidApprovalStatement
            | Self::InvalidExtraction => "validation",
            Self::AutonomyDeclarationRequired => "autonomy_declaration_required",
            Self::ModelSelectionRequired => "model_selection_required",
            Self::VisionModelSelectionRequired => "vision_model_selection_required",
            Self::VisionModelUnavailable => "vision_model_unavailable",
            Self::InvalidImageEvidence | Self::Workflow(AiWorkflowError::InvalidImageEvidence) => {
                "image_evidence_invalid"
            }
            Self::Workflow(AiWorkflowError::InvalidVisionObservation) => "vision_response_invalid",
            Self::SelectedModelArchived
            | Self::Workflow(AiWorkflowError::ConversationModelArchived) => "model_archived",
            Self::SelectedModelUnavailable
            | Self::Workflow(AiWorkflowError::ConversationModelUnavailable) => "model_unavailable",
            Self::Forbidden | Self::Workflow(AiWorkflowError::Forbidden) => "forbidden",
            Self::Settings(SettingsError::MissingCredential) => "ai_not_configured",
            Self::Settings(error) if error.is_validation() => "validation",
            Self::ProviderUnavailable => "ai_unavailable",
            Self::Store(StoreError::NotFound { .. })
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::NotFound { .. })) => {
                "not_found"
            }
            Self::Store(StoreError::Conflict(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Conflict(_))) => {
                "conflict"
            }
            Self::Store(StoreError::Validation(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Validation(_))) => {
                "validation"
            }
            Self::Workflow(error) => error.code(),
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_))
            | Self::ImageStorage(_)
            | Self::Settings(SettingsError::ModelProfileStore(
                StoreError::Database(_) | StoreError::Serialization(_),
            ))
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
            "model_selection_required" => "请选择一个可用的对话模型".to_owned(),
            "vision_model_selection_required" => {
                "当前对话模型不支持视觉，请明确选择一个可用的视觉模型".to_owned()
            }
            "vision_model_unavailable" => "所选视觉模型已停用、不支持视觉或不可用".to_owned(),
            "image_evidence_invalid" => "图片证据不可用、不安全或不属于当前会话".to_owned(),
            "vision_response_invalid" => {
                "视觉模型返回了无效的受控观察，请重试或更换模型".to_owned()
            }
            "model_archived" => "该会话绑定的模型已停用，只能查看历史记录".to_owned(),
            "model_unavailable" => "该会话绑定的模型当前不可用，只能查看历史记录".to_owned(),
            "conversation_read_only" => "该历史会话无法证明原始模型版本，只能查看记录".to_owned(),
            "conversation_model_mismatch" => {
                "当前模型与会话绑定的精确版本不一致，已拒绝继续执行".to_owned()
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

    #[tokio::test]
    async fn invalid_turn_preflight_never_resolves_provider_credentials() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        let domain_store: Arc<dyn MuriArcStore> = store.clone();
        let operation_store: Arc<dyn AiOperationStore> = store;
        let workflow = AiWorkflowService::new(domain_store, operation_store);
        let context = AiExecutionContext::new(
            LOCAL_LAB_ID,
            LOCAL_USER_ID,
            "Local researcher",
            "desktop-preflight",
            [],
            [],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let resolver_calls = std::sync::atomic::AtomicUsize::new(0);
        let request = AssistantTurnRequest {
            conversation_id: Uuid::new_v4(),
            project_id: None,
            message: " \n ".to_owned(),
            source_refs: Vec::new(),
            image_ids: vec![Uuid::new_v4()],
            vision_model_profile_id: Some(Uuid::new_v4()),
        };

        let result = preflight_then_resolve(
            &workflow,
            &context,
            AiModelProfileBinding {
                profile_id: Uuid::new_v4(),
                profile_version: 1,
            },
            &request,
            || {
                resolver_calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<_, DesktopAiError>(()))
            },
        )
        .await;

        assert!(matches!(result, Err(DesktopAiError::Workflow(_))));
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 0);
    }

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
        assert_eq!(
            DesktopAiError::Workflow(AiWorkflowError::InvalidVisionObservation).code(),
            "vision_response_invalid"
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
    fn desktop_conversation_start_input_cannot_smuggle_identity_or_audit() {
        let project_id = Uuid::new_v4();
        let model_profile_id = Uuid::new_v4();
        let input: DesktopConversationStartInput = serde_json::from_value(serde_json::json!({
            "projectId": project_id,
            "title": "Source review",
            "modelProfileId": model_profile_id,
            "requestedMode": "ask"
        }))
        .unwrap();
        assert_eq!(input.project_id, Some(project_id));
        assert_eq!(input.title.as_deref(), Some("Source review"));
        assert_eq!(input.model_profile_id, Some(model_profile_id));

        for unsafe_input in [
            serde_json::json!({
                "projectId": project_id,
                "title": "Source review",
                "modelProfileId": model_profile_id,
                "requestedMode": "ask",
                "userId": Uuid::new_v4()
            }),
            serde_json::json!({
                "projectId": project_id,
                "title": "Source review",
                "modelProfileId": model_profile_id,
                "requestedMode": "ask",
                "audit": {"source": "desktop"}
            }),
        ] {
            assert!(serde_json::from_value::<DesktopConversationStartInput>(unsafe_input).is_err());
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
    fn desktop_vision_routing_distinguishes_direct_relay_and_invalid_requests() {
        let selected_vision_profile = Uuid::new_v4();
        assert_eq!(
            plan_desktop_vision_route(false, false, None).unwrap(),
            DesktopVisionRoute::None
        );
        assert_eq!(
            plan_desktop_vision_route(true, true, None).unwrap(),
            DesktopVisionRoute::Direct
        );
        assert_eq!(
            plan_desktop_vision_route(true, false, None).unwrap(),
            DesktopVisionRoute::Relay(None)
        );
        assert_eq!(
            plan_desktop_vision_route(true, false, Some(selected_vision_profile)).unwrap(),
            DesktopVisionRoute::Relay(Some(selected_vision_profile))
        );
        assert!(matches!(
            plan_desktop_vision_route(false, false, Some(selected_vision_profile)),
            Err(DesktopAiError::InvalidImageEvidence)
        ));
        assert!(matches!(
            plan_desktop_vision_route(true, true, Some(selected_vision_profile)),
            Err(DesktopAiError::InvalidImageEvidence)
        ));
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
            "title": "New conversation",
            "modelProfileId": null,
            "requestedMode": "full",
        });
        assert!(serde_json::from_value::<DesktopConversationStartInput>(valid).is_ok());
        for forbidden in ["declared", "sessionId", "currentPassword", "stepUpVerified"] {
            let mut value = serde_json::json!({
                "projectId": null,
                "title": "New conversation",
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
