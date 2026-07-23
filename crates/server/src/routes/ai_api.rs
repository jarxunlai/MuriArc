use std::{collections::BTreeSet, fmt, time::Instant};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post, put},
};
use muriarc_ai::{
    AccessGrant, AiAutonomyUpdateRequest, AiAutonomyView, AiExecutionContext, AiProvider,
    AiWorkflowError, AiWorkflowService, ApprovalDecision, ApprovalError, ApprovalRequirement,
    AssistantConversationDetail, AssistantConversationSummary, AssistantError,
    AssistantTurnRequest, AssistantTurnResponse, ChatMessage, CompletionRequest,
    DraftDecisionRequest, DraftDecisionResponse, DraftStatus, ProviderCredentials, ProviderError,
    ScopeSet, ToolScope, WriteDraftSummary,
};
use muriarc_core::{
    AiAutonomyMode, AiConversationArchiveFilter, AiConversationChange, Permission, StoreError,
};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AiLabSettingsView, AiProviderDiagnosticsView, AiProviderEndpointView, AiProviderPresetView,
    AiProviderSettingsView, AiProviderStoreError, ApiError, AppState, AuthError, AuthPrincipal,
    AuthenticationMethod, RequestMetadata, ResolvedAiProvider, SaveAiLabSettingsInput,
    SaveAiProviderEndpointInput, SaveAiProviderSettingsInput, ai_data_tools::ServerAiDataTools,
    ai_source_resolver::ServerAiSourceResolver, ai_step_up::AiStepUpLimit,
};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, collection, item, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/ai/settings",
            get(get_settings).put(save_settings).delete(clear_settings),
        )
        .route("/ai/settings/test", post(test_settings))
        .route("/ai/diagnostics", get(diagnostics))
        .route("/ai/provider-presets", get(list_provider_presets))
        .route("/admin/ai", get(get_lab_ai).put(save_lab_ai))
        .route(
            "/admin/ai/endpoints",
            get(list_provider_endpoints).post(create_provider_endpoint),
        )
        .route("/admin/ai/endpoints/{id}", put(update_provider_endpoint))
        .route(
            "/admin/ai/endpoints/{id}/disable",
            post(disable_provider_endpoint),
        )
        .route("/ai/turns", post(run_turn))
        .route(
            "/ai/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/ai/conversations/{id}",
            get(get_conversation).patch(update_conversation),
        )
        .route(
            "/ai/conversations/{id}/autonomy",
            get(get_autonomy).put(set_autonomy),
        )
        .route("/ai/approvals", get(list_approvals))
        .route("/ai/approvals/{id}", get(get_approval))
        .route("/ai/approvals/{id}/decision", post(decide_approval))
}

async fn get_settings(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiProviderSettingsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let settings = state
        .ai_providers
        .get(principal.user_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(settings, &metadata))
}

async fn save_settings(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SaveAiProviderSettingsInput>,
) -> Result<Json<ItemResponse<AiProviderSettingsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let settings = state
        .ai_providers
        .save(principal.user_id, payload, &audit)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(settings, &metadata))
}

async fn clear_settings(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiProviderSettingsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let audit = principal.audit_context(&metadata);
    let settings = state
        .ai_providers
        .clear_key(principal.user_id, &audit)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(settings, &metadata))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AiConnectionTestView {
    ok: bool,
    latency_ms: u128,
    capability: &'static str,
    error_code: Option<&'static str>,
}

const CONNECTION_TEST_MAX_OUTPUT_TOKENS: u32 = 256;

fn connection_test_request() -> CompletionRequest {
    let mut request = CompletionRequest::new(vec![ChatMessage::user(
        "Connection check. Reply with the single word OK.",
    )]);
    // Reasoning-capable OpenAI-compatible models may consume a small token
    // budget entirely in hidden reasoning and return no final content.
    request.max_output_tokens = Some(CONNECTION_TEST_MAX_OUTPUT_TOKENS);
    request.temperature = Some(0.0);
    request
}

async fn test_settings(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiConnectionTestView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let ResolvedAiProvider {
        provider, api_key, ..
    } = state
        .ai_providers
        .resolve(principal.user_id)
        .await
        .map_err(|error| provider_resolve_error(error, &metadata))?;
    let credentials = match api_key.as_ref() {
        Some(secret) => ProviderCredentials::bearer(secret.as_str())
            .map_err(|_| ApiError::internal().with_request_id(metadata.request_id.clone()))?,
        None => ProviderCredentials::none(),
    };
    let request = connection_test_request();
    let started = Instant::now();
    let result = provider.complete(request, credentials).await;
    let view = AiConnectionTestView {
        ok: result.is_ok(),
        latency_ms: started.elapsed().as_millis(),
        capability: "text",
        error_code: result.err().map(provider_test_error_code),
    };
    Ok(item(view, &metadata))
}

async fn diagnostics(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiProviderDiagnosticsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let diagnostics = state
        .ai_providers
        .diagnostics(principal.user_id, principal.lab_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(diagnostics, &metadata))
}

async fn list_provider_presets(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<AiProviderPresetView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let presets = state
        .ai_providers
        .list_provider_presets(principal.lab_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(collection(presets, &metadata))
}

async fn get_lab_ai(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiLabSettingsView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let settings = state
        .ai_providers
        .get_lab_settings(principal.lab_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(settings, &metadata))
}

async fn save_lab_ai(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SaveAiLabSettingsInput>,
) -> Result<Json<ItemResponse<AiLabSettingsView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let settings = state
        .ai_providers
        .save_lab_settings(
            principal.lab_id,
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(settings, &metadata))
}

async fn list_provider_endpoints(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<AiProviderEndpointView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let endpoints = state
        .ai_providers
        .list_provider_endpoints(principal.lab_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(collection(endpoints, &metadata))
}

async fn create_provider_endpoint(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SaveAiProviderEndpointInput>,
) -> Result<Json<ItemResponse<AiProviderEndpointView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let endpoint = state
        .ai_providers
        .save_provider_endpoint(
            principal.lab_id,
            None,
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(endpoint, &metadata))
}

async fn update_provider_endpoint(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(endpoint_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<SaveAiProviderEndpointInput>,
) -> Result<Json<ItemResponse<AiProviderEndpointView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let endpoint = state
        .ai_providers
        .save_provider_endpoint(
            principal.lab_id,
            Some(endpoint_id),
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(endpoint, &metadata))
}

