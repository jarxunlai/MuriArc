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
    AssistantConversationDetail, AssistantConversationStartRequest,
    AssistantConversationStartResponse, AssistantConversationSummary, AssistantError,
    AssistantTurnMedia, AssistantTurnRequest, AssistantTurnResponse, CompletionRequest,
    DraftDecisionRequest, DraftDecisionResponse, DraftStatus, ProviderCredentials, ProviderError,
    ScopeSet, ToolScope, WriteDraftSummary,
};
use muriarc_core::{
    AiAutonomyMode, AiConversationArchiveFilter, AiConversationChange, AiModelProfileBinding,
    Permission, StoreError,
};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AiLabSettingsView, AiModelDefaultsView, AiModelProfileView, AiModelValidationView,
    AiProviderDiagnosticsView, AiProviderEndpointView, AiProviderPresetView,
    AiProviderSettingsView, AiProviderStoreError, ApiError, AppState, ArchiveAiModelProfileInput,
    AuthError, AuthPrincipal, AuthenticationMethod, RequestMetadata, ResolvedAiProvider,
    SaveAiLabSettingsInput, SaveAiModelDefaultsInput, SaveAiModelProfileInput,
    SaveAiProviderEndpointInput, SaveAiProviderSettingsInput, ValidateAiModelProfileInput,
    ai_data_tools::ServerAiDataTools, ai_source_resolver::ServerAiSourceResolver,
    ai_step_up::AiStepUpLimit,
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
        .route(
            "/ai/models",
            get(list_model_profiles).post(create_model_profile),
        )
        .route("/ai/models/validate", post(validate_model_profile))
        .route(
            "/ai/models/defaults",
            get(get_model_defaults).put(save_model_defaults),
        )
        .route(
            "/ai/models/{id}",
            get(get_model_profile).put(update_model_profile),
        )
        .route(
            "/ai/models/{id}/key",
            axum::routing::delete(clear_model_profile_key),
        )
        .route("/ai/models/{id}/archive", post(archive_model_profile))
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
            get(list_conversations).post(start_conversation),
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AiModelListQuery {
    #[serde(default)]
    include_archived: bool,
}

async fn list_model_profiles(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<AiModelListQuery>,
) -> Result<Json<CollectionResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profiles = state
        .ai_providers
        .list_model_profiles(principal.user_id, query.include_archived)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(collection(profiles, &metadata))
}

async fn create_model_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SaveAiModelProfileInput>,
) -> Result<Json<ItemResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profile = state
        .ai_providers
        .create_model_profile(
            principal.user_id,
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(profile, &metadata))
}

async fn get_model_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(profile_id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profile = state
        .ai_providers
        .get_model_profile(principal.user_id, profile_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(profile, &metadata))
}

async fn update_model_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(profile_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<SaveAiModelProfileInput>,
) -> Result<Json<ItemResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profile = state
        .ai_providers
        .update_model_profile(
            principal.user_id,
            profile_id,
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(profile, &metadata))
}

async fn validate_model_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<ValidateAiModelProfileInput>,
) -> Result<Json<ItemResponse<AiModelValidationView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let validation = state
        .ai_providers
        .validate_model_profile(principal.user_id, payload)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(validation, &metadata))
}

async fn clear_model_profile_key(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(profile_id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profile = state
        .ai_providers
        .clear_model_profile_key(
            principal.user_id,
            profile_id,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(profile, &metadata))
}

async fn archive_model_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(profile_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ArchiveAiModelProfileInput>,
) -> Result<Json<ItemResponse<AiModelProfileView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let profile = state
        .ai_providers
        .archive_model_profile(
            principal.user_id,
            profile_id,
            payload.expected_revision,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(profile, &metadata))
}

async fn get_model_defaults(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<AiModelDefaultsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let defaults = state
        .ai_providers
        .get_model_defaults(principal.user_id)
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(defaults, &metadata))
}

