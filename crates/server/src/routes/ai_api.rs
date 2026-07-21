use std::{collections::BTreeSet, fmt, time::Instant};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post, put},
};
use muriarc_ai::{
    AccessGrant, AiExecutionContext, AiProvider, AiWorkflowError, AiWorkflowService,
    ApprovalDecision, ApprovalError, ApprovalRequirement, AssistantConversationDetail,
    AssistantConversationSummary, AssistantTurnRequest, AssistantTurnResponse, ChatMessage,
    CompletionRequest, DraftDecisionRequest, DraftDecisionResponse, DraftStatus,
    ProviderCredentials, ProviderError, ScopeSet, ToolScope, WriteDraftSummary,
};
use muriarc_core::{Permission, StoreError};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AiLabSettingsView, AiProviderDiagnosticsView, AiProviderEndpointView, AiProviderSettingsView,
    AiProviderStoreError, ApiError, AppState, AuthError, AuthPrincipal, AuthenticationMethod,
    RequestMetadata, ResolvedAiProvider, SaveAiLabSettingsInput, SaveAiProviderEndpointInput,
    SaveAiProviderSettingsInput, ai_data_tools::ServerAiDataTools, ai_step_up::AiStepUpLimit,
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
        .route("/ai/conversations", get(list_conversations))
        .route("/ai/conversations/{id}", get(get_conversation))
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

async fn test_settings(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiConnectionTestView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let ResolvedAiProvider { provider, api_key } = state
        .ai_providers
        .resolve(principal.user_id)
        .await
        .map_err(|error| provider_resolve_error(error, &metadata))?;
    let credentials = match api_key.as_ref() {
        Some(secret) => ProviderCredentials::bearer(secret.as_str())
            .map_err(|_| ApiError::internal().with_request_id(metadata.request_id.clone()))?,
        None => ProviderCredentials::none(),
    };
    let mut request = CompletionRequest::new(vec![ChatMessage::user(
        "Connection check. Reply with the single word OK.",
    )]);
    request.max_output_tokens = Some(8);
    request.temperature = Some(0.0);
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
    match error {
        ProviderError::InvalidConfig(_) | ProviderError::InvalidRequest(_) => "invalid_provider",
        ProviderError::RequestTooLarge { .. } => "request_too_large",
        ProviderError::ResponseTooLarge { .. } => "response_too_large",
        ProviderError::Transport { .. } => "transport_failed",
        ProviderError::HttpStatus {
            status: 401 | 403, ..
        } => "credential_rejected",
        ProviderError::HttpStatus { .. } => "provider_http_error",
        ProviderError::MalformedResponse | ProviderError::EmptyResponse => "invalid_response",
        ProviderError::MockExhausted | ProviderError::MockUnavailable => "provider_unavailable",
    }
}

async fn run_turn(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<AssistantTurnRequest>,
) -> Result<Json<ItemResponse<AssistantTurnResponse>>, ApiError> {
    let workflow = workflow(&state, &metadata)?;
    let context = execution_context(&state, &principal, &metadata).await?;
    let ResolvedAiProvider { provider, api_key } = state
        .ai_providers
        .resolve(principal.user_id)
        .await
        .map_err(|error| provider_resolve_error(error, &metadata))?;
    let response = workflow
        .run_turn(
            provider,
            api_key.as_ref().map(|secret| secret.as_str()),
            &context,
            payload,
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(response, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationListQuery {
    project_id: Option<Uuid>,
    limit: Option<u32>,
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
        .list_conversations(&context, query.project_id, query.limit.unwrap_or(50))
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(collection(conversations, &metadata))
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
        workflow = workflow.with_data_tools(std::sync::Arc::new(ServerAiDataTools::new(
            state.store.clone(),
            state.jobs.clone(),
            files.clone(),
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
    .with_data_access(importable_project_ids, exportable_project_ids, lab_import))
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
        "ai_disabled",
        "the shared AI service is disabled by the server administrator",
    )
    .with_request_id(metadata.request_id.clone())
}

fn provider_settings_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiProviderStoreError::InvalidSettings
        | AiProviderStoreError::InvalidCredential
        | AiProviderStoreError::LocalUrlForbidden
        | AiProviderStoreError::CloudUrlForbidden
        | AiProviderStoreError::MissingCredential => ApiError::validation(error.to_string()),
        AiProviderStoreError::Disabled | AiProviderStoreError::NotConfigured => {
            return ai_disabled(metadata);
        }
        AiProviderStoreError::InvalidMasterKey
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::Storage => {
            tracing::error!(kind = ?error, "AI provider settings operation failed");
            ApiError::internal()
        }
    };
    api_error.with_request_id(metadata.request_id.clone())
}

fn provider_resolve_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiProviderStoreError::Disabled
        | AiProviderStoreError::MissingCredential
        | AiProviderStoreError::NotConfigured => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ai_not_configured",
            "AI is not enabled and fully configured for this user",
        ),
        AiProviderStoreError::InvalidSettings
        | AiProviderStoreError::InvalidCredential
        | AiProviderStoreError::LocalUrlForbidden
        | AiProviderStoreError::CloudUrlForbidden
        | AiProviderStoreError::InvalidMasterKey
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::Storage => {
            tracing::error!(kind = ?error, "AI provider resolution failed");
            ApiError::internal()
        }
    };
    api_error.with_request_id(metadata.request_id.clone())
}