async fn disable_provider_endpoint(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(endpoint_id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AiProviderEndpointView>>, ApiError> {
    ensure_lab_admin(&principal, &metadata)?;
    let endpoint = state
        .ai_providers
        .disable_provider_endpoint(
            principal.lab_id,
            endpoint_id,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(endpoint, &metadata))
}

fn ensure_lab_admin(principal: &AuthPrincipal, metadata: &RequestMetadata) -> Result<(), ApiError> {
    ensure_human(principal, metadata)?;
    if principal.is_lab_admin() {
        Ok(())
    } else {
        Err(ApiError::forbidden().with_request_id(metadata.request_id.clone()))
    }
}

fn provider_test_error_code(error: ProviderError) -> &'static str {
    error.code()
}

async fn run_turn(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<AssistantTurnRequest>,
) -> Result<Json<ItemResponse<AssistantTurnResponse>>, ApiError> {
    let workflow = workflow(&state, &metadata)?;
    let context =
        execution_context_with_autonomy(&state, &principal, authentication, &metadata).await?;
    let ResolvedAiProvider {
        provider,
        api_key,
        runtime,
    } = state
        .ai_providers
        .resolve(principal.user_id)
        .await
        .map_err(|error| provider_resolve_error(error, &metadata))?;
    let response = workflow
        .run_turn_with_config(
            provider,
            api_key.as_ref().map(|secret| secret.as_str()),
            &context,
            payload,
            runtime,
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(response, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationListQuery {
    project_id: Option<Uuid>,
    q: Option<String>,
    title_query: Option<String>,
    #[serde(default)]
    archive: AiConversationArchiveFilter,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationCreateHttpRequest {
    project_id: Option<Uuid>,
    title: String,
}

async fn create_conversation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<ConversationCreateHttpRequest>,
) -> Result<(StatusCode, Json<ItemResponse<AssistantConversationSummary>>), ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let conversation = workflow
        .create_conversation(
            &context,
            payload.project_id,
            payload.title,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok((StatusCode::CREATED, item(conversation, &metadata)))
}

async fn list_conversations(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ConversationListQuery>,
) -> Result<Json<CollectionResponse<AssistantConversationSummary>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let conversations = workflow
        .list_conversations(
            &context,
            query.project_id,
            query.q.or(query.title_query),
            query.archive,
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(collection(conversations, &metadata))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConversationUpdateAction {
    Rename,
    Pin,
    Unpin,
    Archive,
    Unarchive,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationUpdateHttpRequest {
    action: ConversationUpdateAction,
    title: Option<String>,
    expected_revision: i64,
}

impl ConversationUpdateHttpRequest {
    fn into_change(self) -> Result<(i64, AiConversationChange), AiWorkflowError> {
        let change = match (self.action, self.title) {
            (ConversationUpdateAction::Rename, Some(title)) => {
                AiConversationChange::Rename { title }
            }
            (ConversationUpdateAction::Rename, None) => {
                return Err(AiWorkflowError::InvalidConversationRequest);
            }
            (ConversationUpdateAction::Pin, None) => AiConversationChange::Pin,
            (ConversationUpdateAction::Unpin, None) => AiConversationChange::Unpin,
            (ConversationUpdateAction::Archive, None) => AiConversationChange::Archive,
            (ConversationUpdateAction::Unarchive, None) => AiConversationChange::Unarchive,
            (_, Some(_)) => return Err(AiWorkflowError::InvalidConversationRequest),
        };
        Ok((self.expected_revision, change))
    }
}

async fn update_conversation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ConversationUpdateHttpRequest>,
) -> Result<Json<ItemResponse<AssistantConversationSummary>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let (expected_revision, change) = payload
        .into_change()
        .map_err(|error| workflow_error(error, &metadata))?;
    let conversation = workflow
        .update_conversation(
            &context,
            id,
            expected_revision,
            change,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(conversation, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationDetailQuery {
    limit: Option<u32>,
}

async fn get_conversation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<ConversationDetailQuery>,
) -> Result<Json<ItemResponse<AssistantConversationDetail>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let conversation = workflow
        .get_conversation(&context, id, query.limit.unwrap_or(200))
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(conversation, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutonomyHttpRequest {
    mode: AiAutonomyMode,
    expected_revision: i64,
    #[serde(default)]
    current_password: Option<StepUpPassword>,
}

async fn get_autonomy(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AiAutonomyView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context =
        execution_context_with_autonomy(&state, &principal, authentication, &metadata).await?;
    let view = workflow
        .get_autonomy(&context, id)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(view, &metadata))
}

async fn set_autonomy(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<AutonomyHttpRequest>,
) -> Result<Json<ItemResponse<AiAutonomyView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let full = payload.mode == AiAutonomyMode::Full;
    let step_up_verified = if full {
        let AuthenticationMethod::Session { session_id } = authentication else {
            return Err(step_up_session_required(&metadata));
        };
        let password = payload
            .current_password
            .as_ref()
            .ok_or_else(|| step_up_required(&metadata))?;
        verify_step_up_password(&state, &principal, session_id, password, &metadata).await?;
        true
    } else {
        false
    };
    let workflow = workflow(&state, &metadata)?;
    let context =
        execution_context_with_autonomy(&state, &principal, authentication, &metadata).await?;
    let view = workflow
        .set_autonomy(
            &context,
            id,
            AiAutonomyUpdateRequest {
                mode: payload.mode,
                expected_revision: payload.expected_revision,
            },
            step_up_verified,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(view, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalListQuery {
    project_id: Option<Uuid>,
    status: Option<DraftStatus>,
}

const MAX_STEP_UP_PASSWORD_BYTES: usize = 1024;

/// One-request password material. It is intentionally not serializable or
/// cloneable; every allocation is zeroized and debug output is redacted.
struct StepUpPassword(Zeroizing<String>);

impl StepUpPassword {
    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for StepUpPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StepUpPassword([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for StepUpPassword {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() > MAX_STEP_UP_PASSWORD_BYTES {
            return Err(serde::de::Error::custom(
                "currentPassword must not exceed 1024 bytes",
            ));
        }
        Ok(Self(Zeroizing::new(value)))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftDecisionHttpRequest {
    expected_revision: u64,
    decision: ApprovalDecision,
    #[serde(default)]
    statement: Option<String>,
    #[serde(default)]
    current_password: Option<StepUpPassword>,
}

async fn list_approvals(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ApprovalListQuery>,
) -> Result<Json<CollectionResponse<WriteDraftSummary>>, ApiError> {
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let drafts = workflow
        .list_drafts(&context, query.project_id, query.status)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(collection(drafts, &metadata))
}

async fn get_approval(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<WriteDraftSummary>>, ApiError> {
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let draft = workflow
        .get_draft(&context, id)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(draft, &metadata))
}

async fn decide_approval(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<DraftDecisionHttpRequest>,
) -> Result<Json<ItemResponse<DraftDecisionResponse>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let draft = workflow
        .get_draft(&context, id)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    let DraftDecisionHttpRequest {
        expected_revision,
        decision,
        statement,
        current_password,
    } = payload;
    let requires_step_up = decision == ApprovalDecision::Approve
        && draft.requirement == ApprovalRequirement::ReinforcedConfirmation;
    let step_up_verified = if requires_step_up {
        let AuthenticationMethod::Session { session_id } = authentication else {
            return Err(step_up_session_required(&metadata));
        };
        let password = current_password
            .as_ref()
            .ok_or_else(|| step_up_required(&metadata))?;
        let attempt = state
            .ai_step_up
            .begin(principal.user_id, session_id, Instant::now())
            .map_err(|limit| step_up_limit_error(limit, &metadata))?;
        match state
            .sessions
            .verify_current_password(principal.user_id, password.expose())
            .await
        {
            Ok(()) => attempt.succeed(),
            Err(error) => {
                if is_step_up_credential_failure(&error) {
                    let failure = attempt.fail(Instant::now());
                    tracing::warn!(
                        target: "muriarc_server::security",
                        security_event = "ai_step_up_password_failed",
                        user_id = %principal.user_id,
                        session_id = %session_id,
                        request_id = %metadata.request_id,
                        failed_attempts = failure.failed_attempts,
                        rate_limited = failure.blocked_for.is_some(),
                        cooldown_seconds = failure
                            .blocked_for
                            .map(duration_seconds)
                            .unwrap_or(0),
                        "AI reinforced approval password verification failed"
                    );
                } else {
                    attempt.cancel(Instant::now());
                }
                return Err(step_up_error(error, &metadata));
            }
        }
        true
    } else {
        false
    };
    let payload = DraftDecisionRequest {
        expected_revision,
        decision,
        statement,
        step_up_verified,
    };
    let response = workflow
        .decide_draft(&context, id, payload, &principal.audit_context(&metadata))
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(response, &metadata))
}

fn step_up_required(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "step_up_required",
        "the current password is required for this reinforced approval",
    )
    .with_request_id(metadata.request_id.clone())
}

async fn verify_step_up_password(
    state: &AppState,
    principal: &AuthPrincipal,
    session_id: Uuid,
    password: &StepUpPassword,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    let attempt = state
        .ai_step_up
        .begin(principal.user_id, session_id, Instant::now())
        .map_err(|limit| step_up_limit_error(limit, metadata))?;
    match state
        .sessions
        .verify_current_password(principal.user_id, password.expose())
        .await
    {
        Ok(()) => {
            attempt.succeed();
            Ok(())
        }
        Err(error) => {
            if is_step_up_credential_failure(&error) {
                let failure = attempt.fail(Instant::now());
                tracing::warn!(
                    target: "muriarc_server::security",
                    security_event = "ai_autonomy_step_up_password_failed",
                    user_id = %principal.user_id,
                    session_id = %session_id,
                    request_id = %metadata.request_id,
                    failed_attempts = failure.failed_attempts,
                    rate_limited = failure.blocked_for.is_some(),
                    "AI autonomy password verification failed"
                );
            } else {
                attempt.cancel(Instant::now());
            }
            Err(step_up_error(error, metadata))
        }
    }
}

fn step_up_session_required(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "step_up_session_required",
        "a live browser session is required for this reinforced approval",
    )
    .with_request_id(metadata.request_id.clone())
}

fn step_up_limit_error(limit: AiStepUpLimit, metadata: &RequestMetadata) -> ApiError {
    let (code, message, retry_after) = match limit {
        AiStepUpLimit::InProgress => (
            "step_up_in_progress",
            "another password verification is already in progress for this session",
            1,
        ),
        AiStepUpLimit::Cooldown { retry_after } => (
            "step_up_rate_limited",
            "too many failed password verifications; wait before retrying",
            duration_seconds(retry_after),
        ),
    };
    ApiError::new(StatusCode::TOO_MANY_REQUESTS, code, message)
        .with_details(json!({ "retryAfterSeconds": retry_after }))
        .with_request_id(metadata.request_id.clone())
}

fn duration_seconds(duration: std::time::Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

fn is_step_up_credential_failure(error: &AuthError) -> bool {
    matches!(
        error,
        AuthError::MissingCredentials
            | AuthError::MalformedBearer
            | AuthError::InvalidToken
            | AuthError::DuplicateToken
            | AuthError::InvalidCredentials
            | AuthError::CsrfFailed
    )
}

fn step_up_error(error: AuthError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        AuthError::Unavailable | AuthError::InvalidConfiguration => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_unavailable",
            "authentication service is unavailable",
        ),
        AuthError::MissingCredentials
        | AuthError::MalformedBearer
        | AuthError::InvalidToken
        | AuthError::DuplicateToken
        | AuthError::InvalidCredentials
        | AuthError::CsrfFailed => ApiError::new(
            StatusCode::FORBIDDEN,
            "step_up_failed",
            "current password verification failed",
        ),
        other => other.into_api_error(),
    };
    error.with_request_id(metadata.request_id.clone())
}

fn workflow(state: &AppState, metadata: &RequestMetadata) -> Result<AiWorkflowService, ApiError> {
    let operations = state
        .ai_operations
        .clone()
        .ok_or_else(|| ai_disabled(metadata))?;
    let mut workflow = AiWorkflowService::new(state.store.clone(), operations);
    if let Some(files) = state.data_files.as_ref() {
        let mut data_tools =
            ServerAiDataTools::new(state.store.clone(), state.jobs.clone(), files.clone());
        if let Some(root) = state.attachment_root.as_ref() {
            data_tools = data_tools.with_attachment_root(root.as_ref());
        }
        workflow = workflow.with_data_tools(std::sync::Arc::new(data_tools));
    }
    if let Some(root) = state.attachment_root.as_ref() {
        workflow = workflow.with_source_resolver(std::sync::Arc::new(ServerAiSourceResolver::new(
            state.store.clone(),
            root.as_ref(),
        )));
    }
    Ok(workflow)
}

async fn execution_context(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
) -> Result<AiExecutionContext, ApiError> {
    let projects = store(state.store.list_projects(principal.lab_id), metadata).await?;
    let allowed_project_ids = projects
        .iter()
        .filter(|project| {
            [
                Permission::ReadAnimal,
                Permission::ReadExperiment,
                Permission::ReadMeasurement,
                Permission::ReadSample,
            ]
            .into_iter()
            .all(|permission| principal.can(permission, Some(project.id)))
        })
        .map(|project| project.id)
        .collect::<BTreeSet<_>>();
    let writable_project_ids = allowed_project_ids
        .iter()
        .copied()
        .filter(|project_id| principal.can(Permission::WriteMeasurementDraft, Some(*project_id)))
        .collect::<BTreeSet<_>>();
    let importable_project_ids = allowed_project_ids
        .iter()
        .copied()
        .filter(|project_id| principal.can(Permission::ImportData, Some(*project_id)))
        .collect::<BTreeSet<_>>();
    let exportable_project_ids = allowed_project_ids
        .iter()
        .copied()
        .filter(|project_id| principal.can(Permission::ExportData, Some(*project_id)))
        .collect::<BTreeSet<_>>();
    let lab_import = principal.can(Permission::ImportData, None);
    let lab_registry_read = principal.is_lab_operator()
        && principal.can(Permission::ReadAnimal, None)
        && principal.can(Permission::ReadExperiment, None);
    let read_activity = principal.can(Permission::ReadActivity, None)
        || allowed_project_ids
            .iter()
            .any(|project_id| principal.can(Permission::ReadActivity, Some(*project_id)));
    let read_audit = principal.can(Permission::ReadAudit, None)
        || allowed_project_ids
            .iter()
            .any(|project_id| principal.can(Permission::ReadAudit, Some(*project_id)));

    let mut effective_scopes = BTreeSet::new();
    if lab_registry_read || !allowed_project_ids.is_empty() {
        effective_scopes.insert(ToolScope::Read);
    }
    if !writable_project_ids.is_empty() {
        effective_scopes.insert(ToolScope::WriteDraft);
    }
    if lab_import || !importable_project_ids.is_empty() {
        effective_scopes.insert(ToolScope::Import);
    }
    if !exportable_project_ids.is_empty() {
        effective_scopes.insert(ToolScope::Export);
    }
    let effective_scopes = ScopeSet::new(effective_scopes);
    let access_grant = if principal.is_external_ai() {
        AccessGrant::external(
            effective_scopes,
            ScopeSet::new(principal.ai_scopes().into_iter().flatten()),
        )
    } else {
        AccessGrant::local_user(effective_scopes)
    };

    Ok(AiExecutionContext::new(
        principal.lab_id,
        principal.user_id,
        principal.display_name.clone(),
        metadata.request_id.clone(),
        allowed_project_ids,
        writable_project_ids,
        lab_registry_read,
        access_grant,
    )
    .with_data_access(importable_project_ids, exportable_project_ids, lab_import)
    .with_governance_reads(read_activity, read_audit))
}

async fn execution_context_with_autonomy(
    state: &AppState,
    principal: &AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: &RequestMetadata,
) -> Result<AiExecutionContext, ApiError> {
    let session_id = match authentication {
        AuthenticationMethod::Session { session_id } => Some(session_id),
        AuthenticationMethod::Bearer => None,
    };
    let max_mode = if principal.is_external_ai() {
        AiAutonomyMode::Ask
    } else {
        state
            .ai_providers
            .get_lab_settings(principal.lab_id)
            .await
            .map_err(|error| provider_settings_error(error, metadata))?
            .max_autonomy_mode
    };
    Ok(execution_context(state, principal, metadata)
        .await?
        .with_autonomy_context(session_id, max_mode))
}

fn ensure_human(principal: &AuthPrincipal, metadata: &RequestMetadata) -> Result<(), ApiError> {
    if principal.is_external_ai() {
        Err(ApiError::forbidden().with_request_id(metadata.request_id.clone()))
    } else {
        Ok(())
    }
}

fn ai_disabled(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "ai_runtime_not_configured",
        "the AI runtime is not configured for this deployment",
    )
    .with_request_id(metadata.request_id.clone())
}

fn provider_settings_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiProviderStoreError::InvalidSettings | AiProviderStoreError::InvalidCredential => {
            ApiError::validation(error.to_string())
        }
        AiProviderStoreError::LocalUrlForbidden | AiProviderStoreError::CloudUrlForbidden => {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "provider_exit_not_approved",
                error.to_string(),
            )
        }
        AiProviderStoreError::ProviderNotSelected => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_provider_not_selected",
            error.to_string(),
        ),
        AiProviderStoreError::MissingCredential => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_api_key_missing",
            "AI is enabled and waiting for the current user's API key",
        ),
        AiProviderStoreError::LabDisabled => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ai_lab_disabled",
            "AI is disabled by laboratory policy",
        ),
        AiProviderStoreError::Disabled => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ai_user_disabled",
            "AI is disabled in the current user's settings",
        ),
        AiProviderStoreError::NotConfigured => return ai_disabled(metadata),
        AiProviderStoreError::InvalidMasterKey
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::Storage => {
            tracing::error!(kind = ?error, "AI provider settings operation failed");
            ApiError::internal()
        }
    };
    api_error.with_request_id(metadata.request_id.clone())
}