async fn save_model_defaults(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<SaveAiModelDefaultsInput>,
) -> Result<Json<ItemResponse<AiModelDefaultsView>>, ApiError> {
    ensure_human(&principal, &metadata)?;
    let defaults = state
        .ai_providers
        .save_model_defaults(
            principal.user_id,
            payload,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| provider_settings_error(error, &metadata))?;
    Ok(item(defaults, &metadata))
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
    let request = CompletionRequest::provider_connection_check();
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
    ApiJson(payload): ApiJson<AssistantTurnHttpRequest>,
) -> Result<Json<ItemResponse<AssistantTurnResponse>>, ApiError> {
    if payload.conversation_id.is_none() {
        ensure_human(&principal, &metadata)?;
    }
    let workflow = workflow(&state, &metadata)?;
    let context =
        execution_context_with_autonomy(&state, &principal, authentication, &metadata).await?;
    let (payload, resolved) = match payload.conversation_id {
        Some(conversation_id) => {
            let payload = payload.into_request(conversation_id);
            let existing_binding = workflow
                .conversation_model_profile(&context, payload.conversation_id)
                .await
                .map_err(|error| workflow_error(error, &metadata))?;
            ensure_conversation_model_available(&state, &principal, existing_binding, &metadata)
                .await?;
            let resolved = state
                .ai_providers
                .resolve_for_profile(principal.user_id, existing_binding)
                .await
                .map_err(|error| conversation_model_resolve_error(error, &metadata))?;
            (payload, resolved)
        }
        None => {
            if !payload.source_refs.is_empty()
                || !payload.image_ids.is_empty()
                || payload.vision_model_profile_id.is_some()
            {
                return Err(ApiError::validation(
                    "conversationId is required when sending AI sources or image evidence",
                )
                .with_request_id(metadata.request_id.clone()));
            }
            let resolved = state
                .ai_providers
                .resolve(principal.user_id)
                .await
                .map_err(|error| provider_resolve_error(error, &metadata))?;
            let mut payload = payload.into_request(Uuid::new_v4());
            workflow
                .validate_turn_request_input(&context, resolved.model_profile, &payload)
                .map_err(|error| workflow_error(error, &metadata))?;
            let started = workflow
                .start_conversation(
                    &context,
                    resolved.model_profile,
                    AssistantConversationStartRequest {
                        project_id: payload.project_id,
                        title: None,
                        requested_mode: AiAutonomyMode::Ask,
                    },
                    false,
                    &principal.audit_context(&metadata),
                )
                .await
                .map_err(|error| workflow_error(error, &metadata))?;
            payload.conversation_id = started.conversation.id;
            (payload, resolved)
        }
    };
    let ResolvedAiProvider {
        provider,
        api_key,
        runtime,
        model_profile,
        supports_vision,
    } = resolved;
    workflow
        .preflight_turn_request(&context, model_profile, &payload)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    let source_bundle = workflow
        .resolve_turn_sources(&context, model_profile, &payload)
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    let source_images = source_bundle.images().to_vec();
    let images = if payload.image_ids.is_empty() {
        Vec::new()
    } else {
        super::ai_images::prepare_assistant_images(
            &state,
            &principal,
            &metadata,
            Some(payload.conversation_id),
            payload.project_id,
            &payload.image_ids,
        )
        .await?
    };
    let media = if images.is_empty() && source_images.is_empty() {
        AssistantTurnMedia::default()
    } else if supports_vision {
        AssistantTurnMedia::direct_with_sources(images, source_images)
            .map_err(|error| workflow_error(error, &metadata))?
    } else {
        let vision = resolve_turn_vision_provider(
            &state,
            &principal,
            payload.vision_model_profile_id,
            &metadata,
        )
        .await?;
        let observation = workflow
            .observe_images_with_sources(
                vision.provider,
                vision.api_key.as_ref().map(|secret| secret.as_str()),
                vision.model_profile,
                &images,
                &source_images,
                vision.runtime,
            )
            .await
            .map_err(|error| workflow_error(error, &metadata))?;
        AssistantTurnMedia::relayed_with_sources(images, source_images, observation)
            .map_err(|error| workflow_error(error, &metadata))?
    };
    let response = workflow
        .run_turn_with_resolved_sources_config(
            provider,
            api_key.as_ref().map(|secret| secret.as_str()),
            &context,
            model_profile,
            payload,
            runtime,
            media,
            source_bundle,
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok(item(response, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssistantTurnHttpRequest {
    #[serde(default, deserialize_with = "deserialize_present_conversation_id")]
    conversation_id: Option<Uuid>,
    #[serde(default)]
    project_id: Option<Uuid>,
    message: String,
    #[serde(default)]
    source_refs: Vec<Uuid>,
    #[serde(default)]
    image_ids: Vec<Uuid>,
    #[serde(default)]
    vision_model_profile_id: Option<Uuid>,
}

fn deserialize_present_conversation_id<'de, D>(deserializer: D) -> Result<Option<Uuid>, D::Error>
where
    D: Deserializer<'de>,
{
    Uuid::deserialize(deserializer).map(Some)
}

impl AssistantTurnHttpRequest {
    fn into_request(self, conversation_id: Uuid) -> AssistantTurnRequest {
        AssistantTurnRequest {
            conversation_id,
            project_id: self.project_id,
            message: self.message,
            source_refs: self.source_refs,
            image_ids: self.image_ids,
            vision_model_profile_id: self.vision_model_profile_id,
        }
    }
}

pub(super) async fn resolve_turn_vision_provider(
    state: &AppState,
    principal: &AuthPrincipal,
    explicit_profile_id: Option<Uuid>,
    metadata: &RequestMetadata,
) -> Result<ResolvedAiProvider, ApiError> {
    let profile_id = match explicit_profile_id {
        Some(profile_id) if !profile_id.is_nil() => profile_id,
        Some(_) => return Err(vision_model_unavailable(metadata)),
        None => state
            .ai_providers
            .get_model_defaults(principal.user_id)
            .await
            .map_err(|error| default_vision_model_error(error, metadata))?
            .default_vision_profile_id
            .ok_or_else(|| vision_model_selection_required(metadata))?,
    };
    let profile = state
        .ai_providers
        .get_model_profile(principal.user_id, profile_id)
        .await
        .map_err(|error| {
            if explicit_profile_id.is_some() {
                explicit_vision_model_error(error, metadata)
            } else {
                default_vision_model_error(error, metadata)
            }
        })?;
    if profile.archived_at.is_some() || !profile.supports_vision {
        return Err(if explicit_profile_id.is_some() {
            vision_model_unavailable(metadata)
        } else {
            vision_model_selection_required(metadata)
        });
    }
    let binding = AiModelProfileBinding {
        profile_id,
        profile_version: profile.current_version,
    };
    let resolved = state
        .ai_providers
        .resolve_for_profile(principal.user_id, binding)
        .await
        .map_err(|error| {
            if explicit_profile_id.is_some() {
                explicit_vision_model_error(error, metadata)
            } else {
                default_vision_model_error(error, metadata)
            }
        })?;
    if !resolved.supports_vision {
        return Err(vision_model_unavailable(metadata));
    }
    Ok(resolved)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationStartHttpRequest {
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model_profile_id: Option<Uuid>,
    requested_mode: AiAutonomyMode,
    #[serde(default)]
    current_password: Option<StepUpPassword>,
}

async fn start_conversation(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<ConversationStartHttpRequest>,
) -> Result<
    (
        StatusCode,
        Json<ItemResponse<AssistantConversationStartResponse>>,
    ),
    ApiError,
> {
    ensure_human(&principal, &metadata)?;
    let step_up_verified = if payload.requested_mode == AiAutonomyMode::Full {
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
    let model_profile =
        conversation_start_model_binding(&state, &principal, payload.model_profile_id, &metadata)
            .await?;
    let response = workflow
        .start_conversation(
            &context,
            model_profile,
            AssistantConversationStartRequest {
                project_id: payload.project_id,
                title: payload.title,
                requested_mode: payload.requested_mode,
            },
            step_up_verified,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| workflow_error(error, &metadata))?;
    Ok((StatusCode::CREATED, item(response, &metadata)))
}

async fn conversation_start_model_binding(
    state: &AppState,
    principal: &AuthPrincipal,
    explicit_profile_id: Option<Uuid>,
    metadata: &RequestMetadata,
) -> Result<AiModelProfileBinding, ApiError> {
    let profile_id = match explicit_profile_id {
        Some(profile_id) if !profile_id.is_nil() => profile_id,
        Some(_) => return Err(model_unavailable(metadata)),
        None => state
            .ai_providers
            .get_model_defaults(principal.user_id)
            .await
            .map_err(|error| default_model_error(error, metadata))?
            .default_conversation_profile_id
            .ok_or_else(|| model_selection_required(metadata))?,
    };
    let profile = state
        .ai_providers
        .get_model_profile(principal.user_id, profile_id)
        .await
        .map_err(|error| {
            if explicit_profile_id.is_some() {
                explicit_model_error(error, metadata)
            } else {
                default_model_error(error, metadata)
            }
        })?;
    if profile.archived_at.is_some() {
        return Err(if explicit_profile_id.is_some() {
            model_archived(metadata)
        } else {
            model_selection_required(metadata)
        });
    }
    let binding = AiModelProfileBinding {
        profile_id: profile.id,
        profile_version: profile.current_version,
    };
    state
        .ai_providers
        .resolve_for_profile(principal.user_id, binding)
        .await
        .map_err(|error| {
            if explicit_profile_id.is_some() {
                explicit_model_error(error, metadata)
            } else {
                default_model_error(error, metadata)
            }
        })?;
    Ok(binding)
}

pub(super) async fn ensure_conversation_model_available(
    state: &AppState,
    principal: &AuthPrincipal,
    binding: AiModelProfileBinding,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    let profiles = state
        .ai_model_profiles
        .as_ref()
        .ok_or_else(|| ai_disabled(metadata))?;
    let profile = match profiles.get_ai_model_profile(binding.profile_id).await {
        Ok(profile) => profile,
        Err(StoreError::NotFound { .. }) => return Err(model_unavailable(metadata)),
        Err(error) => {
            return Err(ApiError::from_store(error).with_request_id(metadata.request_id.clone()));
        }
    };
    if profile.lab_id != principal.lab_id
        || profile.user_id != principal.user_id
        || profile.meta.deleted_at.is_some()
    {
        return Err(model_unavailable(metadata));
    }
    if profile.archived_at.is_some() {
        return Err(model_archived(metadata));
    }
    match profiles
        .get_ai_model_profile_version(binding.profile_id, binding.profile_version)
        .await
    {
        Ok(version)
            if version.profile_id == binding.profile_id
                && version.version == binding.profile_version =>
        {
            Ok(())
        }
        Ok(_) | Err(StoreError::NotFound { .. }) => Err(model_unavailable(metadata)),
        Err(error) => Err(ApiError::from_store(error).with_request_id(metadata.request_id.clone())),
    }
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
        "the current password is required for this reinforced AI action",
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
        "a live browser session is required for this reinforced AI action",
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
    let model_profiles = state
        .ai_model_profiles
        .clone()
        .ok_or_else(|| ai_disabled(metadata))?;
    let mut workflow =
        AiWorkflowService::new(state.store.clone(), operations).with_model_profiles(model_profiles);
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
        let lab_settings = state
            .ai_providers
            .get_lab_settings(principal.lab_id)
            .await
            .map_err(|error| provider_settings_error(error, metadata))?;
        if !lab_settings.enabled {
            return Err(lab_ai_disabled(metadata));
        }
        lab_settings.max_autonomy_mode
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

fn lab_ai_disabled(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "ai_disabled",
        "AI is disabled by laboratory policy",
    )
    .with_request_id(metadata.request_id.clone())
}

fn model_selection_required(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "model_selection_required",
        "select an available conversation model before starting a conversation",
    )
    .with_request_id(metadata.request_id.clone())
}

fn model_archived(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "model_archived",
        "the conversation model has been archived and cannot accept new messages",
    )
    .with_request_id(metadata.request_id.clone())
}

fn model_unavailable(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "model_unavailable",
        "the selected conversation model is not currently available",
    )
    .with_request_id(metadata.request_id.clone())
}

pub(super) fn vision_model_selection_required(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "vision_model_selection_required",
        "select an available vision model before sending images",
    )
    .with_request_id(metadata.request_id.clone())
}

pub(super) fn vision_model_unavailable(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "vision_model_unavailable",
        "the selected vision model is not currently available",
    )
    .with_request_id(metadata.request_id.clone())
}

fn default_vision_model_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    match error {
        AiProviderStoreError::Storage
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::InvalidMasterKey => provider_settings_error(error, metadata),
        _ => vision_model_selection_required(metadata),
    }
}

fn explicit_vision_model_error(
    error: AiProviderStoreError,
    metadata: &RequestMetadata,
) -> ApiError {
    match error {
        AiProviderStoreError::Storage
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::InvalidMasterKey => provider_settings_error(error, metadata),
        _ => vision_model_unavailable(metadata),
    }
}

fn default_model_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    match error {
        AiProviderStoreError::Storage
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::InvalidMasterKey => provider_settings_error(error, metadata),
        _ => model_selection_required(metadata),
    }
}

fn explicit_model_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    match error {
        AiProviderStoreError::Storage
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::InvalidMasterKey => provider_settings_error(error, metadata),
        _ => model_unavailable(metadata),
    }
}

fn conversation_model_resolve_error(
    error: AiProviderStoreError,
    metadata: &RequestMetadata,
) -> ApiError {
    match error {
        AiProviderStoreError::Storage
        | AiProviderStoreError::Encryption
        | AiProviderStoreError::InvalidMasterKey => provider_settings_error(error, metadata),
        _ => model_unavailable(metadata),
    }
}