fn workflow_error(error: AiWorkflowError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiWorkflowError::Forbidden => ApiError::forbidden(),
        AiWorkflowError::Store(error) => return store_error(error, metadata),
        AiWorkflowError::Approval(ApprovalError::RevisionConflict { .. }) => {
            ApiError::conflict("AI draft revision changed before the decision was applied")
        }
        AiWorkflowError::Approval(error) => ApiError::validation(error.to_string()),
        AiWorkflowError::Credential(error) => ApiError::validation(error.to_string()),
        AiWorkflowError::Assistant(error) => {
            tracing::warn!(kind = ?error, "AI provider or tool execution failed");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "ai_unavailable",
                "the AI provider could not complete this request",
            )
        }
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
            ApiError::internal()
        }
        AiWorkflowError::InvalidConversationRequest => ApiError::validation(error.to_string()),
        AiWorkflowError::InvalidStoredConversation => {
            tracing::error!(kind = ?error, "stored AI conversation data is invalid");
            ApiError::internal()
        }
    };
    api_error.with_request_id(metadata.request_id.clone())
}

fn store_error(error: StoreError, metadata: &RequestMetadata) -> ApiError {
    ApiError::from_store(error).with_request_id(metadata.request_id.clone())
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
    };
    use chrono::{Duration, Utc};
    use http_body_util::BodyExt;
    use muriarc_ai::{
        DraftKind, FieldChange, ImportCommitDraftPayload, ProposalActor, ToolName, WriteDraft,
    };
    use muriarc_core::{
        AiOperationStore, Approval, ApprovalDecision as StoredApprovalDecision, AuditContext, Job,
        JobKind, JobStatus, Lab, LabRole, MuriArcStore, RecordMeta, ToolRun, ToolRunStatus, User,
        WriteSource,
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
        draft_id: Uuid,
        job_id: Uuid,
        verification_calls: Arc<AtomicUsize>,
    }

    impl ApprovalFixture {
        async fn new() -> Self {
            Self::with_step_up_policy(AiStepUpPolicy::default()).await
        }

        async fn with_step_up_policy(policy: AiStepUpPolicy) -> Self {
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
            let mut job = Job {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id: None,
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
            files
                .write_upload_bytes(job.id, "animals.csv", b"display_id,sex\nM-STEP-UP,female\n")
                .await
                .unwrap();
            let pending = files
                .preview_animal_import(&job, store.as_ref())
                .await
                .unwrap();
            let expected_job_revision = job.meta.revision;
            job.status = JobStatus::AwaitingConfirmation;
            job.progress_current = 2;
            job.result =
                Some(serde_json::to_value(AnimalImportPreviewResponse::from(&pending)).unwrap());
            job.meta.touch(Utc::now());
            jobs.update(job.clone(), expected_job_revision, audit.clone())
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
                    path: format!("/data/imports/{}", job.id),
                    before: Some(json!({"status": "awaiting_confirmation"})),
                    after: Some(json!({"status": "completed"})),
                }],
                serde_json::to_value(ImportCommitDraftPayload {
                    operation: ImportCommitDraftPayload::OPERATION.to_owned(),
                    job_id: job.id,
                    preview_hash: pending.preview_hash,
                    expected_revision: job.meta.revision,
                })
                .unwrap(),
                Utc::now(),
                now + Duration::hours(24),
            )
            .unwrap();
            let tool_run = ToolRun {
                id: tool_run_id,
                conversation_id: None,
                lab_id: lab.id,
                project_id: None,
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