pub(super) fn provider_resolve_error(
    error: AiProviderStoreError,
    metadata: &RequestMetadata,
) -> ApiError {
    provider_settings_error(error, metadata)
}

pub(super) fn provider_api_error(error: ProviderError, metadata: &RequestMetadata) -> ApiError {
    let code = provider_test_error_code(error.clone());
    let status = match code {
        "request_timeout" => StatusCode::GATEWAY_TIMEOUT,
        "context_exceeded" => StatusCode::UNPROCESSABLE_ENTITY,
        "api_key_rejected" => StatusCode::UNAUTHORIZED,
        "model_not_found" => StatusCode::NOT_FOUND,
        "provider_unreachable"
        | "provider_transport_error"
        | "provider_http_error"
        | "response_format_incompatible"
        | "output_budget_exhausted"
        | "provider_unavailable"
        | "response_too_large" => StatusCode::BAD_GATEWAY,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    tracing::warn!(kind = ?error, diagnostic_code = code, "AI Provider request failed");
    ApiError::new(status, code, error.to_string()).with_request_id(metadata.request_id.clone())
}

fn workflow_error(error: AiWorkflowError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiWorkflowError::Forbidden => ApiError::forbidden(),
        AiWorkflowError::Store(StoreError::NotFound { entity, id }) => {
            ApiError::not_found(format!("{entity} {id} was not found"))
        }
        AiWorkflowError::Store(StoreError::Conflict(message)) => ApiError::conflict(message),
        AiWorkflowError::Store(StoreError::Validation(message)) => ApiError::validation(message),
        AiWorkflowError::Store(
            error @ (StoreError::Database(_) | StoreError::Serialization(_)),
        ) => {
            tracing::error!(kind = ?error, "AI workflow storage operation failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "the AI workflow could not access its persisted state",
            )
        }
        AiWorkflowError::Approval(ApprovalError::RevisionConflict { .. }) => {
            ApiError::conflict("AI draft revision changed before the decision was applied")
        }
        AiWorkflowError::Approval(error) => ApiError::validation(error.to_string()),
        AiWorkflowError::Credential(error) => ApiError::validation(error.to_string()),
        AiWorkflowError::Assistant(AssistantError::Provider(error)) => {
            provider_api_error(error, metadata)
        }
        AiWorkflowError::Assistant(AssistantError::ContextWindowExceeded {
            estimated_tokens,
            max_input_tokens,
        }) => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "context_exceeded",
            "the current question and required context exceed the configured input token budget",
        )
        .with_details(json!({
            "estimatedInputTokens": estimated_tokens,
            "maxInputTokens": max_input_tokens,
            "currentQuestionTruncated": false,
        })),
        AiWorkflowError::Assistant(AssistantError::TotalTimeoutExceeded) => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "request_timeout",
            "the AI request exceeded the configured timeout",
        ),
        AiWorkflowError::Assistant(AssistantError::IterationLimitExceeded) => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "iteration_limit_exceeded",
            "the AI reached its bounded iteration limit before completing any tool result",
        ),
        AiWorkflowError::Assistant(AssistantError::ToolCallLimitExceeded) => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "tool_call_limit_exceeded",
            "the AI reached its bounded tool-call limit before completing any tool result",
        ),
        AiWorkflowError::Assistant(error) => {
            tracing::warn!(kind = ?error, "AI provider or tool execution failed");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "ai_unavailable",
                "the AI provider could not complete this request",
            )
        }
        AiWorkflowError::Config(error) => ApiError::validation(error.to_string()),
        AiWorkflowError::Source(error) => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_ai_source",
            error.to_string(),
        ),
        AiWorkflowError::DataTool(muriarc_ai::ToolExecutionError::Rejected { code }) => {
            ApiError::conflict(format!("AI data operation was rejected ({code})"))
        }
        AiWorkflowError::DataTool(muriarc_ai::ToolExecutionError::Unavailable) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ai_data_unavailable",
            "the bounded AI data operation is temporarily unavailable",
        ),
        AiWorkflowError::InvalidStoredDraft | AiWorkflowError::UnsupportedDraftOperation => {
            tracing::error!(kind = ?error, "stored AI approval data is invalid");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "stored AI approval data is invalid",
            )
        }
        AiWorkflowError::InvalidConversationRequest => ApiError::validation(error.to_string()),
        AiWorkflowError::InvalidStoredConversation => {
            tracing::error!(kind = ?error, "stored AI conversation data is invalid");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "stored AI conversation data is invalid",
            )
        }
    };
    api_error.with_request_id(metadata.request_id.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    use async_trait::async_trait;
    use axum::{
        Router,
        body::Body,
        http::{Method, Request, StatusCode, header},
        response::IntoResponse,
    };
    use chrono::{Duration, Utc};
    use http_body_util::BodyExt;
    use muriarc_ai::{
        AiSourceImportKind, DraftKind, FieldChange, ImportCommitDraftPayload,
        ImportDraftPreviewSummary, ProposalActor, ToolName, TransportFailure, WriteDraft,
    };
    use muriarc_core::{
        ActorType, AiConversation, AiConversationUpdate, AiOperationStore, Animal, Approval,
        ApprovalDecision as StoredApprovalDecision, AuditContext, AuditFilter, EntityType,
        Experiment, ExperimentTemplateVersion, FieldValueType, Job, JobKind, JobStatus, Lab,
        LabRole, MuriArcStore, Participation, Project, RecordMeta, Sex, TemplateField, ToolRun,
        ToolRunStatus, User, WriteSource,
    };
    use muriarc_data::{AnimalImportPreviewResponse, DataFiles};
    use muriarc_store_sqlite::SqliteStore;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::{
        AuthenticatedSession, DisabledAiProviderStore, ExternalTokenSummary, JobRepository,
        NewExternalToken, NewSession, SESSION_COOKIE_NAME, SessionBackend, SessionCookieConfig,
        StaticTokenAuthenticator, StoreJobRepository, ai_step_up::AiStepUpPolicy,
        ai_step_up::AiStepUpRateLimiter, application_router, token_hash,
    };

    const SESSION_TOKEN: &str = "mas_step_up_session_000000000000000000000000000000";
    const CSRF_TOKEN: &str = "mac_step_up_csrf_00000000000000000000000000000000";
    const BEARER_TOKEN: &str = "mat_step_up_bearer_000000000000000000000000000000";
    const CORRECT_PASSWORD: &str = "correct current password";

    #[test]
    fn connection_test_allows_reasoning_models_to_emit_final_content() {
        let request = connection_test_request();

        assert_eq!(
            request.max_output_tokens,
            Some(CONNECTION_TEST_MAX_OUTPUT_TOKENS)
        );
        assert_eq!(request.temperature, Some(0.0));
    }

    #[test]
    fn provider_failures_have_distinct_stable_diagnostic_codes() {
        let cases = [
            (
                ProviderError::HttpStatus {
                    status: 401,
                    request_id: None,
                },
                "api_key_rejected",
            ),
            (
                ProviderError::HttpStatus {
                    status: 404,
                    request_id: None,
                },
                "model_not_found",
            ),
            (
                ProviderError::HttpStatus {
                    status: 500,
                    request_id: None,
                },
                "provider_http_error",
            ),
            (
                ProviderError::Transport {
                    kind: TransportFailure::Connection,
                },
                "provider_unreachable",
            ),
            (
                ProviderError::Transport {
                    kind: TransportFailure::Timeout,
                },
                "request_timeout",
            ),
            (
                ProviderError::MalformedResponse,
                "response_format_incompatible",
            ),
            (
                ProviderError::OutputBudgetExhausted,
                "output_budget_exhausted",
            ),
            (
                ProviderError::RequestTooLarge { limit: 1024 },
                "context_exceeded",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(provider_test_error_code(error), expected);
        }
    }

    #[tokio::test]
    async fn zero_progress_assistant_limits_have_stable_diagnostic_codes() {
        let metadata = RequestMetadata {
            request_id: "limit-request".to_owned(),
            reason: None,
        };
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

        for (error, expected_code) in cases {
            let response =
                workflow_error(AiWorkflowError::Assistant(error), &metadata).into_response();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["error"]["code"], expected_code);
            assert_eq!(payload["error"]["request_id"], "limit-request");
        }
    }

    #[tokio::test]
    async fn corrupted_ai_state_uses_the_shared_storage_error_code() {
        let metadata = RequestMetadata {
            request_id: "storage-request".to_owned(),
            reason: None,
        };
        let response =
            workflow_error(AiWorkflowError::InvalidStoredConversation, &metadata).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["code"], "storage_error");
        assert_eq!(payload["error"]["request_id"], "storage-request");
    }

    #[tokio::test]
    async fn human_can_search_and_manage_only_owned_conversations_with_http_audit() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Conversation HTTP lab", now).unwrap();
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let user = User::new(
            lab.id,
            "conversation-http@example.test",
            "Conversation owner",
            now,
        )
        .unwrap();
        store.create_user(&user, &bootstrap).await.unwrap();
        let other = User::new(
            lab.id,
            "conversation-other@example.test",
            "Other owner",
            now,
        )
        .unwrap();
        store.create_user(&other, &bootstrap).await.unwrap();
        let project = muriarc_core::Project::new(lab.id, "Conversation project", now).unwrap();
        store.create_project(&project, &bootstrap).await.unwrap();
        let principal = AuthPrincipal::human(
            user.id,
            user.display_name.clone(),
            lab.id,
            [LabRole::LabAdmin],
        );
        let owner_audit = principal.audit_context(&RequestMetadata {
            request_id: "conversation-create".to_owned(),
            reason: Some("prepare conversation fixture".to_owned()),
        });
        let active = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            title: "Alpha longitudinal study".to_owned(),
            pinned_at: Some(now),
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let archived = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            title: "Archived study notes".to_owned(),
            pinned_at: None,
            archived_at: Some(now),
            meta: RecordMeta::new(now),
        };
        let hidden = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: other.id,
            title: "Alpha other owner".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        store
            .create_ai_conversation(&active, &owner_audit)
            .await
            .unwrap();
        store
            .create_ai_conversation(&archived, &owner_audit)
            .await
            .unwrap();
        let other_audit = AuditContext {
            actor: muriarc_core::Actor::human(other.id, other.display_name.clone()),
            source: WriteSource::Web,
            request_id: Some("other-conversation-create".to_owned()),
            reason: None,
        };
        store
            .create_ai_conversation(&hidden, &other_audit)
            .await
            .unwrap();

        let authenticator =
            StaticTokenAuthenticator::new([(BEARER_TOKEN.to_owned(), principal)]).unwrap();
        let jobs = Arc::new(StoreJobRepository::new(store.clone()));
        let state = AppState::new(store.clone(), Arc::new(authenticator), jobs)
            .with_ai(store.clone(), Arc::new(DisabledAiProviderStore));
        let app = application_router(state, None);

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/ai/conversations")
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .header("x-request-id", "conversation-create-http")
                    .header("x-audit-reason", "prepare source-bound conversation")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "project_id": project.id,
                            "title": "  Source review  "
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);
        let create = response_json(create).await;
        let created_id: Uuid = serde_json::from_value(create["data"]["id"].clone()).unwrap();
        assert_eq!(create["data"]["projectId"], project.id.to_string());
        assert_eq!(create["data"]["title"], "Source review");
        assert!(
            store
                .list_ai_conversation_messages(created_id, 20)
                .await
                .unwrap()
                .is_empty()
        );
        let create_audits = store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                entity_id: Some(created_id),
            })
            .await
            .unwrap();
        let create_audit = create_audits
            .iter()
            .find(|entry| entry.entity_type == EntityType::AiConversation)
            .unwrap();
        assert_eq!(create_audit.actor.actor_type, ActorType::Human);
        assert_eq!(create_audit.actor.user_id, Some(user.id));
        assert_eq!(create_audit.source, WriteSource::Web);
        assert_eq!(
            create_audit.request_id.as_deref(),
            Some("conversation-create-http")
        );
        assert_eq!(
            create_audit.reason.as_deref(),
            Some("prepare source-bound conversation")
        );

        let forbidden_create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/ai/conversations")
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "project_id": Uuid::new_v4(),
                            "title": "Out of scope"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden_create.status(), StatusCode::FORBIDDEN);

        let search = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/ai/conversations?project_id={}&q=ALPHA&archive=active&limit=20",
                        project.id
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(search.status(), StatusCode::OK);
        let search = response_json(search).await;
        assert_eq!(search["count"], 1);
        assert_eq!(search["data"][0]["id"], active.id.to_string());
        assert!(search["data"][0]["pinnedAt"].is_string());
        assert!(search["data"][0]["archivedAt"].is_null());

        let alias = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/ai/conversations?project_id={}&title_query=notes&archive=archived",
                        project.id
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(alias.status(), StatusCode::OK);
        assert_eq!(
            response_json(alias).await["data"][0]["id"],
            archived.id.to_string()
        );

        let invalid_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/ai/conversations?q={}", "a".repeat(257)))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_query.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/ai/conversations/{}", active.id))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .header("x-request-id", "conversation-update-http")
                    .header("x-audit-reason", "rename a study conversation")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "action": "rename",
                            "title": "  Renamed study  ",
                            "expected_revision": active.meta.revision
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::OK);
        let update = response_json(update).await;
        assert_eq!(update["data"]["title"], "Renamed study");
        assert_eq!(update["data"]["revision"], 2);
        let persisted = store.get_ai_conversation(active.id).await.unwrap();
        assert_eq!(persisted.title, "Renamed study");

        let audits = store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                entity_id: Some(active.id),
            })
            .await
            .unwrap();
        let audit = audits
            .iter()
            .find(|entry| {
                entry.entity_type == EntityType::AiConversation && entry.entity_revision == Some(2)
            })
            .unwrap();
        assert_eq!(audit.actor.actor_type, ActorType::Human);
        assert_eq!(audit.actor.user_id, Some(user.id));
        assert_eq!(audit.source, WriteSource::Web);
        assert_eq!(
            audit.request_id.as_deref(),
            Some("conversation-update-http")
        );
        assert_eq!(audit.reason.as_deref(), Some("rename a study conversation"));

        let stale = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/ai/conversations/{}", active.id))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "action": "archive",
                            "expected_revision": active.meta.revision
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);

        let forbidden = app
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/ai/conversations/{}", hidden.id))
                    .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "action": "archive",
                            "expected_revision": hidden.meta.revision
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    }

    #[derive(Clone)]
    struct TestSessions {
        principal: AuthPrincipal,
        session_id: Uuid,
        verification_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SessionBackend for TestSessions {
        async fn login(
            &self,
            _email: &str,
            _password: &str,
            _session: &NewSession,
        ) -> Result<AuthenticatedSession, AuthError> {
            Err(AuthError::Unavailable)
        }

        async fn authenticate_session(
            &self,
            session_token: &str,
        ) -> Result<AuthenticatedSession, AuthError> {
            if session_token != SESSION_TOKEN {
                return Err(AuthError::InvalidCredentials);
            }
            Ok(AuthenticatedSession {
                principal: self.principal.clone(),
                session_id: self.session_id,
                csrf_hash: token_hash(CSRF_TOKEN),
                expires_at: Utc::now() + Duration::hours(1),
            })
        }

        async fn verify_current_password(
            &self,
            user_id: Uuid,
            password: &str,
        ) -> Result<(), AuthError> {
            self.verification_calls.fetch_add(1, Ordering::SeqCst);
            if user_id == self.principal.user_id && password == CORRECT_PASSWORD {
                Ok(())
            } else {
                Err(AuthError::InvalidCredentials)
            }
        }

        async fn revoke_session(&self, _session_id: Uuid, _user_id: Uuid) -> Result<(), AuthError> {
            Err(AuthError::Unavailable)
        }

        async fn create_external_token(
            &self,
            _user_id: Uuid,
            _token: &NewExternalToken,
        ) -> Result<ExternalTokenSummary, AuthError> {
            Err(AuthError::Unavailable)
        }

        async fn list_external_tokens(
            &self,
            _user_id: Uuid,
        ) -> Result<Vec<ExternalTokenSummary>, AuthError> {
            Err(AuthError::Unavailable)
        }

        async fn revoke_external_token(
            &self,
            _user_id: Uuid,
            _token_id: Uuid,
        ) -> Result<(), AuthError> {
            Err(AuthError::Unavailable)
        }
    }

    struct ApprovalFixture {
        _temp: TempDir,
        app: Router,
        store: Arc<SqliteStore>,
        conversation_id: Uuid,
        draft_id: Uuid,
        job_id: Uuid,
        verification_calls: Arc<AtomicUsize>,
    }

    #[derive(Clone, Copy)]
    enum ApprovalFixtureScope {
        Project,
        LabWide,
    }

    impl ApprovalFixture {
        async fn new() -> Self {
            Self::with_step_up_policy(AiStepUpPolicy::default()).await
        }

        async fn lab_wide() -> Self {
            Self::with_scope(AiStepUpPolicy::default(), ApprovalFixtureScope::LabWide).await
        }

        async fn with_step_up_policy(policy: AiStepUpPolicy) -> Self {
            Self::with_scope(policy, ApprovalFixtureScope::Project).await
        }

        async fn with_scope(policy: AiStepUpPolicy, scope: ApprovalFixtureScope) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(SqliteStore::in_memory().await.unwrap());
            store.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new("Step-up lab", now).unwrap();
            store.create_lab(&lab, &bootstrap).await.unwrap();
            let user =
                User::new(lab.id, "step-up@example.test", "Step-up researcher", now).unwrap();
            store.create_user(&user, &bootstrap).await.unwrap();
            let principal = AuthPrincipal::human(
                user.id,
                user.display_name.clone(),
                lab.id,
                [LabRole::LabAdmin],
            );
            let authenticator =
                StaticTokenAuthenticator::new([(BEARER_TOKEN.to_owned(), principal.clone())])
                    .unwrap();
            let jobs = Arc::new(StoreJobRepository::new(store.clone()));
            let files = DataFiles::new(temp.path().join("data"));
            let audit = principal.audit_context(&RequestMetadata {
                request_id: "step-up-fixture".to_owned(),
                reason: Some("prepare import preview".to_owned()),
            });
            let (project_id, experiment_id) = match scope {
                ApprovalFixtureScope::Project => {
                    let project = Project::new(lab.id, "Step-up project", now).unwrap();
                    store.create_project(&project, &bootstrap).await.unwrap();
                    let mut template = ExperimentTemplateVersion::draft(
                        lab.id,
                        "step-up-measurements",
                        1,
                        "Step-up measurements",
                        now,
                    )
                    .unwrap();
                    template
                        .replace_fields(
                            vec![TemplateField {
                                key: "body_weight".to_owned(),
                                label: "Body weight".to_owned(),
                                value_type: FieldValueType::Number,
                                unit: Some("g".to_owned()),
                                required: true,
                                categories: Vec::new(),
                                minimum: Some(0.0),
                                maximum: None,
                                display_order: 0,
                                ai_writable: true,
                            }],
                            now,
                        )
                        .unwrap();
                    store
                        .create_template_version(&template, &bootstrap)
                        .await
                        .unwrap();
                    let published = store
                        .publish_template_version(
                            template.id,
                            template.meta.revision,
                            user.id,
                            now,
                            &audit,
                        )
                        .await
                        .unwrap();
                    let mut experiment =
                        Experiment::new(lab.id, project.id, "Step-up weights", now).unwrap();
                    experiment.template_version_id = Some(published.id);
                    store
                        .create_experiment(&experiment, &bootstrap)
                        .await
                        .unwrap();
                    let animal = Animal::new_mouse(lab.id, "M-STEP-UP", Sex::Female, now).unwrap();
                    store.create_animal(&animal, &bootstrap).await.unwrap();
                    let participation = Participation::enroll(experiment.id, animal.id, now);
                    store
                        .create_participation(&participation, &bootstrap)
                        .await
                        .unwrap();
                    (Some(project.id), Some(experiment.id))
                }
                ApprovalFixtureScope::LabWide => (None, None),
            };
            let mut job = Job {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id,
                created_by: user.id,
                kind: JobKind::Import,
                status: JobStatus::Parsing,
                idempotency_key: format!("step-up-import-{}", Uuid::new_v4()),
                progress_current: 0,
                progress_total: Some(3),
                result: None,
                error_report: None,
                cancellation_requested: false,
                meta: RecordMeta::new(now),
            };
            jobs.create(job.clone(), audit.clone()).await.unwrap();
            let preview = if let Some(experiment_id) = experiment_id {
                files
                    .write_upload_bytes(
                        job.id,
                        "measurements.csv",
                        b"display_id,measurement_key,value_type,value,unit,measured_at\nM-STEP-UP,body_weight,number,22.4,g,2026-07-19T08:00:00Z\n",
                    )
                    .await
                    .unwrap();
                let pending = files
                    .preview_measurement_import(&job, experiment_id, store.as_ref())
                    .await
                    .unwrap();
                AnimalImportPreviewResponse::from(&pending)
            } else {
                files
                    .write_upload_bytes(
                        job.id,
                        "animals.csv",
                        b"display_id,sex\nM-STEP-UP,female\n",
                    )
                    .await
                    .unwrap();
                let pending = files
                    .preview_animal_import(&job, store.as_ref())
                    .await
                    .unwrap();
                AnimalImportPreviewResponse::from(&pending)
            };
            assert!(preview.can_confirm);
            let import_preview =
                if let (Some(project_id), Some(experiment_id)) = (project_id, experiment_id) {
                    let issue_count = preview.issues.len();
                    let preview_row_count = preview.preview_rows.len();
                    ImportDraftPreviewSummary::from_public_preview(&json!({
                        "project_id": project_id,
                        "import_kind": preview.import_kind,
                        "experiment_id": experiment_id,
                        "file_name": preview.file_name.clone(),
                        "sheet_name": preview.sheet_name.clone(),
                        "total_rows": preview.total_rows,
                        "accepted_rows": preview.accepted_rows,
                        "issue_count": issue_count,
                        "issues_truncated": issue_count > 50,
                        "can_confirm": preview.can_confirm,
                        "preview_rows": preview.preview_rows.clone(),
                        "preview_rows_truncated": preview.accepted_rows > preview_row_count,
                        "issues": preview.issues.iter().take(50).collect::<Vec<_>>(),
                    }))
                    .unwrap()
                } else {
                    // Legacy lab-wide fixtures are rejection-only and never reach
                    // the import backend, but retain a structurally valid payload.
                    ImportDraftPreviewSummary {
                        import_kind: AiSourceImportKind::Measurement,
                        project_id: Uuid::new_v4(),
                        experiment_id: Uuid::new_v4(),
                        file_name: preview.file_name.clone(),
                        sheet_name: preview.sheet_name.clone(),
                        total_rows: preview.total_rows,
                        accepted_rows: preview.accepted_rows,
                        issue_count: 0,
                        issues_truncated: false,
                        can_confirm: true,
                        preview_rows: Vec::new(),
                        preview_rows_truncated: preview.accepted_rows > 0,
                        issues: Vec::new(),
                    }
                };
            let expected_job_revision = job.meta.revision;
            job.status = JobStatus::AwaitingConfirmation;
            job.progress_current = 2;
            job.result = Some(serde_json::to_value(&preview).unwrap());
            job.meta.touch(Utc::now());
            jobs.update(job.clone(), expected_job_revision, audit.clone())
                .await
                .unwrap();

            let conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id,
                user_id: user.id,
                title: "Import approval conversation".to_owned(),
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            store
                .create_ai_conversation(&conversation, &audit)
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
                project_id,
                vec![FieldChange {
                    path: format!("/data/imports/{}", job.id),
                    before: Some(json!({"status": "awaiting_confirmation"})),
                    after: Some(json!({"status": "completed"})),
                }],
                serde_json::to_value(ImportCommitDraftPayload {
                    operation: ImportCommitDraftPayload::OPERATION.to_owned(),
                    job_id: job.id,
                    preview_hash: preview.preview_hash.clone(),
                    expected_revision: job.meta.revision,
                    preview: import_preview,
                })
                .unwrap(),
                Utc::now(),
                now + Duration::hours(24),
            )
            .unwrap();
            let tool_run = ToolRun {
                id: tool_run_id,
                conversation_id: Some(conversation.id),
                lab_id: lab.id,
                project_id,
                user_id: user.id,
                tool_name: ToolName::ImportCommitDraft.as_str().to_owned(),
                input: json!({"job_id": job.id}),
                output: Some(json!({"draft": &draft})),
                status: ToolRunStatus::AwaitingApproval,
                source: WriteSource::Ai,
                started_at: Some(now),
                completed_at: None,
                error: None,
                meta: RecordMeta::new(now),
            };
            store.create_tool_run(&tool_run, &bootstrap).await.unwrap();
            store
                .create_approval(
                    &Approval {
                        id: draft.id(),
                        tool_run_id,
                        requested_diff: json!({"draft": &draft}),
                        decision: StoredApprovalDecision::Pending,
                        decided_by: None,
                        decided_at: None,
                        reason: None,
                        meta: RecordMeta::new(now),
                    },
                    &bootstrap,
                )
                .await
                .unwrap();

            let verification_calls = Arc::new(AtomicUsize::new(0));
            let sessions = TestSessions {
                principal,
                session_id: Uuid::new_v4(),
                verification_calls: verification_calls.clone(),
            };
            let state = AppState::new(store.clone(), Arc::new(authenticator), jobs)
                .with_sessions(
                    Arc::new(sessions),
                    SessionCookieConfig::new(false, Duration::hours(1)).unwrap(),
                )
                .with_data_storage(files, temp.path().join("attachments"))
                .with_ai(store.clone(), Arc::new(DisabledAiProviderStore))
                .with_ai_step_up_limiter(AiStepUpRateLimiter::new(policy));
            Self {
                _temp: temp,
                app: application_router(state, None),
                store,
                conversation_id: conversation.id,
                draft_id: draft.id(),
                job_id: job.id,
                verification_calls,
            }
        }

        fn session_request(&self, value: Value) -> Request<Body> {
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/ai/approvals/{}/decision", self.draft_id))
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE_NAME}={SESSION_TOKEN}"),
                )
                .header(crate::auth::CSRF_HEADER_NAME, CSRF_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .unwrap()
        }

        fn bearer_request(&self, value: Value) -> Request<Body> {
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/ai/approvals/{}/decision", self.draft_id))
                .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .unwrap()
        }
    }

    fn approval_request(password: Option<&str>) -> Value {
        let mut value = json!({
            "expectedRevision": 1,
            "decision": "approve",
            "statement": "I reviewed the complete import preview"
        });
        if let Some(password) = password {
            value["currentPassword"] = json!(password);
        }
        value
    }

    async fn response_json(response: axum::response::Response) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[tokio::test]
    async fn reinforced_approval_requires_live_session_and_server_verified_password() {
        let fixture = ApprovalFixture::new().await;

        let non_session = fixture
            .app
            .clone()
            .oneshot(fixture.bearer_request(approval_request(Some(CORRECT_PASSWORD))))
            .await
            .unwrap();
        assert_eq!(non_session.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(non_session).await["error"]["code"],
            "step_up_session_required"
        );

        let missing = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(None)))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(missing).await["error"]["code"],
            "step_up_required"
        );

        let wrong = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(Some("wrong password"))))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(wrong).await["error"]["code"],
            "step_up_failed"
        );

        let mut forged = approval_request(None);
        forged["stepUpVerified"] = json!(true);
        let forged = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(forged))
            .await
            .unwrap();
        assert_eq!(forged.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response_json(forged).await["error"]["code"], "invalid_json");
        assert_eq!(
            fixture
                .store
                .get_approval(fixture.draft_id)
                .await
                .unwrap()
                .decision,
            StoredApprovalDecision::Pending
        );

        let approved = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(Some(CORRECT_PASSWORD))))
            .await
            .unwrap();
        assert_eq!(approved.status(), StatusCode::OK);
        let body = response_json(approved).await;
        assert_eq!(body["data"]["draft"]["status"], "applied");
        assert_eq!(body["data"]["jobId"], fixture.job_id.to_string());
        assert_eq!(
            fixture.store.get_job(fixture.job_id).await.unwrap().status,
            JobStatus::Completed
        );
    }

    #[tokio::test]
    async fn lab_wide_write_draft_application_is_forbidden_but_rejection_remains_available() {
        let fixture = ApprovalFixture::lab_wide().await;

        let blocked = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(Some(CORRECT_PASSWORD))))
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(blocked).await["error"]["code"], "forbidden");
        assert_eq!(fixture.verification_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture
                .store
                .get_approval(fixture.draft_id)
                .await
                .unwrap()
                .decision,
            StoredApprovalDecision::Pending
        );
        assert_eq!(
            fixture.store.get_job(fixture.job_id).await.unwrap().status,
            JobStatus::AwaitingConfirmation
        );

        let rejected = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(json!({
                "expectedRevision": 1,
                "decision": "reject",
                "statement": "A write draft must be bound to a project"
            })))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::OK);
        assert_eq!(
            response_json(rejected).await["data"]["draft"]["status"],
            "rejected"
        );
    }

    #[tokio::test]
    async fn archived_conversation_allows_rejection_but_blocks_draft_application() {
        let fixture = ApprovalFixture::new().await;
        let conversation = fixture
            .store
            .get_ai_conversation(fixture.conversation_id)
            .await
            .unwrap();
        fixture
            .store
            .update_ai_conversation(
                &AiConversationUpdate {
                    id: conversation.id,
                    expected_revision: conversation.meta.revision,
                    change: AiConversationChange::Archive,
                    updated_at: Utc::now(),
                },
                &AuditContext {
                    actor: muriarc_core::Actor::human(conversation.user_id, "Step-up researcher"),
                    source: WriteSource::Web,
                    request_id: Some("archive-before-approval".to_owned()),
                    reason: Some("verify archived conversation write boundary".to_owned()),
                },
            )
            .await
            .unwrap();

        let blocked = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(Some(CORRECT_PASSWORD))))
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::CONFLICT);
        assert_eq!(
            fixture
                .store
                .get_approval(fixture.draft_id)
                .await
                .unwrap()
                .decision,
            StoredApprovalDecision::Pending
        );
        assert_eq!(
            fixture.store.get_job(fixture.job_id).await.unwrap().status,
            JobStatus::AwaitingConfirmation
        );

        let rejected = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(json!({
                "expectedRevision": 1,
                "decision": "reject"
            })))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::OK);
        assert_eq!(
            response_json(rejected).await["data"]["draft"]["status"],
            "rejected"
        );
    }

    #[tokio::test]
    async fn repeated_step_up_failures_are_cooled_down_without_more_argon2_work() {
        let fixture = ApprovalFixture::with_step_up_policy(AiStepUpPolicy::for_test(
            2,
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(300),
        ))
        .await;

        for password in ["wrong password one", "wrong password two"] {
            let response = fixture
                .app
                .clone()
                .oneshot(fixture.session_request(approval_request(Some(password))))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                response_json(response).await["error"]["code"],
                "step_up_failed"
            );
        }
        assert_eq!(fixture.verification_calls.load(Ordering::SeqCst), 2);

        let limited = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(approval_request(Some(CORRECT_PASSWORD))))
            .await
            .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        let limited = response_json(limited).await;
        assert_eq!(limited["error"]["code"], "step_up_rate_limited");
        assert!(
            limited["error"]["details"]["retryAfterSeconds"]
                .as_u64()
                .is_some_and(|seconds| seconds > 0)
        );
        let serialized = limited.to_string();
        assert!(!serialized.contains(CORRECT_PASSWORD));
        assert!(!serialized.contains("wrong password"));
        assert_eq!(fixture.verification_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            fixture
                .store
                .get_approval(fixture.draft_id)
                .await
                .unwrap()
                .decision,
            StoredApprovalDecision::Pending
        );
        assert_eq!(
            fixture.store.get_job(fixture.job_id).await.unwrap().status,
            JobStatus::AwaitingConfirmation
        );
    }

    #[tokio::test]
    async fn rejecting_a_reinforced_draft_does_not_require_a_password() {
        let fixture = ApprovalFixture::new().await;
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(json!({
                "expectedRevision": 1,
                "decision": "reject"
            })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await["data"]["draft"]["status"],
            "rejected"
        );
    }
}