fn provider_settings_error(error: AiProviderStoreError, metadata: &RequestMetadata) -> ApiError {
    let api_error = match error {
        AiProviderStoreError::InvalidSettings | AiProviderStoreError::InvalidCredential => {
            ApiError::validation(error.to_string())
        }
        AiProviderStoreError::CredentialRequired => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_model_api_key_required",
            error.to_string(),
        ),
        AiProviderStoreError::ModelProfileNotFound => {
            ApiError::not_found("AI model profile was not found")
        }
        AiProviderStoreError::RevisionConflict => ApiError::conflict(error.to_string()),
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
        AiProviderStoreError::UnsupportedProtocol => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_ai_provider_protocol",
            error.to_string(),
        ),
        AiProviderStoreError::LabDisabled => return lab_ai_disabled(metadata),
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
        AiWorkflowError::LegacyConversationReadOnly => ApiError::new(
            StatusCode::CONFLICT,
            "legacy_conversation_read_only",
            error.to_string(),
        ),
        AiWorkflowError::ConversationModelProfileMismatch => ApiError::new(
            StatusCode::CONFLICT,
            "conversation_model_profile_mismatch",
            error.to_string(),
        ),
        AiWorkflowError::ConversationModelArchived => {
            ApiError::new(StatusCode::CONFLICT, "model_archived", error.to_string())
        }
        AiWorkflowError::ConversationModelUnavailable => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "model_unavailable",
            error.to_string(),
        ),
        AiWorkflowError::InvalidImageEvidence => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "image_evidence_invalid",
            error.to_string(),
        ),
        AiWorkflowError::InvalidVisionObservation => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "vision_response_invalid",
            error.to_string(),
        ),
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc::{self, Receiver},
        },
        thread,
        time::{Duration as StdDuration, Instant as StdInstant},
    };

    use super::*;

    use async_trait::async_trait;
    use axum::{
        Router,
        body::Body,
        http::{Method, Request, StatusCode, header},
        response::IntoResponse,
    };
    use base64::Engine as _;
    use chrono::{Duration, Utc};
    use http_body_util::BodyExt;
    use muriarc_ai::{
        AiSourceImportKind, AssistantRuntimeConfig, BuiltinProvider, DraftKind, FieldChange,
        ImportCommitDraftPayload, ImportDraftPreviewSummary, ProposalActor, ProviderConfig,
        ToolName, TransportFailure, WriteDraft,
    };
    use muriarc_core::{
        ActorType, AiConversation, AiConversationUpdate, AiModelProfile, AiModelProfileStore,
        AiModelProfileVersion, AiOperationStore, AiScope, Animal, Approval,
        ApprovalDecision as StoredApprovalDecision, AuditContext, AuditFilter, EntityType,
        Experiment, ExperimentTemplateVersion, FieldValueType, Job, JobKind, JobStatus, Lab,
        LabRole, MuriArcStore, Participation, Project, RecordMeta, Sex, TemplateField, ToolRun,
        ToolRunStatus, User, WriteSource,
    };
    use muriarc_data::{AnimalImportPreviewResponse, DataFiles};
    #[cfg(feature = "postgres")]
    use muriarc_store_postgres::PostgresStore;
    use muriarc_store_sqlite::SqliteStore;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;

    #[cfg(feature = "postgres")]
    use crate::{AiMasterKey, PostgresAiProviderStore};
    use crate::{
        AuthenticatedSession, DisabledAiProviderStore, ExternalTokenSummary, JobRepository,
        NewExternalToken, NewSession, SESSION_COOKIE_NAME, SessionBackend, SessionCookieConfig,
        StaticTokenAuthenticator, StoreJobRepository, UserAiProviderStore,
        ai_step_up::AiStepUpPolicy, ai_step_up::AiStepUpRateLimiter, application_router,
        token_hash,
    };

    const SESSION_TOKEN: &str = "mas_step_up_session_000000000000000000000000000000";
    const CSRF_TOKEN: &str = "mac_step_up_csrf_00000000000000000000000000000000";
    const BEARER_TOKEN: &str = "mat_step_up_bearer_000000000000000000000000000000";
    const EXTERNAL_BEARER_TOKEN: &str = "mat_step_up_external_000000000000000000000000000000";
    const CORRECT_PASSWORD: &str = "correct current password";

    fn request_complete(request: &[u8]) -> bool {
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        request.len() >= header_end + content_length
    }

    fn chat_response(id: &str, model: &str, content: &str) -> String {
        json!({
            "id": id,
            "model": model,
            "choices": [{
                "message": {"content": content, "tool_calls": []},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
        })
        .to_string()
    }

    fn portable_png() -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
            )
            .unwrap()
    }

    async fn create_test_model_profile(
        store: &SqliteStore,
        lab_id: Uuid,
        user_id: Uuid,
        name: &str,
        now: chrono::DateTime<Utc>,
        audit: &AuditContext,
    ) -> AiModelProfileBinding {
        let profile = AiModelProfile {
            id: Uuid::new_v4(),
            lab_id,
            user_id,
            name: name.to_owned(),
            current_version: 1,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let version = AiModelProfileVersion {
            profile_id: profile.id,
            version: 1,
            protocol: muriarc_core::AiProviderProtocol::OpenaiChatCompletions,
            transport: muriarc_core::AiProviderTransport::LocalHttp,
            base_url: "http://127.0.0.1:9".to_owned(),
            normalized_base_url: "http://127.0.0.1:9".to_owned(),
            model_id: "route-test-model".to_owned(),
            supports_vision: false,
            context_window_tokens: 32_768,
            max_input_tokens: 16_384,
            max_output_tokens: 2_048,
            history_token_budget: 8_192,
            history_turns: 20,
            temperature: 0.0,
            timeout_ms: 1_000,
            created_at: now,
        };
        store
            .create_ai_model_profile(&profile, &version, audit)
            .await
            .unwrap();
        AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: version.version,
        }
    }

    fn spawn_provider_sequence(
        bodies: Vec<String>,
    ) -> (String, Receiver<Vec<String>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(StdDuration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request_complete(&request) {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
            }
            sender.send(requests).unwrap();
        });
        (format!("http://{address}/v1"), receiver, handle)
    }

    fn spawn_no_call_probe() -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = calls.clone();
        let handle = thread::spawn(move || {
            let deadline = StdInstant::now() + StdDuration::from_millis(500);
            while StdInstant::now() < deadline {
                match listener.accept() {
                    Ok(_) => {
                        probe.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(StdDuration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{address}/v1"), calls, handle)
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
        let owner_model = create_test_model_profile(
            store.as_ref(),
            lab.id,
            user.id,
            "Conversation HTTP model",
            now,
            &bootstrap,
        )
        .await;
        let other_model = create_test_model_profile(
            store.as_ref(),
            lab.id,
            other.id,
            "Other owner HTTP model",
            now,
            &bootstrap,
        )
        .await;
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
            model_profile: Some(owner_model),
            legacy_read_only: false,
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
            model_profile: Some(owner_model),
            legacy_read_only: false,
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
            model_profile: Some(other_model),
            legacy_read_only: false,
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
        let providers = Arc::new(ConversationProviderStore::new(
            user.id,
            owner_model.profile_id,
            Some(owner_model.profile_id),
            AiAutonomyMode::Ask,
        ));
        let state = AppState::new(store.clone(), Arc::new(authenticator), jobs).with_ai(
            store.clone(),
            store.clone(),
            providers,
        );
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
                            "projectId": project.id,
                            "title": "  Source review  ",
                            "modelProfileId": owner_model.profile_id,
                            "requestedMode": "ask"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);
        let create = response_json(create).await;
        let created_id: Uuid =
            serde_json::from_value(create["data"]["conversation"]["id"].clone()).unwrap();
        assert_eq!(
            create["data"]["conversation"]["projectId"],
            project.id.to_string()
        );
        assert_eq!(create["data"]["conversation"]["title"], "Source review");
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
                            "projectId": Uuid::new_v4(),
                            "title": "Out of scope",
                            "modelProfileId": owner_model.profile_id,
                            "requestedMode": "ask"
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

    struct ConversationProviderStore {
        user_id: Uuid,
        profile_id: Uuid,
        default_profile: Option<Uuid>,
        base_url: String,
        supports_vision: bool,
        vision_profile_id: Option<Uuid>,
        vision_base_url: Option<String>,
        max_mode: AiAutonomyMode,
        lab_enabled: AtomicBool,
        archived: AtomicBool,
        resolve_calls: AtomicUsize,
        default_resolve_failure: AtomicUsize,
    }

    impl ConversationProviderStore {
        fn new(
            user_id: Uuid,
            profile_id: Uuid,
            default_profile: Option<Uuid>,
            max_mode: AiAutonomyMode,
        ) -> Self {
            Self {
                user_id,
                profile_id,
                default_profile,
                base_url: "http://127.0.0.1:9".to_owned(),
                supports_vision: false,
                vision_profile_id: None,
                vision_base_url: None,
                max_mode,
                lab_enabled: AtomicBool::new(true),
                archived: AtomicBool::new(false),
                resolve_calls: AtomicUsize::new(0),
                default_resolve_failure: AtomicUsize::new(0),
            }
        }

        fn with_vision_test_runtime(
            mut self,
            base_url: String,
            supports_vision: bool,
            vision_profile: Option<(Uuid, String)>,
        ) -> Self {
            self.base_url = base_url;
            self.supports_vision = supports_vision;
            if let Some((profile_id, base_url)) = vision_profile {
                self.vision_profile_id = Some(profile_id);
                self.vision_base_url = Some(base_url);
            }
            self
        }

        fn profile_for(&self, profile_id: Uuid) -> Option<AiModelProfileView> {
            let now = Utc::now();
            let (name, base_url, model_id, supports_vision, archived_at) =
                if profile_id == self.profile_id {
                    (
                        "Conversation model",
                        self.base_url.as_str(),
                        "conversation-test-model",
                        self.supports_vision,
                        self.archived.load(Ordering::SeqCst).then_some(now),
                    )
                } else if Some(profile_id) == self.vision_profile_id {
                    (
                        "Vision model",
                        self.vision_base_url.as_deref()?,
                        "vision-test-model",
                        true,
                        None,
                    )
                } else {
                    return None;
                };
            Some(AiModelProfileView {
                id: profile_id,
                name: name.to_owned(),
                current_version: 1,
                protocol: muriarc_core::AiProviderProtocol::OpenaiChatCompletions,
                transport: muriarc_core::AiProviderTransport::LocalHttp,
                base_url: base_url.to_owned(),
                model_id: model_id.to_owned(),
                supports_vision,
                context_window_tokens: 32_768,
                max_input_tokens: 16_384,
                max_output_tokens: 2_048,
                history_token_budget: 8_192,
                history_turns: 20,
                temperature: 0.0,
                timeout_ms: 2_000,
                has_key: false,
                archived_at,
                is_default_conversation: self.default_profile == Some(profile_id),
                is_default_vision: self.vision_profile_id == Some(profile_id),
                revision: 1,
                created_at: now,
                updated_at: now,
            })
        }

        fn profile(&self) -> AiModelProfileView {
            self.profile_for(self.profile_id)
                .expect("conversation test profile must exist")
        }

        fn fail_default_resolution_with(&self, error: AiProviderStoreError) {
            let code = match error {
                AiProviderStoreError::LabDisabled => 1,
                AiProviderStoreError::NotConfigured => 2,
                other => panic!("unsupported default resolution test error: {other:?}"),
            };
            self.default_resolve_failure.store(code, Ordering::SeqCst);
        }

        fn disable_lab(&self) {
            self.lab_enabled.store(false, Ordering::SeqCst);
        }

        fn resolved(
            &self,
            user_id: Uuid,
            binding: AiModelProfileBinding,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            if user_id != self.user_id || binding.profile_version != 1 {
                return Err(AiProviderStoreError::ModelProfileNotFound);
            }
            let profile = self
                .profile_for(binding.profile_id)
                .ok_or(AiProviderStoreError::ModelProfileNotFound)?;
            if profile.archived_at.is_some() {
                return Err(AiProviderStoreError::ProviderNotSelected);
            }
            let provider = BuiltinProvider::from_config(ProviderConfig::local_http(
                if binding.profile_id == self.profile_id {
                    "conversation-test-provider"
                } else {
                    "vision-test-provider"
                },
                profile.model_id.clone(),
                profile.base_url.clone(),
            ))
            .unwrap();
            Ok(ResolvedAiProvider {
                provider,
                api_key: None,
                runtime: AssistantRuntimeConfig {
                    context_window_tokens: 32_768,
                    max_input_tokens: 16_384,
                    max_output_tokens: 2_048,
                    history_token_budget: 8_192,
                    history_turns: 20,
                    temperature: 0.0,
                    timeout_ms: 2_000,
                },
                model_profile: binding,
                supports_vision: profile.supports_vision,
            })
        }
    }

    #[async_trait]
    impl UserAiProviderStore for ConversationProviderStore {
        async fn get(
            &self,
            _user_id: Uuid,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn save(
            &self,
            _user_id: Uuid,
            _input: SaveAiProviderSettingsInput,
            _audit: &AuditContext,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn clear_key(
            &self,
            _user_id: Uuid,
            _audit: &AuditContext,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn resolve(&self, user_id: Uuid) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            match self.default_resolve_failure.load(Ordering::SeqCst) {
                1 => return Err(AiProviderStoreError::LabDisabled),
                2 => return Err(AiProviderStoreError::NotConfigured),
                _ => {}
            }
            let profile_id = self
                .default_profile
                .ok_or(AiProviderStoreError::ProviderNotSelected)?;
            self.resolved(
                user_id,
                AiModelProfileBinding {
                    profile_id,
                    profile_version: 1,
                },
            )
        }

        async fn resolve_for_profile(
            &self,
            user_id: Uuid,
            binding: AiModelProfileBinding,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            self.resolved(user_id, binding)
        }

        async fn resolve_vision(
            &self,
            user_id: Uuid,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            let profile_id = self
                .vision_profile_id
                .ok_or(AiProviderStoreError::ProviderNotSelected)?;
            self.resolved(
                user_id,
                AiModelProfileBinding {
                    profile_id,
                    profile_version: 1,
                },
            )
        }

        async fn diagnostics(
            &self,
            _user_id: Uuid,
            _lab_id: Uuid,
        ) -> Result<AiProviderDiagnosticsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn get_lab_settings(
            &self,
            _lab_id: Uuid,
        ) -> Result<AiLabSettingsView, AiProviderStoreError> {
            Ok(AiLabSettingsView {
                enabled: self.lab_enabled.load(Ordering::SeqCst),
                custom_url_approval_required: true,
                configured_user_count: 1,
                enabled_user_count: 1,
                vision_user_count: i64::from(
                    self.supports_vision || self.vision_profile_id.is_some(),
                ),
                revision: 1,
                max_autonomy_mode: self.max_mode,
            })
        }

        async fn save_lab_settings(
            &self,
            _lab_id: Uuid,
            _input: SaveAiLabSettingsInput,
            _audit: &AuditContext,
        ) -> Result<AiLabSettingsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn list_provider_presets(
            &self,
            _lab_id: Uuid,
        ) -> Result<Vec<AiProviderPresetView>, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn list_provider_endpoints(
            &self,
            _lab_id: Uuid,
        ) -> Result<Vec<AiProviderEndpointView>, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn save_provider_endpoint(
            &self,
            _lab_id: Uuid,
            _endpoint_id: Option<Uuid>,
            _input: SaveAiProviderEndpointInput,
            _audit: &AuditContext,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn disable_provider_endpoint(
            &self,
            _lab_id: Uuid,
            _endpoint_id: Uuid,
            _audit: &AuditContext,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn list_model_profiles(
            &self,
            user_id: Uuid,
            include_archived: bool,
        ) -> Result<Vec<AiModelProfileView>, AiProviderStoreError> {
            if user_id != self.user_id {
                return Ok(Vec::new());
            }
            Ok([
                Some(self.profile()),
                self.vision_profile_id.and_then(|id| self.profile_for(id)),
            ]
            .into_iter()
            .flatten()
            .filter(|profile| include_archived || profile.archived_at.is_none())
            .collect())
        }

        async fn get_model_profile(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            if user_id != self.user_id {
                return Err(AiProviderStoreError::ModelProfileNotFound);
            }
            self.profile_for(profile_id)
                .ok_or(AiProviderStoreError::ModelProfileNotFound)
        }

        async fn create_model_profile(
            &self,
            _user_id: Uuid,
            _input: SaveAiModelProfileInput,
            _audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn update_model_profile(
            &self,
            _user_id: Uuid,
            _profile_id: Uuid,
            _input: SaveAiModelProfileInput,
            _audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn validate_model_profile(
            &self,
            _user_id: Uuid,
            _input: ValidateAiModelProfileInput,
        ) -> Result<AiModelValidationView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn clear_model_profile_key(
            &self,
            _user_id: Uuid,
            _profile_id: Uuid,
            _audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn archive_model_profile(
            &self,
            _user_id: Uuid,
            _profile_id: Uuid,
            _revision: i64,
            _audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }

        async fn get_model_defaults(
            &self,
            user_id: Uuid,
        ) -> Result<AiModelDefaultsView, AiProviderStoreError> {
            if user_id != self.user_id {
                return Err(AiProviderStoreError::Storage);
            }
            Ok(AiModelDefaultsView {
                default_conversation_profile_id: self.default_profile,
                default_vision_profile_id: self.vision_profile_id,
                revision: 1,
            })
        }

        async fn save_model_defaults(
            &self,
            _user_id: Uuid,
            _input: SaveAiModelDefaultsInput,
            _audit: &AuditContext,
        ) -> Result<AiModelDefaultsView, AiProviderStoreError> {
            Err(AiProviderStoreError::NotConfigured)
        }
    }

    struct ConversationFixture {
        _temp: TempDir,
        app: Router,
        state: AppState,
        principal: AuthPrincipal,
        store: Arc<SqliteStore>,
        providers: Arc<ConversationProviderStore>,
        profile_id: Uuid,
        vision_profile_id: Option<Uuid>,
        audit: AuditContext,
        verification_calls: Arc<AtomicUsize>,
    }

    struct ConversationProviderRuntime {
        base_url: String,
        supports_vision: bool,
        vision_profile: Option<(Uuid, String)>,
    }

    impl ConversationFixture {
        async fn new(default_selected: bool, max_mode: AiAutonomyMode) -> Self {
            Self::with_step_up_policy(default_selected, max_mode, AiStepUpPolicy::default()).await
        }

        async fn with_vision_runtime(base_url: String, conversation_supports_vision: bool) -> Self {
            let vision_profile =
                (!conversation_supports_vision).then(|| (Uuid::new_v4(), base_url.clone()));
            Self::configured(
                true,
                AiAutonomyMode::Full,
                AiStepUpPolicy::default(),
                Some(ConversationProviderRuntime {
                    base_url,
                    supports_vision: conversation_supports_vision,
                    vision_profile,
                }),
            )
            .await
        }

        async fn with_missing_vision_runtime(base_url: String) -> Self {
            Self::configured(
                true,
                AiAutonomyMode::Full,
                AiStepUpPolicy::default(),
                Some(ConversationProviderRuntime {
                    base_url,
                    supports_vision: false,
                    vision_profile: None,
                }),
            )
            .await
        }

        async fn with_step_up_policy(
            default_selected: bool,
            max_mode: AiAutonomyMode,
            policy: AiStepUpPolicy,
        ) -> Self {
            Self::configured(default_selected, max_mode, policy, None).await
        }

        async fn configured(
            default_selected: bool,
            max_mode: AiAutonomyMode,
            policy: AiStepUpPolicy,
            provider_runtime: Option<ConversationProviderRuntime>,
        ) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let attachment_root = temp.path().join("attachments");
            let store = Arc::new(SqliteStore::in_memory().await.unwrap());
            store.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new("Conversation start route lab", now).unwrap();
            store.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                "conversation-start@example.test",
                "Conversation route user",
                now,
            )
            .unwrap();
            store.create_user(&user, &bootstrap).await.unwrap();
            let principal = AuthPrincipal::human(
                user.id,
                user.display_name.clone(),
                lab.id,
                [LabRole::LabAdmin],
            );
            let audit = principal.audit_context(&RequestMetadata {
                request_id: "conversation-start-fixture".to_owned(),
                reason: Some("prepare conversation model".to_owned()),
            });
            let profile_id = Uuid::new_v4();
            let ConversationProviderRuntime {
                base_url,
                supports_vision,
                vision_profile,
            } = provider_runtime.unwrap_or_else(|| ConversationProviderRuntime {
                base_url: "http://127.0.0.1:9".to_owned(),
                supports_vision: false,
                vision_profile: None,
            });
            let profile = AiModelProfile {
                id: profile_id,
                lab_id: lab.id,
                user_id: user.id,
                name: "Conversation model".to_owned(),
                current_version: 1,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            let version = AiModelProfileVersion {
                profile_id,
                version: 1,
                protocol: muriarc_core::AiProviderProtocol::OpenaiChatCompletions,
                transport: muriarc_core::AiProviderTransport::LocalHttp,
                base_url: base_url.clone(),
                normalized_base_url: base_url.clone(),
                model_id: "conversation-test-model".to_owned(),
                supports_vision,
                context_window_tokens: 32_768,
                max_input_tokens: 16_384,
                max_output_tokens: 2_048,
                history_token_budget: 8_192,
                history_turns: 20,
                temperature: 0.0,
                timeout_ms: 1_000,
                created_at: now,
            };
            store
                .create_ai_model_profile(&profile, &version, &audit)
                .await
                .unwrap();
            if let Some((vision_profile_id, vision_base_url)) = vision_profile.as_ref() {
                let vision_profile_record = AiModelProfile {
                    id: *vision_profile_id,
                    lab_id: lab.id,
                    user_id: user.id,
                    name: "Vision model".to_owned(),
                    current_version: 1,
                    archived_at: None,
                    meta: RecordMeta::new(now),
                };
                let vision_version = AiModelProfileVersion {
                    profile_id: *vision_profile_id,
                    version: 1,
                    protocol: muriarc_core::AiProviderProtocol::OpenaiChatCompletions,
                    transport: muriarc_core::AiProviderTransport::LocalHttp,
                    base_url: vision_base_url.clone(),
                    normalized_base_url: vision_base_url.clone(),
                    model_id: "vision-test-model".to_owned(),
                    supports_vision: true,
                    context_window_tokens: 32_768,
                    max_input_tokens: 16_384,
                    max_output_tokens: 2_048,
                    history_token_budget: 8_192,
                    history_turns: 20,
                    temperature: 0.0,
                    timeout_ms: 2_000,
                    created_at: now,
                };
                store
                    .create_ai_model_profile(&vision_profile_record, &vision_version, &audit)
                    .await
                    .unwrap();
            }

            let providers = Arc::new(
                ConversationProviderStore::new(
                    user.id,
                    profile_id,
                    default_selected.then_some(profile_id),
                    max_mode,
                )
                .with_vision_test_runtime(
                    base_url,
                    supports_vision,
                    vision_profile.clone(),
                ),
            );
            let verification_calls = Arc::new(AtomicUsize::new(0));
            let sessions = TestSessions {
                principal: principal.clone(),
                session_id: Uuid::new_v4(),
                verification_calls: verification_calls.clone(),
            };
            let fixture_principal = principal.clone();
            let external = principal.clone().with_ai_scopes([AiScope::Read]);
            let authenticator = StaticTokenAuthenticator::new([
                (BEARER_TOKEN.to_owned(), principal),
                (EXTERNAL_BEARER_TOKEN.to_owned(), external),
            ])
            .unwrap();
            let jobs = Arc::new(StoreJobRepository::new(store.clone()));
            let state = AppState::new(store.clone(), Arc::new(authenticator), jobs)
                .with_sessions(
                    Arc::new(sessions),
                    SessionCookieConfig::new(false, Duration::hours(1)).unwrap(),
                )
                .with_ai(store.clone(), store.clone(), providers.clone())
                .with_data_storage(DataFiles::new(temp.path().join("data")), attachment_root)
                .with_ai_step_up_limiter(AiStepUpRateLimiter::new(policy));
            Self {
                _temp: temp,
                app: application_router(state.clone(), None),
                state,
                principal: fixture_principal,
                store,
                providers,
                profile_id,
                vision_profile_id: vision_profile.map(|(profile_id, _)| profile_id),
                audit,
                verification_calls,
            }
        }

        fn session_request(
            &self,
            method: Method,
            uri: &str,
            value: Option<Value>,
        ) -> Request<Body> {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE_NAME}={SESSION_TOKEN}"),
                )
                .header(crate::auth::CSRF_HEADER_NAME, CSRF_TOKEN);
            let body = match value {
                Some(value) => {
                    request = request.header(header::CONTENT_TYPE, "application/json");
                    Body::from(value.to_string())
                }
                None => Body::empty(),
            };
            request.body(body).unwrap()
        }

        fn bearer_request(&self, value: Value) -> Request<Body> {
            self.bearer_request_with_token(BEARER_TOKEN, value)
        }

        fn bearer_request_with_token(&self, token: &str, value: Value) -> Request<Body> {
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/ai/conversations")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .unwrap()
        }

        async fn archive_model(&self) {
            let mut profile = self
                .store
                .get_ai_model_profile(self.profile_id)
                .await
                .unwrap();
            let expected_revision = profile.meta.revision;
            let now = Utc::now();
            profile.archived_at = Some(now);
            profile.meta.touch(now);
            self.store
                .archive_ai_model_profile(&profile, expected_revision, &self.audit)
                .await
                .unwrap();
            self.providers.archived.store(true, Ordering::SeqCst);
        }
    }

    fn conversation_start_request(
        mode: AiAutonomyMode,
        profile_id: Option<Uuid>,
        password: Option<&str>,
    ) -> Value {
        let mut value = json!({
            "requestedMode": match mode {
                AiAutonomyMode::Ask => "ask",
                AiAutonomyMode::Auto => "auto",
                AiAutonomyMode::Full => "full",
            }
        });
        if let Some(profile_id) = profile_id {
            value["modelProfileId"] = json!(profile_id);
        }
        if let Some(password) = password {
            value["currentPassword"] = json!(password);
        }
        value
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

            let model_profile = create_test_model_profile(
                store.as_ref(),
                lab.id,
                user.id,
                "Approval fixture model",
                now,
                &audit,
            )
            .await;
            let conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id,
                user_id: user.id,
                title: "Import approval conversation".to_owned(),
                model_profile: Some(model_profile),
                legacy_read_only: false,
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
                .with_ai(
                    store.clone(),
                    store.clone(),
                    Arc::new(DisabledAiProviderStore),
                )
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
    async fn stale_defaults_fail_closed_and_secret_storage_errors_stay_generic() {
        let metadata = RequestMetadata {
            request_id: "phase-five-default-safety".to_owned(),
            reason: None,
        };

        for error in [
            AiProviderStoreError::ModelProfileNotFound,
            AiProviderStoreError::ProviderNotSelected,
        ] {
            let error = default_model_error(error, &metadata);
            assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let body = response_json(error.into_response()).await;
            assert_eq!(body["error"]["code"], "model_selection_required");
            assert_eq!(body["error"]["request_id"], metadata.request_id);
        }
        for error in [
            AiProviderStoreError::ModelProfileNotFound,
            AiProviderStoreError::ProviderNotSelected,
        ] {
            let error = default_vision_model_error(error, &metadata);
            assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let body = response_json(error.into_response()).await;
            assert_eq!(body["error"]["code"], "vision_model_selection_required");
            assert_eq!(body["error"]["request_id"], metadata.request_id);
        }

        for error in [
            AiProviderStoreError::InvalidMasterKey,
            AiProviderStoreError::Encryption,
            AiProviderStoreError::Storage,
        ] {
            let error = provider_settings_error(error, &metadata);
            assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = response_json(error.into_response()).await;
            assert_eq!(body["error"]["code"], "internal_error");
            assert_eq!(
                body["error"]["message"],
                "an internal server error occurred"
            );
            assert_eq!(body["error"]["request_id"], metadata.request_id);
        }
    }

    async fn upload_test_image(fixture: &ConversationFixture) -> (Uuid, Uuid) {
        let conversation_id = start_test_conversation(fixture).await;
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/api/v1/ai/images/upload?file_name=evidence.png&media_type=image%2Fpng&conversation_id={conversation_id}"
                    ))
                    .header(
                        header::COOKIE,
                        format!("{SESSION_COOKIE_NAME}={SESSION_TOKEN}"),
                    )
                    .header(crate::auth::CSRF_HEADER_NAME, CSRF_TOKEN)
                    .header(header::CONTENT_TYPE, "image/png")
                    .body(Body::from(portable_png()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let image_id = response_json(response).await["data"]["image"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        (conversation_id, image_id)
    }

    async fn start_test_conversation(fixture: &ConversationFixture) -> Uuid {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(json!({
                    "title": "Source image routing",
                    "modelProfileId": fixture.profile_id,
                    "requestedMode": "ask",
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await["data"]["conversation"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn upload_test_image_source(
        fixture: &ConversationFixture,
        conversation_id: Uuid,
    ) -> Uuid {
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/api/v1/ai/sources/upload?file_name=source.png&media_type=image%2Fpng&conversation_id={conversation_id}"
                    ))
                    .header(
                        header::COOKIE,
                        format!("{SESSION_COOKIE_NAME}={SESSION_TOKEN}"),
                    )
                    .header(crate::auth::CSRF_HEADER_NAME, CSRF_TOKEN)
                    .header(header::CONTENT_TYPE, "image/png")
                    .body(Body::from(portable_png()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await["data"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn run_test_image_turn(
        fixture: &ConversationFixture,
        conversation_id: Uuid,
        image_id: Uuid,
    ) -> axum::response::Response {
        fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": conversation_id,
                    "message": "Describe only the visible evidence",
                    "imageIds": [image_id],
                })),
            ))
            .await
            .unwrap()
    }

    async fn run_test_source_image_turn(
        fixture: &ConversationFixture,
        conversation_id: Uuid,
        source_id: Uuid,
    ) -> axum::response::Response {
        fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": conversation_id,
                    "message": "Describe only the selected source image",
                    "sourceRefs": [source_id],
                })),
            ))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn missing_default_vision_model_fails_before_any_provider_resolution_or_call() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        let metadata = RequestMetadata {
            request_id: "missing-default-vision".to_owned(),
            reason: None,
        };
        let error =
            match resolve_turn_vision_provider(&fixture.state, &fixture.principal, None, &metadata)
                .await
            {
                Ok(_) => panic!("a missing default vision model must fail closed"),
                Err(error) => error,
            };
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(error.into_response()).await["error"]["code"],
            "vision_model_selection_required"
        );
        assert_eq!(
            fixture.providers.resolve_calls.load(Ordering::SeqCst),
            0,
            "selection must fail before a Provider can be resolved or called"
        );
    }

    #[tokio::test]
    async fn direct_vision_http_chain_uploads_sanitizes_calls_once_and_traces_exact_profile() {
        let (base_url, captured, handle) = spawn_provider_sequence(vec![chat_response(
            "direct-final",
            "conversation-test-model",
            "Direct grounded answer",
        )]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        let (conversation_id, image_id) = upload_test_image(&fixture).await;
        let response = run_test_image_turn(&fixture, conversation_id, image_id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["content"], "Direct grounded answer");
        assert_eq!(
            response["data"]["trace"]["modelCalls"][0]["purpose"],
            "vision_and_final"
        );
        assert_eq!(
            response["data"]["trace"]["modelCalls"][0]["modelProfileId"],
            fixture.profile_id.to_string()
        );
        assert_eq!(response["data"]["trace"]["usage"]["provider_calls"], 1);
        assert_eq!(
            response["data"]["trace"]["imageEvidence"][0]["imageId"],
            image_id.to_string()
        );

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn direct_vision_rejects_an_unused_relay_profile_before_any_provider_call() {
        let (base_url, calls, handle) = spawn_no_call_probe();
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        let (conversation_id, image_id) = upload_test_image(&fixture).await;
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": conversation_id,
                    "message": "Do not silently ignore the relay selection",
                    "imageIds": [image_id],
                    "visionModelProfileId": Uuid::new_v4(),
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "image_evidence_invalid"
        );
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn relayed_vision_http_chain_records_two_stages_and_json_evidence_envelope() {
        let observation = json!({
            "observations": [{"imageIndex": 1, "description": "one visible panel"}]
        })
        .to_string();
        let (base_url, captured, handle) = spawn_provider_sequence(vec![
            chat_response("vision-observation", "vision-test-model", &observation),
            chat_response(
                "relay-final",
                "conversation-test-model",
                "Relayed grounded answer",
            ),
        ]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, false).await;
        let (conversation_id, image_id) = upload_test_image(&fixture).await;
        let response = run_test_image_turn(&fixture, conversation_id, image_id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["content"], "Relayed grounded answer");
        assert_eq!(
            response["data"]["trace"]["modelCalls"][0]["purpose"],
            "vision_observation"
        );
        assert_eq!(
            response["data"]["trace"]["modelCalls"][0]["modelProfileId"],
            fixture.vision_profile_id.unwrap().to_string()
        );
        assert_eq!(
            response["data"]["trace"]["modelCalls"][1]["purpose"],
            "final_answer"
        );
        assert_eq!(
            response["data"]["trace"]["modelCalls"][1]["modelProfileId"],
            fixture.profile_id.to_string()
        );
        assert_eq!(response["data"]["trace"]["usage"]["provider_calls"], 2);

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("data:image/png;base64,"));
        assert!(!requests[1].contains("data:image/png;base64,"));
        assert!(requests[1].contains("MURIARC_VISION_EVIDENCE_V1="));
        assert!(!requests[1].contains("<vision_observation>"));
    }

    #[tokio::test]
    async fn missing_default_vision_http_chain_returns_selection_error_with_zero_network_calls() {
        let (base_url, calls, handle) = spawn_no_call_probe();
        let fixture = ConversationFixture::with_missing_vision_runtime(base_url).await;
        let (conversation_id, image_id) = upload_test_image(&fixture).await;
        let response = run_test_image_turn(&fixture, conversation_id, image_id).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "vision_model_selection_required"
        );
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn source_image_routes_directly_when_the_bound_conversation_model_supports_vision() {
        let (base_url, captured, handle) = spawn_provider_sequence(vec![chat_response(
            "source-direct-final",
            "conversation-test-model",
            "Direct source-grounded answer",
        )]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        let conversation_id = start_test_conversation(&fixture).await;
        let source_id = upload_test_image_source(&fixture, conversation_id).await;
        let response = run_test_source_image_turn(&fixture, conversation_id, source_id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["content"], "Direct source-grounded answer");
        assert_eq!(response["data"]["trace"]["usage"]["provider_calls"], 1);
        assert!(
            response["data"]["trace"]
                .as_object()
                .unwrap()
                .get("imageEvidence")
                .is_none()
        );

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn source_image_uses_the_selected_vision_relay_for_a_text_only_conversation_model() {
        let observation = json!({
            "observations": [{"imageIndex": 1, "description": "one source image"}]
        })
        .to_string();
        let (base_url, captured, handle) = spawn_provider_sequence(vec![
            chat_response("source-observation", "vision-test-model", &observation),
            chat_response(
                "source-relay-final",
                "conversation-test-model",
                "Relayed source-grounded answer",
            ),
        ]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, false).await;
        let conversation_id = start_test_conversation(&fixture).await;
        let source_id = upload_test_image_source(&fixture, conversation_id).await;
        let response = run_test_source_image_turn(&fixture, conversation_id, source_id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(
            response["data"]["content"],
            "Relayed source-grounded answer"
        );
        assert_eq!(response["data"]["trace"]["usage"]["provider_calls"], 2);
        assert!(
            response["data"]["trace"]
                .as_object()
                .unwrap()
                .get("imageEvidence")
                .is_none()
        );

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("data:image/png;base64,"));
        assert!(!requests[1].contains("data:image/png;base64,"));
        assert!(requests[1].contains("MURIARC_VISION_EVIDENCE_V1="));
    }

    #[tokio::test]
    async fn source_image_without_a_default_vision_model_fails_before_any_provider_call() {
        let (base_url, calls, handle) = spawn_no_call_probe();
        let fixture = ConversationFixture::with_missing_vision_runtime(base_url).await;
        let conversation_id = start_test_conversation(&fixture).await;
        let source_id = upload_test_image_source(&fixture, conversation_id).await;
        let response = run_test_source_image_turn(&fixture, conversation_id, source_id).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "vision_model_selection_required"
        );
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn legacy_text_turn_starts_an_audited_ask_conversation_with_the_exact_default_binding() {
        let (base_url, captured, handle) = spawn_provider_sequence(vec![chat_response(
            "legacy-text-final",
            "conversation-test-model",
            "Legacy text answer",
        )]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        let now = Utc::now();
        let project = Project::new(fixture.principal.lab_id, "Legacy turn project", now).unwrap();
        fixture
            .store
            .create_project(&project, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();

        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "projectId": project.id,
                    "message": "Use the backwards-compatible text path"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["content"], "Legacy text answer");
        let conversation_id = response["data"]["conversationId"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let conversation = fixture
            .store
            .get_ai_conversation(conversation_id)
            .await
            .unwrap();
        assert_eq!(conversation.project_id, Some(project.id));
        assert_eq!(
            conversation.model_profile,
            Some(AiModelProfileBinding {
                profile_id: fixture.profile_id,
                profile_version: 1,
            })
        );
        let autonomy = fixture
            .store
            .get_ai_autonomy_grant(conversation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(autonomy.mode, AiAutonomyMode::Ask);
        assert_eq!(autonomy.project_id, Some(project.id));
        assert_eq!(autonomy.user_id, fixture.principal.user_id);
        let audit = fixture
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: fixture.principal.lab_id,
                project_id: Some(project.id),
                entity_id: Some(conversation_id),
            })
            .await
            .unwrap();
        assert!(audit.iter().any(|entry| {
            entry.action == muriarc_core::AuditAction::Create
                && entry.actor.actor_type == ActorType::Human
                && entry.actor.user_id == Some(fixture.principal.user_id)
                && entry.source == WriteSource::Web
        }));
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 1);

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn legacy_text_turn_persists_ask_conversation_before_provider_failure() {
        let (base_url, captured, handle) =
            spawn_provider_sequence(vec!["not valid provider json".to_owned()]);
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({"message": "Persist before calling the Provider"})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let requests = captured.recv_timeout(StdDuration::from_secs(2)).unwrap();
        handle.join().unwrap();
        assert_eq!(requests.len(), 1);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = response_json(listed).await;
        assert_eq!(listed["count"], 1);
        let conversation_id = listed["data"][0]["id"].as_str().unwrap().parse().unwrap();
        let conversation = fixture
            .store
            .get_ai_conversation(conversation_id)
            .await
            .unwrap();
        assert_eq!(
            conversation.model_profile,
            Some(AiModelProfileBinding {
                profile_id: fixture.profile_id,
                profile_version: 1,
            })
        );
        assert_eq!(
            fixture
                .store
                .get_ai_autonomy_grant(conversation_id)
                .await
                .unwrap()
                .unwrap()
                .mode,
            AiAutonomyMode::Ask
        );
    }

    #[tokio::test]
    async fn legacy_text_turn_rejects_external_bearer_before_resolution_or_write() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/ai/turns")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {EXTERNAL_BEARER_TOKEN}"),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"message": "External AI cannot create a conversation"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(response).await["error"]["code"], "forbidden");
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(response_json(listed).await["count"], 0);
    }

    #[tokio::test]
    async fn explicit_null_conversation_id_remains_invalid_json() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": null,
                    "message": "Null is not an omitted conversation ID"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "invalid_json"
        );
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(response_json(listed).await["count"], 0);
    }

    #[tokio::test]
    async fn invalid_legacy_messages_are_validated_after_resolution_but_before_any_write_or_call() {
        let (base_url, calls, handle) = spawn_no_call_probe();
        let fixture = ConversationFixture::with_vision_runtime(base_url, true).await;
        for message in [
            " \n\t ".to_owned(),
            "x".repeat(muriarc_ai::AssistantLimits::default().max_user_message_bytes + 1),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(fixture.session_request(
                    Method::POST,
                    "/api/v1/ai/turns",
                    Some(json!({"message": message})),
                ))
                .await
                .unwrap();
            assert!(!response.status().is_success());
        }
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 2);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(response_json(listed).await["count"], 0);
        let audits = fixture
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: fixture.principal.lab_id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap();
        assert!(audits.iter().all(|entry| {
            !matches!(
                entry.entity_type,
                EntityType::AiConversation | EntityType::AiAutonomyGrant
            )
        }));
    }

    #[tokio::test]
    async fn explicit_unknown_conversation_is_never_replaced_by_a_default_conversation() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        let unknown_id = Uuid::new_v4();
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": unknown_id,
                    "message": "Do not create a replacement conversation"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response_json(response).await["error"]["code"], "not_found");
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        assert_eq!(response_json(listed).await["count"], 0);
    }

    #[tokio::test]
    async fn legacy_turn_preserves_disabled_and_unconfigured_provider_errors() {
        for (error, expected_code) in [
            (AiProviderStoreError::LabDisabled, "ai_disabled"),
            (
                AiProviderStoreError::NotConfigured,
                "ai_runtime_not_configured",
            ),
        ] {
            let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
            fixture.providers.fail_default_resolution_with(error);
            let response = fixture
                .app
                .clone()
                .oneshot(fixture.session_request(
                    Method::POST,
                    "/api/v1/ai/turns",
                    Some(json!({"message": "Keep the legacy provider error semantics"})),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                response_json(response).await["error"]["code"],
                expected_code
            );

            let listed = fixture
                .app
                .clone()
                .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
                .await
                .unwrap();
            assert_eq!(response_json(listed).await["count"], 0);
        }
    }

    #[tokio::test]
    async fn lab_disable_blocks_legacy_turn_before_default_provider_resolution() {
        let fixture = ConversationFixture::new(false, AiAutonomyMode::Full).await;
        fixture.providers.disable_lab();

        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({"message": "The provider must not be resolved"})),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "ai_disabled"
        );
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn legacy_turn_rejects_bound_evidence_and_unknown_fields_before_resolution() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        for payload in [
            json!({
                "message": "A source needs an explicit conversation",
                "sourceRefs": [Uuid::new_v4()]
            }),
            json!({
                "message": "An image needs an explicit conversation",
                "imageIds": [Uuid::new_v4()]
            }),
            json!({
                "message": "A vision route needs an explicit conversation",
                "visionModelProfileId": Uuid::new_v4()
            }),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(fixture.session_request(Method::POST, "/api/v1/ai/turns", Some(payload)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(
                response_json(response).await["error"]["code"],
                "validation_error"
            );
        }
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "message": "Unknown fields remain rejected",
                    "modelProfileId": fixture.profile_id
                })),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "invalid_json"
        );
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let listed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(response_json(listed).await["count"], 0);
    }

    #[tokio::test]
    async fn full_conversation_start_requires_live_session_and_verified_password_before_model_resolution()
     {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;

        let bearer = fixture
            .app
            .clone()
            .oneshot(fixture.bearer_request(conversation_start_request(
                AiAutonomyMode::Full,
                None,
                Some(CORRECT_PASSWORD),
            )))
            .await
            .unwrap();
        assert_eq!(bearer.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(bearer).await["error"]["code"],
            "step_up_session_required"
        );

        let external = fixture
            .app
            .clone()
            .oneshot(fixture.bearer_request_with_token(
                EXTERNAL_BEARER_TOKEN,
                conversation_start_request(AiAutonomyMode::Full, None, Some(CORRECT_PASSWORD)),
            ))
            .await
            .unwrap();
        assert_eq!(external.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(external).await["error"]["code"], "forbidden");

        let missing = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(AiAutonomyMode::Full, None, None)),
            ))
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
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(
                    AiAutonomyMode::Full,
                    None,
                    Some("wrong password"),
                )),
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(wrong).await["error"]["code"],
            "step_up_failed"
        );
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let created = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(
                    AiAutonomyMode::Full,
                    None,
                    Some(CORRECT_PASSWORD),
                )),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        assert_eq!(created["data"]["autonomy"]["mode"], "full");
        assert_eq!(created["data"]["autonomy"]["effectiveMode"], "full");
        assert_eq!(
            created["data"]["conversation"]["modelProfileId"],
            fixture.profile_id.to_string()
        );
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.verification_calls.load(Ordering::SeqCst), 2);
        assert!(!created.to_string().contains(CORRECT_PASSWORD));
        assert!(!created.to_string().contains("wrong password"));
    }

    #[tokio::test]
    async fn rate_limited_full_start_creates_nothing_and_never_resolves_a_provider() {
        let fixture = ConversationFixture::with_step_up_policy(
            true,
            AiAutonomyMode::Full,
            AiStepUpPolicy::for_test(
                1,
                std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(300),
            ),
        )
        .await;
        let failed = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(
                    AiAutonomyMode::Full,
                    None,
                    Some("wrong password"),
                )),
            ))
            .await
            .unwrap();
        assert_eq!(failed.status(), StatusCode::FORBIDDEN);

        let limited = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(
                    AiAutonomyMode::Full,
                    None,
                    Some(CORRECT_PASSWORD),
                )),
            ))
            .await
            .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response_json(limited).await["error"]["code"],
            "step_up_rate_limited"
        );
        assert_eq!(fixture.verification_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let conversations = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(Method::GET, "/api/v1/ai/conversations", None))
            .await
            .unwrap();
        assert_eq!(conversations.status(), StatusCode::OK);
        assert_eq!(response_json(conversations).await["count"], 0);
    }

    #[tokio::test]
    async fn explicit_model_start_preserves_requested_full_while_admin_ceiling_is_auto() {
        let fixture = ConversationFixture::new(false, AiAutonomyMode::Auto).await;
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(
                    AiAutonomyMode::Full,
                    Some(fixture.profile_id),
                    Some(CORRECT_PASSWORD),
                )),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let response = response_json(response).await;
        assert_eq!(response["data"]["autonomy"]["mode"], "full");
        assert_eq!(response["data"]["autonomy"]["effectiveMode"], "auto");
        assert_eq!(response["data"]["autonomy"]["maxMode"], "auto");
        assert_eq!(
            response["data"]["conversation"]["modelProfileId"],
            fixture.profile_id.to_string()
        );
        assert_eq!(response["data"]["conversation"]["modelProfileVersion"], 1);
    }

    #[tokio::test]
    async fn ask_and_auto_starts_need_no_password_but_missing_default_requires_selection() {
        let selected = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        for mode in [AiAutonomyMode::Ask, AiAutonomyMode::Auto] {
            let response = selected
                .app
                .clone()
                .oneshot(selected.session_request(
                    Method::POST,
                    "/api/v1/ai/conversations",
                    Some(conversation_start_request(mode, None, None)),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let response = response_json(response).await;
            assert_eq!(
                response["data"]["autonomy"]["mode"],
                match mode {
                    AiAutonomyMode::Ask => "ask",
                    AiAutonomyMode::Auto => "auto",
                    AiAutonomyMode::Full => unreachable!(),
                }
            );
        }
        assert_eq!(selected.verification_calls.load(Ordering::SeqCst), 0);

        let missing = ConversationFixture::new(false, AiAutonomyMode::Full).await;
        let response = missing
            .app
            .clone()
            .oneshot(missing.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(AiAutonomyMode::Ask, None, None)),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "model_selection_required"
        );
        assert_eq!(missing.providers.resolve_calls.load(Ordering::SeqCst), 0);

        let mut unknown = conversation_start_request(AiAutonomyMode::Ask, None, None);
        unknown["stepUpVerified"] = json!(true);
        let response = missing
            .app
            .clone()
            .oneshot(missing.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(unknown),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "invalid_json"
        );
    }

    #[tokio::test]
    async fn archived_conversation_remains_readable_but_rejects_provider_turns() {
        let fixture = ConversationFixture::new(true, AiAutonomyMode::Full).await;
        let created = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(AiAutonomyMode::Ask, None, None)),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        let conversation_id = created["data"]["conversation"]["id"].as_str().unwrap();
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 1);
        fixture.archive_model().await;

        let history = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::GET,
                &format!("/api/v1/ai/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(history.status(), StatusCode::OK);
        let history = response_json(history).await;
        assert_eq!(history["data"]["conversation"]["readOnly"], true);
        assert_eq!(
            history["data"]["conversation"]["readOnlyReason"],
            "model_archived"
        );

        let replacement = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/conversations",
                Some(conversation_start_request(AiAutonomyMode::Ask, None, None)),
            ))
            .await
            .unwrap();
        assert_eq!(replacement.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(replacement).await["error"]["code"],
            "model_selection_required"
        );
        assert_eq!(
            fixture.providers.resolve_calls.load(Ordering::SeqCst),
            1,
            "an archived default must not be resolved or replaced by a list fallback"
        );

        let turn = fixture
            .app
            .clone()
            .oneshot(fixture.session_request(
                Method::POST,
                "/api/v1/ai/turns",
                Some(json!({
                    "conversationId": conversation_id,
                    "message": "This must not reach the Provider"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(turn.status(), StatusCode::CONFLICT);
        assert_eq!(response_json(turn).await["error"]["code"], "model_archived");
        assert_eq!(fixture.providers.resolve_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn model_management_routes_use_authenticated_user_scoped_postgres_store() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
        );
        let postgres = PostgresStore::connect(&database_url).await.unwrap();
        postgres.migrate().await.unwrap();
        let sqlite = Arc::new(SqliteStore::in_memory().await.unwrap());
        sqlite.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new(format!("AI model routes lab {}", Uuid::new_v4()), now).unwrap();
        let user = User::new(
            lab.id,
            format!("ai-model-routes-{}@example.test", Uuid::new_v4()),
            "AI model routes owner",
            now,
        )
        .unwrap();
        postgres.create_lab(&lab, &bootstrap).await.unwrap();
        postgres.create_user(&user, &bootstrap).await.unwrap();
        sqlite.create_lab(&lab, &bootstrap).await.unwrap();
        sqlite.create_user(&user, &bootstrap).await.unwrap();

        let principal = AuthPrincipal::human(
            user.id,
            user.display_name.clone(),
            lab.id,
            [LabRole::AnimalManager],
        );
        let authenticator =
            StaticTokenAuthenticator::new([(BEARER_TOKEN.to_owned(), principal)]).unwrap();
        let jobs = Arc::new(StoreJobRepository::new(sqlite.clone()));
        let master_key =
            AiMasterKey::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", 1).unwrap();
        let providers = Arc::new(PostgresAiProviderStore::new(postgres, master_key));
        let state = AppState::new(sqlite.clone(), Arc::new(authenticator), jobs).with_ai(
            sqlite.clone(),
            sqlite,
            providers,
        );
        let app = application_router(state, None);

        let create = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/ai/models")
            .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "name": "Route model",
                    "protocol": "openai_chat_completions",
                    "transport": "open_ai_compatible",
                    "baseUrl": "https://api.deepseek.com",
                    "modelId": "route-model/自由",
                    "supportsVision": true,
                    "contextWindowTokens": 131072,
                    "maxInputTokens": 65536,
                    "maxOutputTokens": 4096,
                    "historyTokenBudget": 32768,
                    "historyTurns": 20,
                    "temperature": 0,
                    "timeoutMs": 120000,
                    "apiKey": "route-test-key"
                })
                .to_string(),
            ))
            .unwrap();
        let created = app.clone().oneshot(create).await.unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created = response_json(created).await;
        assert_eq!(created["data"]["currentVersion"], 1);
        assert_eq!(created["data"]["transport"], "open_ai_compatible");
        assert_eq!(created["data"]["modelId"], "route-model/自由");
        assert_eq!(created["data"]["hasKey"], true);
        let profile_id = created["data"]["id"].as_str().unwrap();
        let rotate_key = Request::builder()
            .method(Method::PUT)
            .uri(format!("/api/v1/ai/models/{profile_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "name": "Route model",
                    "protocol": "openai_chat_completions",
                    "transport": "open_ai_compatible",
                    "baseUrl": "https://api.deepseek.com",
                    "modelId": "route-model/自由",
                    "supportsVision": true,
                    "contextWindowTokens": 131072,
                    "maxInputTokens": 65536,
                    "maxOutputTokens": 4096,
                    "historyTokenBudget": 32768,
                    "historyTurns": 20,
                    "temperature": 0,
                    "timeoutMs": 120000,
                    "apiKey": "route-test-key-rotated",
                    "expectedRevision": 1
                })
                .to_string(),
            ))
            .unwrap();
        let key_rotated = app.clone().oneshot(rotate_key).await.unwrap();
        assert_eq!(key_rotated.status(), StatusCode::OK);
        let key_rotated = response_json(key_rotated).await;
        assert_eq!(key_rotated["data"]["currentVersion"], 1);
        assert_eq!(key_rotated["data"]["revision"], 1);
        assert!(!key_rotated.to_string().contains("route-test-key-rotated"));

        let list = Request::builder()
            .method(Method::GET)
            .uri("/api/v1/ai/models")
            .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let listed = app.clone().oneshot(list).await.unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = response_json(listed).await;
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["data"][0]["name"], "Route model");
        assert!(!listed.to_string().contains("route-test-key"));

        let validate_without_key = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/ai/models/validate")
            .header(header::AUTHORIZATION, format!("Bearer {BEARER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "protocol": "openai_chat_completions",
                    "transport": "open_ai_compatible",
                    "baseUrl": "https://api.deepseek.com",
                    "modelId": "unsaved-cloud-model",
                    "supportsVision": false,
                    "contextWindowTokens": 131072,
                    "maxInputTokens": 65536,
                    "maxOutputTokens": 4096,
                    "historyTokenBudget": 32768,
                    "historyTurns": 20,
                    "temperature": 0,
                    "timeoutMs": 120000
                })
                .to_string(),
            ))
            .unwrap();
        let missing_key = app.oneshot(validate_without_key).await.unwrap();
        assert_eq!(missing_key.status(), StatusCode::OK);
        let missing_key = response_json(missing_key).await;
        assert_eq!(missing_key["data"]["ok"], false);
        assert_eq!(missing_key["data"]["errorCode"], "missing_credential");
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
