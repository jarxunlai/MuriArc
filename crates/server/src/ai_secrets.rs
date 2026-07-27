use std::{fmt, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use muriarc_ai::{
    AiProvider, CompletionRequest, ProviderConfig, ProviderCredentials, ProviderError,
    TransportFailure,
};
use muriarc_ai::{AssistantRuntimeConfig, BuiltinProvider, ProviderKind};
use muriarc_core::{
    AiAutonomyMode, AiModelProfileBinding, AiProviderProtocol, AiProviderTransport, AuditContext,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(feature = "postgres")]
const SERVER_PROVIDER_ID: &str = "server-user-provider";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderSettingsView {
    pub enabled: bool,
    pub provider_kind: ProviderKind,
    pub provider_preset_id: String,
    pub model: String,
    pub base_url: String,
    pub has_key: bool,
    pub supports_vision: bool,
    pub vision_model: Option<String>,
    pub context_window_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub history_token_budget: u32,
    pub history_turns: u32,
    pub temperature: f32,
    pub timeout_ms: u64,
    pub revision: i64,
}

impl AiProviderSettingsView {
    #[cfg(feature = "postgres")]
    fn unconfigured() -> Self {
        let runtime = AssistantRuntimeConfig::default();
        Self {
            enabled: true,
            provider_kind: ProviderKind::OpenAiCompatible,
            provider_preset_id: "deepseek".to_owned(),
            model: "deepseek-chat".to_owned(),
            base_url: "https://api.deepseek.com".to_owned(),
            has_key: false,
            supports_vision: false,
            vision_model: None,
            context_window_tokens: runtime.context_window_tokens,
            max_input_tokens: runtime.max_input_tokens,
            max_output_tokens: runtime.max_output_tokens,
            history_token_budget: runtime.history_token_budget,
            history_turns: runtime.history_turns,
            temperature: runtime.temperature,
            timeout_ms: runtime.timeout_ms,
            revision: 0,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiProviderSettingsInput {
    pub enabled: bool,
    pub provider_kind: ProviderKind,
    #[serde(default = "default_provider_preset_id")]
    pub provider_preset_id: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default = "default_context_window_tokens")]
    pub context_window_tokens: u32,
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u32,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_history_token_budget")]
    pub history_token_budget: u32,
    #[serde(default = "default_history_turns")]
    pub history_turns: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_provider_preset_id() -> String {
    "deepseek".to_owned()
}
const fn default_context_window_tokens() -> u32 {
    131_072
}
const fn default_max_input_tokens() -> u32 {
    65_536
}
const fn default_max_output_tokens() -> u32 {
    4_096
}
const fn default_history_token_budget() -> u32 {
    32_768
}
const fn default_history_turns() -> u32 {
    20
}
const fn default_temperature() -> f32 {
    0.0
}
const fn default_timeout_ms() -> u64 {
    120_000
}

impl SaveAiProviderSettingsInput {
    fn runtime(&self) -> Result<AssistantRuntimeConfig, AiProviderStoreError> {
        AssistantRuntimeConfig {
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            history_token_budget: self.history_token_budget,
            history_turns: self.history_turns,
            temperature: self.temperature,
            timeout_ms: self.timeout_ms,
        }
        .validate()
        .map_err(|_| AiProviderStoreError::InvalidSettings)
    }
}

impl fmt::Debug for SaveAiProviderSettingsInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SaveAiProviderSettingsInput")
            .field("enabled", &self.enabled)
            .field("provider_kind", &self.provider_kind)
            .field("provider_preset_id", &self.provider_preset_id)
            .field("model", &"[REDACTED]")
            .field("base_url", &"[REDACTED]")
            .field("supports_vision", &self.supports_vision)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("history_token_budget", &self.history_token_budget)
            .field("history_turns", &self.history_turns)
            .field("temperature", &self.temperature)
            .field("timeout_ms", &self.timeout_ms)
            .field(
                "vision_model",
                &self.vision_model.as_ref().map(|_| "[REDACTED]"),
            )
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

pub struct SensitiveSecret(Zeroizing<String>);

impl SensitiveSecret {
    #[cfg(feature = "postgres")]
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SensitiveSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveSecret([REDACTED])")
    }
}

pub struct ResolvedAiProvider {
    pub provider: BuiltinProvider,
    pub api_key: Option<SensitiveSecret>,
    pub runtime: AssistantRuntimeConfig,
    pub model_profile: AiModelProfileBinding,
    /// Capability of this exact immutable profile version, not the current
    /// mutable profile projection.
    pub supports_vision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderDiagnosticsView {
    pub runtime_configured: bool,
    pub lab_enabled: bool,
    pub user_enabled: bool,
    pub provider_presets_available: bool,
    pub status: String,
    pub provider_configured: bool,
    pub provider_enabled: bool,
    pub credential_configured: bool,
    pub supports_vision: bool,
    pub text_model_configured: bool,
    pub vision_model_configured: bool,
    pub local_endpoint_count: usize,
    pub cloud_endpoint_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderModelPresetView {
    pub id: String,
    pub display_name: String,
    pub context_window_tokens: u32,
    pub max_output_tokens: u32,
    pub supports_vision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderPresetView {
    pub id: String,
    pub display_name: String,
    pub provider_kind: ProviderKind,
    pub recommended_base_url: String,
    pub models: Vec<AiProviderModelPresetView>,
    pub supports_vision: bool,
    pub documentation_url: String,
    pub builtin: bool,
    pub enabled: bool,
    pub default_preset: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiLabSettingsView {
    pub enabled: bool,
    pub custom_url_approval_required: bool,
    pub configured_user_count: i64,
    pub enabled_user_count: i64,
    pub vision_user_count: i64,
    pub revision: i64,
    pub max_autonomy_mode: AiAutonomyMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiLabSettingsInput {
    pub enabled: bool,
    pub custom_url_approval_required: bool,
    #[serde(default = "default_max_autonomy_mode")]
    pub max_autonomy_mode: AiAutonomyMode,
}

const fn default_max_autonomy_mode() -> AiAutonomyMode {
    AiAutonomyMode::Full
}

const fn autonomy_mode_name(mode: AiAutonomyMode) -> &'static str {
    match mode {
        AiAutonomyMode::Ask => "ask",
        AiAutonomyMode::Auto => "auto",
        AiAutonomyMode::Full => "full",
    }
}

fn parse_autonomy_mode(value: &str) -> Result<AiAutonomyMode, AiProviderStoreError> {
    match value {
        "ask" => Ok(AiAutonomyMode::Ask),
        "auto" => Ok(AiAutonomyMode::Auto),
        "full" => Ok(AiAutonomyMode::Full),
        _ => Err(AiProviderStoreError::Storage),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderEndpointView {
    pub id: Uuid,
    pub provider_kind: ProviderKind,
    pub protocol: AiProviderProtocol,
    pub label: String,
    pub base_url: String,
    pub enabled: bool,
    pub builtin: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiProviderEndpointInput {
    pub provider_kind: ProviderKind,
    #[serde(default)]
    pub protocol: AiProviderProtocol,
    pub label: String,
    pub base_url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelProfileView {
    pub id: Uuid,
    pub name: String,
    pub current_version: i64,
    pub protocol: AiProviderProtocol,
    pub transport: AiProviderTransport,
    pub base_url: String,
    pub model_id: String,
    pub supports_vision: bool,
    pub context_window_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub history_token_budget: u32,
    pub history_turns: u32,
    pub temperature: f32,
    pub timeout_ms: u64,
    pub has_key: bool,
    pub archived_at: Option<DateTime<Utc>>,
    pub is_default_conversation: bool,
    pub is_default_vision: bool,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiModelProfileInput {
    pub name: String,
    pub protocol: AiProviderProtocol,
    pub transport: AiProviderTransport,
    pub base_url: String,
    pub model_id: String,
    pub supports_vision: bool,
    pub context_window_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub history_token_budget: u32,
    pub history_turns: u32,
    pub temperature: f32,
    pub timeout_ms: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<i64>,
}

impl fmt::Debug for SaveAiModelProfileInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SaveAiModelProfileInput")
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("transport", &self.transport)
            .field("base_url", &"[REDACTED]")
            .field("model_id", &"[REDACTED]")
            .field("supports_vision", &self.supports_vision)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("history_token_budget", &self.history_token_budget)
            .field("history_turns", &self.history_turns)
            .field("temperature", &self.temperature)
            .field("timeout_ms", &self.timeout_ms)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("expected_revision", &self.expected_revision)
            .finish()
    }
}

impl SaveAiModelProfileInput {
    fn runtime(&self) -> Result<AssistantRuntimeConfig, AiProviderStoreError> {
        AssistantRuntimeConfig {
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            history_token_budget: self.history_token_budget,
            history_turns: self.history_turns,
            temperature: self.temperature,
            timeout_ms: self.timeout_ms,
        }
        .validate()
        .map_err(|_| AiProviderStoreError::InvalidSettings)
    }

    fn trimmed_key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateAiModelProfileInput {
    pub protocol: AiProviderProtocol,
    pub transport: AiProviderTransport,
    pub base_url: String,
    pub model_id: String,
    pub supports_vision: bool,
    pub context_window_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub history_token_budget: u32,
    pub history_turns: u32,
    pub temperature: f32,
    pub timeout_ms: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    #[serde(default)]
    pub current_version: Option<i64>,
}

impl fmt::Debug for ValidateAiModelProfileInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidateAiModelProfileInput")
            .field("protocol", &self.protocol)
            .field("transport", &self.transport)
            .field("base_url", &"[REDACTED]")
            .field("model_id", &"[REDACTED]")
            .field("supports_vision", &self.supports_vision)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("history_token_budget", &self.history_token_budget)
            .field("history_turns", &self.history_turns)
            .field("temperature", &self.temperature)
            .field("timeout_ms", &self.timeout_ms)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("profile_id", &self.profile_id)
            .field("current_version", &self.current_version)
            .finish()
    }
}

impl ValidateAiModelProfileInput {
    fn profile_binding_hint(&self) -> Result<Option<(Uuid, i64)>, AiProviderStoreError> {
        match (self.profile_id, self.current_version) {
            (None, None) => Ok(None),
            (Some(profile_id), Some(profile_version)) if profile_version > 0 => {
                Ok(Some((profile_id, profile_version)))
            }
            _ => Err(AiProviderStoreError::InvalidSettings),
        }
    }

    fn runtime(&self) -> Result<AssistantRuntimeConfig, AiProviderStoreError> {
        AssistantRuntimeConfig {
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            history_token_budget: self.history_token_budget,
            history_turns: self.history_turns,
            temperature: self.temperature,
            timeout_ms: self.timeout_ms,
        }
        .validate()
        .map_err(|_| AiProviderStoreError::InvalidSettings)
    }

    fn trimmed_key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveAiModelProfileInput {
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelDefaultsView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_conversation_profile_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_vision_profile_id: Option<Uuid>,
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiModelDefaultsInput {
    pub default_conversation_profile_id: Option<Uuid>,
    pub default_vision_profile_id: Option<Uuid>,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelValidationView {
    pub ok: bool,
    pub latency_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

#[derive(Debug, Error)]
pub enum AiProviderStoreError {
    #[error("AI provider settings are invalid")]
    InvalidSettings,
    #[error("AI provider API key is invalid")]
    InvalidCredential,
    #[error("local AI provider URL is not enabled as a laboratory Provider endpoint")]
    LocalUrlForbidden,
    #[error("OpenAI-compatible provider URL is not enabled as a laboratory Provider endpoint")]
    CloudUrlForbidden,
    #[error("laboratory AI is disabled")]
    LabDisabled,
    #[error("AI is disabled for the current user")]
    Disabled,
    #[error("the current user has not selected an AI Provider")]
    ProviderNotSelected,
    #[error("AI provider credential is missing")]
    MissingCredential,
    #[error("the selected AI model profile uses a provider protocol that is not supported yet")]
    UnsupportedProtocol,
    #[error("AI secret master key configuration is invalid")]
    InvalidMasterKey,
    #[error("AI secret could not be encrypted or decrypted")]
    Encryption,
    #[error("AI provider settings storage is unavailable")]
    Storage,
    #[error("AI provider settings are not configured")]
    NotConfigured,
    #[error("AI model profile was not found")]
    ModelProfileNotFound,
    #[error("AI model profile changed concurrently")]
    RevisionConflict,
    #[error("a new API key is required for this AI model profile")]
    CredentialRequired,
}

#[async_trait]
pub trait UserAiProviderStore: Send + Sync {
    async fn get(&self, user_id: Uuid) -> Result<AiProviderSettingsView, AiProviderStoreError>;
    async fn save(
        &self,
        user_id: Uuid,
        input: SaveAiProviderSettingsInput,
        audit: &AuditContext,
    ) -> Result<AiProviderSettingsView, AiProviderStoreError>;
    async fn clear_key(
        &self,
        user_id: Uuid,
        audit: &AuditContext,
    ) -> Result<AiProviderSettingsView, AiProviderStoreError>;
    async fn resolve(&self, user_id: Uuid) -> Result<ResolvedAiProvider, AiProviderStoreError>;
    async fn resolve_for_profile(
        &self,
        user_id: Uuid,
        binding: AiModelProfileBinding,
    ) -> Result<ResolvedAiProvider, AiProviderStoreError>;
    async fn resolve_vision(
        &self,
        user_id: Uuid,
    ) -> Result<ResolvedAiProvider, AiProviderStoreError>;
    async fn diagnostics(
        &self,
        user_id: Uuid,
        lab_id: Uuid,
    ) -> Result<AiProviderDiagnosticsView, AiProviderStoreError>;
    async fn get_lab_settings(
        &self,
        lab_id: Uuid,
    ) -> Result<AiLabSettingsView, AiProviderStoreError>;
    async fn save_lab_settings(
        &self,
        lab_id: Uuid,
        input: SaveAiLabSettingsInput,
        audit: &AuditContext,
    ) -> Result<AiLabSettingsView, AiProviderStoreError>;
    async fn list_provider_presets(
        &self,
        lab_id: Uuid,
    ) -> Result<Vec<AiProviderPresetView>, AiProviderStoreError>;
    async fn list_provider_endpoints(
        &self,
        lab_id: Uuid,
    ) -> Result<Vec<AiProviderEndpointView>, AiProviderStoreError>;
    async fn save_provider_endpoint(
        &self,
        lab_id: Uuid,
        endpoint_id: Option<Uuid>,
        input: SaveAiProviderEndpointInput,
        audit: &AuditContext,
    ) -> Result<AiProviderEndpointView, AiProviderStoreError>;
    async fn disable_provider_endpoint(
        &self,
        lab_id: Uuid,
        endpoint_id: Uuid,
        audit: &AuditContext,
    ) -> Result<AiProviderEndpointView, AiProviderStoreError>;
    async fn list_model_profiles(
        &self,
        user_id: Uuid,
        include_archived: bool,
    ) -> Result<Vec<AiModelProfileView>, AiProviderStoreError>;
    async fn get_model_profile(
        &self,
        user_id: Uuid,
        profile_id: Uuid,
    ) -> Result<AiModelProfileView, AiProviderStoreError>;
    async fn create_model_profile(
        &self,
        user_id: Uuid,
        input: SaveAiModelProfileInput,
        audit: &AuditContext,
    ) -> Result<AiModelProfileView, AiProviderStoreError>;
    async fn update_model_profile(
        &self,
        user_id: Uuid,
        profile_id: Uuid,
        input: SaveAiModelProfileInput,
        audit: &AuditContext,
    ) -> Result<AiModelProfileView, AiProviderStoreError>;
    async fn validate_model_profile(
        &self,
        user_id: Uuid,
        input: ValidateAiModelProfileInput,
    ) -> Result<AiModelValidationView, AiProviderStoreError>;
    async fn clear_model_profile_key(
        &self,
        user_id: Uuid,
        profile_id: Uuid,
        audit: &AuditContext,
    ) -> Result<AiModelProfileView, AiProviderStoreError>;
    async fn archive_model_profile(
        &self,
        user_id: Uuid,
        profile_id: Uuid,
        revision: i64,
        audit: &AuditContext,
    ) -> Result<AiModelProfileView, AiProviderStoreError>;
    async fn get_model_defaults(
        &self,
        user_id: Uuid,
    ) -> Result<AiModelDefaultsView, AiProviderStoreError>;
    async fn save_model_defaults(
        &self,
        user_id: Uuid,
        input: SaveAiModelDefaultsInput,
        audit: &AuditContext,
    ) -> Result<AiModelDefaultsView, AiProviderStoreError>;
}

#[derive(Debug, Default)]
pub struct DisabledAiProviderStore;

#[async_trait]
impl UserAiProviderStore for DisabledAiProviderStore {
    async fn get(&self, _user_id: Uuid) -> Result<AiProviderSettingsView, AiProviderStoreError> {
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
    async fn resolve(&self, _user_id: Uuid) -> Result<ResolvedAiProvider, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
    }
    async fn resolve_for_profile(
        &self,
        _user_id: Uuid,
        _binding: AiModelProfileBinding,
    ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
    }
    async fn resolve_vision(
        &self,
        _user_id: Uuid,
    ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
    }
    async fn diagnostics(
        &self,
        _user_id: Uuid,
        _lab_id: Uuid,
    ) -> Result<AiProviderDiagnosticsView, AiProviderStoreError> {
        Ok(AiProviderDiagnosticsView {
            runtime_configured: false,
            lab_enabled: true,
            user_enabled: true,
            provider_presets_available: false,
            status: "runtime_not_configured".to_owned(),
            provider_configured: false,
            provider_enabled: false,
            credential_configured: false,
            supports_vision: false,
            text_model_configured: false,
            vision_model_configured: false,
            local_endpoint_count: 0,
            cloud_endpoint_count: 0,
        })
    }
    async fn get_lab_settings(
        &self,
        _lab_id: Uuid,
    ) -> Result<AiLabSettingsView, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
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
        _user_id: Uuid,
        _include_archived: bool,
    ) -> Result<Vec<AiModelProfileView>, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
    }

    async fn get_model_profile(
        &self,
        _user_id: Uuid,
        _profile_id: Uuid,
    ) -> Result<AiModelProfileView, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
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
        _user_id: Uuid,
    ) -> Result<AiModelDefaultsView, AiProviderStoreError> {
        Err(AiProviderStoreError::NotConfigured)
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

#[cfg(test)]
mod shared_tests {
    use super::*;

    #[test]
    fn provider_input_debug_is_redacted() {
        let input = SaveAiProviderSettingsInput {
            enabled: true,
            provider_kind: ProviderKind::OpenAiCompatible,
            provider_preset_id: "custom-openai-compatible".to_owned(),
            model: "private-model".to_owned(),
            base_url: "https://private-provider.example.test/v1".to_owned(),
            supports_vision: true,
            vision_model: Some("private-vision-model".to_owned()),
            context_window_tokens: default_context_window_tokens(),
            max_input_tokens: default_max_input_tokens(),
            max_output_tokens: default_max_output_tokens(),
            history_token_budget: default_history_token_budget(),
            history_turns: default_history_turns(),
            temperature: default_temperature(),
            timeout_ms: default_timeout_ms(),
            api_key: Some("private-api-key".to_owned()),
        };
        let debug = format!("{input:?}");
        assert!(!debug.contains("private-model"));
        assert!(!debug.contains("private-provider"));
        assert!(!debug.contains("private-api-key"));
    }

    #[test]
    fn endpoint_protocol_defaults_for_legacy_input_and_is_serialized_in_views() {
        let legacy: SaveAiProviderEndpointInput = serde_json::from_value(serde_json::json!({
            "providerKind": "open_ai_compatible",
            "label": "Legacy endpoint",
            "baseUrl": "https://provider.example.test/v1",
            "enabled": true
        }))
        .unwrap();
        assert_eq!(legacy.protocol, AiProviderProtocol::OpenaiChatCompletions);

        let explicit: SaveAiProviderEndpointInput = serde_json::from_value(serde_json::json!({
            "providerKind": "open_ai_compatible",
            "protocol": "anthropic_messages",
            "label": "Anthropic endpoint",
            "baseUrl": "https://provider.example.test",
            "enabled": true
        }))
        .unwrap();
        assert_eq!(explicit.protocol, AiProviderProtocol::AnthropicMessages);

        let serialized = serde_json::to_value(AiProviderEndpointView {
            id: Uuid::nil(),
            provider_kind: ProviderKind::OpenAiCompatible,
            protocol: AiProviderProtocol::OpenaiResponses,
            label: "Responses endpoint".to_owned(),
            base_url: "https://provider.example.test/v1".to_owned(),
            enabled: true,
            builtin: false,
            revision: 1,
        })
        .unwrap();
        assert_eq!(serialized["protocol"], "openai_responses");
    }

    #[test]
    fn model_management_contract_uses_transport_expected_revision_and_redacts_secrets() {
        let input: SaveAiModelProfileInput = serde_json::from_value(serde_json::json!({
            "name": "自由模型",
            "protocol": "openai_responses",
            "transport": "open_ai_compatible",
            "baseUrl": "https://provider.example.test/v1",
            "modelId": "供应商/自由-model",
            "supportsVision": true,
            "contextWindowTokens": 131072,
            "maxInputTokens": 65536,
            "maxOutputTokens": 4096,
            "historyTokenBudget": 32768,
            "historyTurns": 20,
            "temperature": 0.2,
            "timeoutMs": 120000,
            "apiKey": "must-not-leak",
            "expectedRevision": 7
        }))
        .unwrap();
        assert_eq!(input.protocol, AiProviderProtocol::OpenaiResponses);
        assert_eq!(input.transport, AiProviderTransport::OpenAiCompatible);
        assert_eq!(input.expected_revision, Some(7));
        assert_eq!(input.model_id, "供应商/自由-model");
        assert!(!format!("{input:?}").contains("must-not-leak"));

        let validation: ValidateAiModelProfileInput = serde_json::from_value(serde_json::json!({
            "protocol": "anthropic_messages",
            "transport": "open_ai_compatible",
            "baseUrl": "https://provider.example.test",
            "modelId": "claude-compatible",
            "supportsVision": false,
            "contextWindowTokens": 131072,
            "maxInputTokens": 65536,
            "maxOutputTokens": 4096,
            "historyTokenBudget": 32768,
            "historyTurns": 20,
            "temperature": 0,
            "timeoutMs": 120000,
            "apiKey": "",
            "profileId": Uuid::nil(),
            "currentVersion": 3
        }))
        .unwrap();
        assert_eq!(validation.protocol, AiProviderProtocol::AnthropicMessages);
        assert!(validation.trimmed_key().is_none());
        assert!(!format!("{validation:?}").contains("provider.example"));

        let serialized = serde_json::to_value(AiModelValidationView {
            ok: true,
            latency_ms: 12,
            error_code: None,
        })
        .unwrap();
        assert_eq!(serialized, serde_json::json!({"ok": true, "latencyMs": 12}));
    }

    #[test]
    fn model_management_contract_requires_protocol_transport_and_all_configuration_fields() {
        let save_input = serde_json::json!({
            "name": "Required fields",
            "protocol": "openai_chat_completions",
            "transport": "local_http",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "modelId": "free-form-model",
            "supportsVision": false,
            "contextWindowTokens": 131072,
            "maxInputTokens": 65536,
            "maxOutputTokens": 4096,
            "historyTokenBudget": 32768,
            "historyTurns": 20,
            "temperature": 0.2,
            "timeoutMs": 120000
        });
        let parsed: SaveAiModelProfileInput = serde_json::from_value(save_input.clone()).unwrap();
        assert!(parsed.api_key.is_none());
        assert!(parsed.expected_revision.is_none());
        for field in [
            "name",
            "protocol",
            "transport",
            "baseUrl",
            "modelId",
            "supportsVision",
            "contextWindowTokens",
            "maxInputTokens",
            "maxOutputTokens",
            "historyTokenBudget",
            "historyTurns",
            "temperature",
            "timeoutMs",
        ] {
            let mut missing = save_input.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<SaveAiModelProfileInput>(missing).is_err(),
                "SaveAiModelProfileInput unexpectedly accepted missing {field}"
            );
        }

        let validation_input = serde_json::json!({
            "protocol": "openai_responses",
            "transport": "local_http",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "modelId": "free-form-model",
            "supportsVision": false,
            "contextWindowTokens": 131072,
            "maxInputTokens": 65536,
            "maxOutputTokens": 4096,
            "historyTokenBudget": 32768,
            "historyTurns": 20,
            "temperature": 0.2,
            "timeoutMs": 120000
        });
        serde_json::from_value::<ValidateAiModelProfileInput>(validation_input.clone()).unwrap();
        for field in [
            "protocol",
            "transport",
            "baseUrl",
            "modelId",
            "supportsVision",
            "contextWindowTokens",
            "maxInputTokens",
            "maxOutputTokens",
            "historyTokenBudget",
            "historyTurns",
            "temperature",
            "timeoutMs",
        ] {
            let mut missing = validation_input.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<ValidateAiModelProfileInput>(missing).is_err(),
                "ValidateAiModelProfileInput unexpectedly accepted missing {field}"
            );
        }
    }

    #[test]
    fn model_defaults_and_validation_binding_contracts_are_explicit() {
        assert!(
            serde_json::from_value::<SaveAiModelDefaultsInput>(serde_json::json!({
                "defaultConversationProfileId": null,
                "defaultVisionProfileId": null
            }))
            .is_err()
        );
        let defaults: SaveAiModelDefaultsInput = serde_json::from_value(serde_json::json!({
            "defaultConversationProfileId": null,
            "defaultVisionProfileId": null,
            "expectedRevision": 0
        }))
        .unwrap();
        assert_eq!(defaults.expected_revision, 0);
        let empty_defaults = serde_json::to_value(AiModelDefaultsView {
            default_conversation_profile_id: None,
            default_vision_profile_id: None,
            revision: 0,
        })
        .unwrap();
        assert_eq!(empty_defaults, serde_json::json!({"revision": 0}));

        let validation_input = serde_json::json!({
            "protocol": "openai_chat_completions",
            "transport": "open_ai_compatible",
            "baseUrl": "https://provider.example.test/v1",
            "modelId": "free-form-model",
            "supportsVision": false,
            "contextWindowTokens": 131072,
            "maxInputTokens": 65536,
            "maxOutputTokens": 4096,
            "historyTokenBudget": 32768,
            "historyTurns": 20,
            "temperature": 0.2,
            "timeoutMs": 120000,
            "apiKey": "explicit-key"
        });
        let without_binding: ValidateAiModelProfileInput =
            serde_json::from_value(validation_input.clone()).unwrap();
        assert_eq!(without_binding.profile_binding_hint().unwrap(), None);

        let mut paired = validation_input.clone();
        paired["profileId"] = serde_json::json!(Uuid::nil());
        paired["currentVersion"] = serde_json::json!(1);
        let paired: ValidateAiModelProfileInput = serde_json::from_value(paired).unwrap();
        assert_eq!(
            paired.profile_binding_hint().unwrap(),
            Some((Uuid::nil(), 1))
        );

        for (field, value) in [
            ("profileId", serde_json::json!(Uuid::nil())),
            ("currentVersion", serde_json::json!(1)),
        ] {
            let mut incomplete = validation_input.clone();
            incomplete[field] = value;
            let incomplete: ValidateAiModelProfileInput =
                serde_json::from_value(incomplete).unwrap();
            assert!(matches!(
                incomplete.profile_binding_hint(),
                Err(AiProviderStoreError::InvalidSettings)
            ));
        }
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use base64::{Engine as _, engine::general_purpose};
    use chrono::Utc;
    use muriarc_core::{ActorType, WriteSource};
    use muriarc_store_postgres::PostgresStore;
    use ring::{
        aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
        rand::{SecureRandom, SystemRandom},
    };
    use serde_json::{Value, json};
    use sqlx::{Postgres, Row, Transaction};

    use super::*;

    const NONCE_BYTES: usize = 12;
    const KEY_BYTES: usize = 32;
    const PROFILE_SECRET_MIGRATION_LOCK: i64 = 0x4d55_5249_4152_4341;
    const OFFICIAL_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
    const OFFICIAL_GLM_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
    const OFFICIAL_KIMI_BASE_URL: &str = "https://api.moonshot.cn/v1";
    const OFFICIAL_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

    #[derive(Debug, Clone, Copy)]
    enum SecretAadBinding {
        LegacyUser {
            user_id: Uuid,
        },
        ModelProfileVersion {
            user_id: Uuid,
            profile_id: Uuid,
            profile_version: i64,
        },
    }

    impl SecretAadBinding {
        fn aad(self, master_key_version: i32) -> String {
            match self {
                Self::LegacyUser { user_id } => {
                    format!("MuriArc/ai-provider-secret/v{master_key_version}/{user_id}")
                }
                Self::ModelProfileVersion {
                    user_id,
                    profile_id,
                    profile_version,
                } => format!(
                    "MuriArc/ai-model-profile-secret/v{master_key_version}/{user_id}/{profile_id}/{profile_version}"
                ),
            }
        }
    }

    pub struct AiMasterKey {
        bytes: Zeroizing<Vec<u8>>,
        version: i32,
    }

    impl fmt::Debug for AiMasterKey {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("AiMasterKey")
                .field("bytes", &"[REDACTED]")
                .field("version", &self.version)
                .finish()
        }
    }

    impl AiMasterKey {
        pub fn from_base64(value: &str, version: i32) -> Result<Self, AiProviderStoreError> {
            if version <= 0 {
                return Err(AiProviderStoreError::InvalidMasterKey);
            }
            let decoded = general_purpose::STANDARD
                .decode(value)
                .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(value))
                .map_err(|_| AiProviderStoreError::InvalidMasterKey)?;
            if decoded.len() != KEY_BYTES {
                return Err(AiProviderStoreError::InvalidMasterKey);
            }
            Ok(Self {
                bytes: Zeroizing::new(decoded),
                version,
            })
        }

        fn key(&self) -> Result<LessSafeKey, AiProviderStoreError> {
            UnboundKey::new(&AES_256_GCM, self.bytes.as_slice())
                .map(LessSafeKey::new)
                .map_err(|_| AiProviderStoreError::InvalidMasterKey)
        }
    }

    pub struct PostgresAiProviderStore {
        postgres: PostgresStore,
        master_key: AiMasterKey,
    }

    impl fmt::Debug for PostgresAiProviderStore {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("PostgresAiProviderStore")
                .field("master_key", &self.master_key)
                .finish_non_exhaustive()
        }
    }

    impl PostgresAiProviderStore {
        pub fn new(postgres: PostgresStore, master_key: AiMasterKey) -> Self {
            Self {
                postgres,
                master_key,
            }
        }

        fn config(
            input: &SaveAiProviderSettingsInput,
        ) -> Result<ProviderConfig, AiProviderStoreError> {
            let config = match input.provider_kind {
                ProviderKind::OpenAiCompatible => ProviderConfig::openai_compatible(
                    SERVER_PROVIDER_ID,
                    input.model.clone(),
                    input.base_url.clone(),
                ),
                ProviderKind::LocalHttp => ProviderConfig::local_http(
                    SERVER_PROVIDER_ID,
                    input.model.clone(),
                    input.base_url.clone(),
                ),
            };
            let mut config = config;
            config.timeout_ms = input.timeout_ms;
            input.runtime()?;
            validate_preset_id(&input.provider_preset_id)?;
            BuiltinProvider::from_config(config.clone())
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            Ok(config)
        }

        fn endpoint_config(
            input: &SaveAiProviderEndpointInput,
        ) -> Result<ProviderConfig, AiProviderStoreError> {
            let config = match input.provider_kind {
                ProviderKind::OpenAiCompatible => ProviderConfig::openai_compatible(
                    "endpoint-validation",
                    "muriarc-endpoint-validation",
                    input.base_url.clone(),
                ),
                ProviderKind::LocalHttp => ProviderConfig::local_http(
                    "endpoint-validation",
                    "muriarc-endpoint-validation",
                    input.base_url.clone(),
                ),
            };
            BuiltinProvider::from_config(config.clone())
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            Ok(config)
        }

        fn model_config(
            protocol: AiProviderProtocol,
            transport: AiProviderTransport,
            model_id: &str,
            base_url: &str,
            timeout_ms: u64,
        ) -> Result<ProviderConfig, AiProviderStoreError> {
            let model_id = model_id.trim();
            let base_url = base_url.trim();
            let mut config = match transport {
                AiProviderTransport::OpenAiCompatible => {
                    ProviderConfig::openai_compatible(SERVER_PROVIDER_ID, model_id, base_url)
                }
                AiProviderTransport::LocalHttp => {
                    ProviderConfig::local_http(SERVER_PROVIDER_ID, model_id, base_url)
                }
            }
            .with_protocol(protocol);
            config.timeout_ms = timeout_ms;
            BuiltinProvider::from_config(config.clone())
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            Ok(config)
        }

        #[allow(clippy::too_many_arguments)]
        fn validated_model_configuration(
            name: &str,
            protocol: AiProviderProtocol,
            transport: AiProviderTransport,
            base_url: &str,
            model_id: &str,
            supports_vision: bool,
            runtime: AssistantRuntimeConfig,
        ) -> Result<(String, ProviderConfig), AiProviderStoreError> {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 120 {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let token_total =
                u64::from(runtime.max_input_tokens) + u64::from(runtime.max_output_tokens);
            if !(4_096..=2_000_000).contains(&runtime.context_window_tokens)
                || !(1_024..=1_900_000).contains(&runtime.max_input_tokens)
                || !(1..=131_072).contains(&runtime.max_output_tokens)
                || runtime.history_token_budget > 1_000_000
                || runtime.history_token_budget > runtime.max_input_tokens
                || runtime.history_turns > 100
                || token_total > u64::from(runtime.context_window_tokens)
                || model_id.trim().is_empty()
                || model_id.trim().chars().count() > 256
            {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let _ = supports_vision;
            let config =
                Self::model_config(protocol, transport, model_id, base_url, runtime.timeout_ms)?;
            Ok((name.to_owned(), config))
        }

        fn validated_endpoint_label(label: &str) -> Result<String, AiProviderStoreError> {
            let label = label.trim();
            if label.is_empty() || label.len() > 120 {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            Ok(label.to_owned())
        }

        fn provider_kind_name(kind: ProviderKind) -> &'static str {
            match kind {
                ProviderKind::OpenAiCompatible => "open_ai_compatible",
                ProviderKind::LocalHttp => "local_http",
            }
        }

        fn provider_kind_from_db(value: &str) -> Result<ProviderKind, AiProviderStoreError> {
            match value {
                "open_ai_compatible" => Ok(ProviderKind::OpenAiCompatible),
                "local_http" => Ok(ProviderKind::LocalHttp),
                _ => Err(AiProviderStoreError::Storage),
            }
        }

        fn protocol_name(protocol: AiProviderProtocol) -> &'static str {
            match protocol {
                AiProviderProtocol::OpenaiChatCompletions => "openai_chat_completions",
                AiProviderProtocol::OpenaiResponses => "openai_responses",
                AiProviderProtocol::AnthropicMessages => "anthropic_messages",
            }
        }

        fn protocol_from_db(value: &str) -> Result<AiProviderProtocol, AiProviderStoreError> {
            match value {
                "openai_chat_completions" => Ok(AiProviderProtocol::OpenaiChatCompletions),
                "openai_responses" => Ok(AiProviderProtocol::OpenaiResponses),
                "anthropic_messages" => Ok(AiProviderProtocol::AnthropicMessages),
                _ => Err(AiProviderStoreError::Storage),
            }
        }

        fn transport_name(transport: AiProviderTransport) -> &'static str {
            match transport {
                AiProviderTransport::OpenAiCompatible => "open_ai_compatible",
                AiProviderTransport::LocalHttp => "local_http",
            }
        }

        fn transport_from_provider_kind(kind: ProviderKind) -> AiProviderTransport {
            match kind {
                ProviderKind::OpenAiCompatible => AiProviderTransport::OpenAiCompatible,
                ProviderKind::LocalHttp => AiProviderTransport::LocalHttp,
            }
        }

        fn transport_from_db(value: &str) -> Result<AiProviderTransport, AiProviderStoreError> {
            match value {
                "open_ai_compatible" => Ok(AiProviderTransport::OpenAiCompatible),
                "local_http" => Ok(AiProviderTransport::LocalHttp),
                _ => Err(AiProviderStoreError::Storage),
            }
        }

        fn builtin_endpoint_id(index: u128) -> Uuid {
            Uuid::from_u128(index)
        }

        fn builtin_endpoints() -> Vec<AiProviderEndpointView> {
            [
                (1, "DeepSeek API", OFFICIAL_DEEPSEEK_BASE_URL),
                (2, "智谱 GLM API", OFFICIAL_GLM_BASE_URL),
                (3, "Moonshot / Kimi API", OFFICIAL_KIMI_BASE_URL),
                (4, "OpenAI API", OFFICIAL_OPENAI_BASE_URL),
            ]
            .into_iter()
            .map(|(id, label, base_url)| AiProviderEndpointView {
                id: Self::builtin_endpoint_id(id),
                provider_kind: ProviderKind::OpenAiCompatible,
                protocol: AiProviderProtocol::OpenaiChatCompletions,
                label: label.to_owned(),
                base_url: base_url.to_owned(),
                enabled: true,
                builtin: true,
                revision: 1,
            })
            .collect()
        }

        fn builtin_provider_presets() -> Vec<AiProviderPresetView> {
            vec![
                preset(
                    "deepseek",
                    "DeepSeek",
                    OFFICIAL_DEEPSEEK_BASE_URL,
                    "https://api-docs.deepseek.com/",
                    true,
                    false,
                    &[
                        ("deepseek-chat", "DeepSeek Chat", 131_072, 8_192, false),
                        (
                            "deepseek-reasoner",
                            "DeepSeek Reasoner",
                            131_072,
                            65_536,
                            false,
                        ),
                    ],
                ),
                preset(
                    "zhipu-glm",
                    "智谱 GLM",
                    OFFICIAL_GLM_BASE_URL,
                    "https://docs.bigmodel.cn/cn/guide/develop/http/introduction",
                    false,
                    true,
                    &[
                        ("glm-5.2", "GLM-5.2", 131_072, 65_536, false),
                        ("glm-5v-turbo", "GLM-5V-Turbo", 131_072, 16_384, true),
                    ],
                ),
                preset(
                    "moonshot-kimi",
                    "Moonshot / Kimi",
                    OFFICIAL_KIMI_BASE_URL,
                    "https://platform.kimi.com/docs/api/chat",
                    false,
                    true,
                    &[
                        ("kimi-k3", "Kimi K3", 262_144, 65_536, true),
                        ("kimi-k2.6", "Kimi K2.6", 262_144, 65_536, true),
                    ],
                ),
                preset(
                    "openai",
                    "OpenAI",
                    OFFICIAL_OPENAI_BASE_URL,
                    "https://developers.openai.com/api/docs/guides/latest-model",
                    false,
                    true,
                    &[
                        ("gpt-5.6", "GPT-5.6", 400_000, 128_000, true),
                        ("gpt-5.6-terra", "GPT-5.6 Terra", 400_000, 128_000, true),
                        ("gpt-5.6-luna", "GPT-5.6 Luna", 400_000, 128_000, true),
                    ],
                ),
                preset(
                    "custom-openai-compatible",
                    "自定义 OpenAI-compatible",
                    "",
                    "",
                    false,
                    false,
                    &[],
                ),
            ]
        }

        fn builtin_endpoint(endpoint_id: Uuid) -> bool {
            Self::builtin_endpoints()
                .iter()
                .any(|endpoint| endpoint.id == endpoint_id)
        }

        fn endpoint_view(
            row: &sqlx::postgres::PgRow,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            let kind: String = row
                .try_get("provider_kind")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let protocol: String = row
                .try_get("protocol")
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(AiProviderEndpointView {
                id: row
                    .try_get("id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                provider_kind: Self::provider_kind_from_db(&kind)?,
                protocol: Self::protocol_from_db(&protocol)?,
                label: row
                    .try_get("label")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                base_url: row
                    .try_get("base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                enabled: row
                    .try_get("enabled")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                builtin: row
                    .try_get("builtin")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                revision: row
                    .try_get("revision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            })
        }

        fn endpoint_audit_state(view: &AiProviderEndpointView) -> Value {
            json!({
                "provider_kind": Self::provider_kind_name(view.provider_kind),
                "protocol": Self::protocol_name(view.protocol),
                "label": view.label,
                "enabled": view.enabled,
                "builtin": view.builtin,
                "base_url_present": true,
                "revision": view.revision,
            })
        }

        fn model_profile_view(
            row: &sqlx::postgres::PgRow,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            let protocol = Self::protocol_from_db(
                row.try_get::<String, _>("protocol")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            let transport = Self::transport_from_db(
                row.try_get::<String, _>("transport")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            Ok(AiModelProfileView {
                id: row
                    .try_get("id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                name: row
                    .try_get("name")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                current_version: row
                    .try_get("current_version")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                protocol,
                transport,
                base_url: row
                    .try_get("base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                model_id: row
                    .try_get("model_id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                supports_vision: row
                    .try_get("supports_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                context_window_tokens: numeric_u32(row, "context_window_tokens")?,
                max_input_tokens: numeric_u32(row, "max_input_tokens")?,
                max_output_tokens: numeric_u32(row, "max_output_tokens")?,
                history_token_budget: numeric_u32(row, "history_token_budget")?,
                history_turns: numeric_u32(row, "history_turns")?,
                temperature: row
                    .try_get::<f64, _>("temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    as f32,
                timeout_ms: numeric_u64(row, "timeout_ms")?,
                has_key: row
                    .try_get("has_key")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                archived_at: row
                    .try_get("archived_at")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                is_default_conversation: row
                    .try_get("is_default_conversation")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                is_default_vision: row
                    .try_get("is_default_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                revision: row
                    .try_get("revision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                updated_at: row
                    .try_get("updated_at")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            })
        }

        async fn model_profile_views(
            &self,
            user_id: Uuid,
            include_archived: bool,
        ) -> Result<Vec<AiModelProfileView>, AiProviderStoreError> {
            let rows = sqlx::query(
                "SELECT p.id, p.name, p.current_version, p.archived_at,
                        p.revision, p.created_at, p.updated_at,
                        v.protocol, v.transport, v.base_url, v.model_id,
                        v.supports_vision, v.context_window_tokens,
                        v.max_input_tokens, v.max_output_tokens,
                        v.history_token_budget, v.history_turns,
                        v.temperature, v.timeout_ms,
                        EXISTS(
                            SELECT 1 FROM ai_model_profile_secrets s
                            WHERE s.profile_id = p.id
                              AND s.profile_version = p.current_version
                        ) AS has_key,
                        COALESCE(
                            d.default_conversation_profile_id = p.id, FALSE
                        ) AS is_default_conversation,
                        COALESCE(
                            d.default_vision_profile_id = p.id, FALSE
                        ) AS is_default_vision
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 LEFT JOIN ai_user_model_defaults d
                   ON d.user_id = p.user_id AND d.deleted_at IS NULL
                 WHERE p.user_id = $1
                   AND p.deleted_at IS NULL
                   AND ($2 OR p.archived_at IS NULL)
                 ORDER BY p.updated_at DESC, p.id",
            )
            .bind(user_id)
            .bind(include_archived)
            .fetch_all(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            rows.iter().map(Self::model_profile_view).collect()
        }

        async fn model_profile_view_by_id(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            let row = sqlx::query(
                "SELECT p.id, p.name, p.current_version, p.archived_at,
                        p.revision, p.created_at, p.updated_at,
                        v.protocol, v.transport, v.base_url, v.model_id,
                        v.supports_vision, v.context_window_tokens,
                        v.max_input_tokens, v.max_output_tokens,
                        v.history_token_budget, v.history_turns,
                        v.temperature, v.timeout_ms,
                        EXISTS(
                            SELECT 1 FROM ai_model_profile_secrets s
                            WHERE s.profile_id = p.id
                              AND s.profile_version = p.current_version
                        ) AS has_key,
                        COALESCE(
                            d.default_conversation_profile_id = p.id, FALSE
                        ) AS is_default_conversation,
                        COALESCE(
                            d.default_vision_profile_id = p.id, FALSE
                        ) AS is_default_vision
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 LEFT JOIN ai_user_model_defaults d
                   ON d.user_id = p.user_id AND d.deleted_at IS NULL
                 WHERE p.id = $1 AND p.user_id = $2
                   AND p.deleted_at IS NULL",
            )
            .bind(profile_id)
            .bind(user_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ModelProfileNotFound)?;
            Self::model_profile_view(&row)
        }

        #[allow(clippy::too_many_arguments)]
        async fn insert_model_version(
            transaction: &mut Transaction<'_, Postgres>,
            profile_id: Uuid,
            profile_version: i64,
            protocol: AiProviderProtocol,
            transport: AiProviderTransport,
            config: &ProviderConfig,
            supports_vision: bool,
            runtime: AssistantRuntimeConfig,
        ) -> Result<(), AiProviderStoreError> {
            sqlx::query(
                "INSERT INTO ai_model_profile_versions (
                    profile_id, version, protocol, transport, base_url,
                    normalized_base_url, model_id, supports_vision,
                    context_window_tokens, max_input_tokens, max_output_tokens,
                    history_token_budget, history_turns, temperature,
                    timeout_ms, created_at
                 ) VALUES (
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,now()
                 )",
            )
            .bind(profile_id)
            .bind(profile_version)
            .bind(Self::protocol_name(protocol))
            .bind(Self::transport_name(transport))
            .bind(&config.base_url)
            .bind(normalized_url(&config.base_url))
            .bind(&config.model)
            .bind(supports_vision)
            .bind(i64::from(runtime.context_window_tokens))
            .bind(i64::from(runtime.max_input_tokens))
            .bind(i64::from(runtime.max_output_tokens))
            .bind(i64::from(runtime.history_token_budget))
            .bind(
                i32::try_from(runtime.history_turns)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?,
            )
            .bind(f64::from(runtime.temperature))
            .bind(
                i64::try_from(runtime.timeout_ms)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?,
            )
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn save_model_version_secret(
            &self,
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            profile_id: Uuid,
            profile_version: i64,
            secret: &str,
            audit: &AuditContext,
            reason: &'static str,
        ) -> Result<(), AiProviderStoreError> {
            let (key_version, nonce, ciphertext) =
                self.encrypt_profile_secret(user_id, profile_id, profile_version, secret)?;
            sqlx::query(
                "INSERT INTO ai_model_profile_secrets (
                    profile_id, profile_version, key_version, nonce,
                    ciphertext, created_at, updated_at
                 ) VALUES ($1,$2,$3,$4,$5,now(),now())",
            )
            .bind(profile_id)
            .bind(profile_version)
            .bind(key_version)
            .bind(nonce.to_vec())
            .bind(ciphertext)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            write_human_profile_secret_audit(
                transaction,
                lab_id,
                profile_id,
                profile_version,
                "create",
                audit,
                false,
                true,
                reason,
            )
            .await
        }

        async fn active_user_lab_from_pool(
            &self,
            user_id: Uuid,
        ) -> Result<Uuid, AiProviderStoreError> {
            sqlx::query_scalar(
                "SELECT lab_id FROM users WHERE id = $1 AND status = 'active' AND deleted_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::Storage)
        }

        async fn lab_enabled(&self, lab_id: Uuid) -> Result<bool, AiProviderStoreError> {
            sqlx::query_scalar("SELECT enabled FROM ai_lab_settings WHERE lab_id = $1")
                .bind(lab_id)
                .fetch_optional(self.postgres.pool())
                .await
                .map_err(|_| AiProviderStoreError::Storage)
                .map(|value| value.unwrap_or(true))
        }

        async fn validate_endpoint_for_lab(
            &self,
            lab_id: Uuid,
            protocol: AiProviderProtocol,
            config: &ProviderConfig,
        ) -> Result<(), AiProviderStoreError> {
            let normalized = normalized_url(&config.base_url);
            if protocol == AiProviderProtocol::OpenaiChatCompletions
                && config.kind == ProviderKind::OpenAiCompatible
                && [
                    OFFICIAL_DEEPSEEK_BASE_URL,
                    OFFICIAL_GLM_BASE_URL,
                    OFFICIAL_KIMI_BASE_URL,
                    OFFICIAL_OPENAI_BASE_URL,
                ]
                .into_iter()
                .any(|value| normalized == normalized_url(value))
            {
                return Ok(());
            }
            let approval_required: bool = sqlx::query_scalar(
                "SELECT custom_url_approval_required FROM ai_lab_settings WHERE lab_id = $1",
            )
            .bind(lab_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .unwrap_or(true);
            if !approval_required {
                return Ok(());
            }
            let allowed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ai_provider_endpoints WHERE lab_id = $1 AND protocol = $2 AND normalized_base_url = $3 AND enabled = TRUE)",
            )
            .bind(lab_id)
            .bind(Self::protocol_name(protocol))
            .bind(&normalized)
            .fetch_one(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            match (allowed, config.kind) {
                (true, _) => Ok(()),
                (false, ProviderKind::LocalHttp) => Err(AiProviderStoreError::LocalUrlForbidden),
                (false, ProviderKind::OpenAiCompatible) => {
                    Err(AiProviderStoreError::CloudUrlForbidden)
                }
            }
        }

        async fn endpoint_counts(
            &self,
            lab_id: Uuid,
        ) -> Result<(usize, usize), AiProviderStoreError> {
            let counts: (i64, i64) = sqlx::query_as(
                "SELECT count(*) FILTER (WHERE provider_kind = 'local_http' AND enabled = TRUE)::bigint, count(*) FILTER (WHERE provider_kind = 'open_ai_compatible' AND enabled = TRUE)::bigint FROM ai_provider_endpoints WHERE lab_id = $1",
            )
            .bind(lab_id)
            .fetch_one(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            Ok((counts.0 as usize, counts.1 as usize + 4))
        }

        fn encrypt_for_binding(
            &self,
            binding: SecretAadBinding,
            secret: &str,
        ) -> Result<([u8; NONCE_BYTES], Vec<u8>), AiProviderStoreError> {
            ProviderCredentials::bearer(secret)
                .map_err(|_| AiProviderStoreError::InvalidCredential)?;
            let mut nonce = [0_u8; NONCE_BYTES];
            SystemRandom::new()
                .fill(&mut nonce)
                .map_err(|_| AiProviderStoreError::Encryption)?;
            let mut ciphertext = secret.as_bytes().to_vec();
            self.master_key
                .key()?
                .seal_in_place_append_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(binding.aad(self.master_key.version).as_bytes()),
                    &mut ciphertext,
                )
                .map_err(|_| AiProviderStoreError::Encryption)?;
            Ok((nonce, ciphertext))
        }

        fn decrypt_for_binding(
            &self,
            binding: SecretAadBinding,
            key_version: i32,
            nonce: &[u8],
            ciphertext: &[u8],
        ) -> Result<SensitiveSecret, AiProviderStoreError> {
            if key_version != self.master_key.version || nonce.len() != NONCE_BYTES {
                return Err(AiProviderStoreError::Encryption);
            }
            let nonce: [u8; NONCE_BYTES] = nonce
                .try_into()
                .map_err(|_| AiProviderStoreError::Encryption)?;
            let mut plaintext = ciphertext.to_vec();
            let opened = self
                .master_key
                .key()?
                .open_in_place(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(binding.aad(key_version).as_bytes()),
                    &mut plaintext,
                )
                .map_err(|_| AiProviderStoreError::Encryption)?;
            let secret =
                String::from_utf8(opened.to_vec()).map_err(|_| AiProviderStoreError::Encryption)?;
            plaintext.fill(0);
            Ok(SensitiveSecret::new(secret))
        }

        fn encrypt(
            &self,
            user_id: Uuid,
            secret: &str,
        ) -> Result<([u8; NONCE_BYTES], Vec<u8>), AiProviderStoreError> {
            self.encrypt_for_binding(SecretAadBinding::LegacyUser { user_id }, secret)
        }

        fn decrypt(
            &self,
            user_id: Uuid,
            key_version: i32,
            nonce: &[u8],
            ciphertext: &[u8],
        ) -> Result<SensitiveSecret, AiProviderStoreError> {
            self.decrypt_for_binding(
                SecretAadBinding::LegacyUser { user_id },
                key_version,
                nonce,
                ciphertext,
            )
        }

        pub fn encrypt_profile_secret(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            profile_version: i64,
            secret: &str,
        ) -> Result<(i32, [u8; NONCE_BYTES], Vec<u8>), AiProviderStoreError> {
            if profile_version <= 0 {
                return Err(AiProviderStoreError::Encryption);
            }
            let (nonce, ciphertext) = self.encrypt_for_binding(
                SecretAadBinding::ModelProfileVersion {
                    user_id,
                    profile_id,
                    profile_version,
                },
                secret,
            )?;
            Ok((self.master_key.version, nonce, ciphertext))
        }

        pub fn decrypt_profile_secret(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            profile_version: i64,
            key_version: i32,
            nonce: &[u8],
            ciphertext: &[u8],
        ) -> Result<SensitiveSecret, AiProviderStoreError> {
            if profile_version <= 0 {
                return Err(AiProviderStoreError::Encryption);
            }
            self.decrypt_for_binding(
                SecretAadBinding::ModelProfileVersion {
                    user_id,
                    profile_id,
                    profile_version,
                },
                key_version,
                nonce,
                ciphertext,
            )
        }

        /// Re-encrypts a legacy user-bound credential for one immutable model
        /// profile version without exposing the plaintext to the caller.
        ///
        /// Persistence remains an explicit caller responsibility so a migration
        /// can store the returned key version, nonce, and ciphertext atomically.
        pub fn reencrypt_legacy_secret_for_profile(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            profile_version: i64,
            legacy_key_version: i32,
            legacy_nonce: &[u8],
            legacy_ciphertext: &[u8],
        ) -> Result<(i32, [u8; NONCE_BYTES], Vec<u8>), AiProviderStoreError> {
            let secret =
                self.decrypt(user_id, legacy_key_version, legacy_nonce, legacy_ciphertext)?;
            self.encrypt_profile_secret(user_id, profile_id, profile_version, secret.as_str())
        }

        /// Copies legacy user-bound credentials into the immutable default model
        /// profile versions using version-bound AAD.
        ///
        /// The old ciphertext is intentionally retained for forward-only
        /// compatibility. A single transaction and advisory lock make the
        /// migration safe to run at every server startup.
        pub async fn migrate_legacy_profile_secrets(&self) -> Result<u64, AiProviderStoreError> {
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(PROFILE_SECRET_MIGRATION_LOCK)
                .execute(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let rows = sqlx::query(
                "SELECT s.user_id, u.lab_id, s.secret_key_version, s.secret_nonce,
                        s.secret_ciphertext, d.default_conversation_profile_id,
                        d.default_vision_profile_id
                 FROM ai_provider_settings s
                 JOIN users u
                   ON u.id = s.user_id AND u.deleted_at IS NULL
                 JOIN ai_user_model_defaults d
                   ON d.user_id = s.user_id AND d.deleted_at IS NULL
                 WHERE s.secret_key_version IS NOT NULL
                    OR s.secret_nonce IS NOT NULL
                    OR s.secret_ciphertext IS NOT NULL
                 FOR UPDATE OF s, d",
            )
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;

            let mut migrated = 0_u64;
            for row in rows {
                let user_id: Uuid = row
                    .try_get("user_id")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let lab_id: Uuid = row
                    .try_get("lab_id")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let key_version: Option<i32> = row
                    .try_get("secret_key_version")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let nonce: Option<Vec<u8>> = row
                    .try_get("secret_nonce")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let ciphertext: Option<Vec<u8>> = row
                    .try_get("secret_ciphertext")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let legacy_secret = match (key_version, nonce, ciphertext) {
                    (Some(version), Some(nonce), Some(ciphertext)) => {
                        self.decrypt(user_id, version, &nonce, &ciphertext)?
                    }
                    _ => return Err(AiProviderStoreError::Encryption),
                };
                let conversation_profile_id: Uuid = row
                    .try_get::<Option<Uuid>, _>("default_conversation_profile_id")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .ok_or(AiProviderStoreError::Storage)?;
                let mut profile_ids = vec![conversation_profile_id];
                if let Some(vision_profile_id) = row
                    .try_get::<Option<Uuid>, _>("default_vision_profile_id")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    && vision_profile_id != conversation_profile_id
                {
                    profile_ids.push(vision_profile_id);
                }

                for profile_id in profile_ids {
                    let profile_version = Self::active_profile_current_version(
                        &mut transaction,
                        profile_id,
                        user_id,
                        lab_id,
                    )
                    .await?;
                    let existing = sqlx::query(
                        "SELECT key_version, nonce, ciphertext
                         FROM ai_model_profile_secrets
                         WHERE profile_id = $1 AND profile_version = $2
                         FOR UPDATE",
                    )
                    .bind(profile_id)
                    .bind(profile_version)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                    if let Some(existing) = existing {
                        let existing_version: i32 = existing
                            .try_get("key_version")
                            .map_err(|_| AiProviderStoreError::Storage)?;
                        let existing_nonce: Vec<u8> = existing
                            .try_get("nonce")
                            .map_err(|_| AiProviderStoreError::Storage)?;
                        let existing_ciphertext: Vec<u8> = existing
                            .try_get("ciphertext")
                            .map_err(|_| AiProviderStoreError::Storage)?;
                        self.decrypt_profile_secret(
                            user_id,
                            profile_id,
                            profile_version,
                            existing_version,
                            &existing_nonce,
                            &existing_ciphertext,
                        )?;
                        continue;
                    }

                    let (profile_key_version, profile_nonce, profile_ciphertext) = self
                        .encrypt_profile_secret(
                            user_id,
                            profile_id,
                            profile_version,
                            legacy_secret.as_str(),
                        )?;
                    sqlx::query(
                        "INSERT INTO ai_model_profile_secrets (
                            profile_id, profile_version, key_version, nonce,
                            ciphertext, created_at, updated_at
                         ) VALUES ($1, $2, $3, $4, $5, now(), now())",
                    )
                    .bind(profile_id)
                    .bind(profile_version)
                    .bind(profile_key_version)
                    .bind(profile_nonce.to_vec())
                    .bind(profile_ciphertext)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                    write_profile_secret_audit(
                        &mut transaction,
                        lab_id,
                        profile_id,
                        profile_version,
                        "create",
                        "migration",
                        None,
                        "MuriArc AI secret migration",
                        "migration",
                        None,
                        false,
                        true,
                        "Legacy AI credential migrated to profile-version-bound encryption",
                    )
                    .await?;
                    migrated += 1;
                }
            }

            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(migrated)
        }

        async fn ensure_profile_owner(
            transaction: &mut Transaction<'_, Postgres>,
            profile_id: Uuid,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<(), AiProviderStoreError> {
            let owner: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT user_id, lab_id
                 FROM ai_model_profiles
                 WHERE id = $1
                   AND archived_at IS NULL
                   AND deleted_at IS NULL
                 FOR SHARE",
            )
            .bind(profile_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if owner != Some((user_id, lab_id)) {
                return Err(AiProviderStoreError::Storage);
            }
            Ok(())
        }

        async fn active_profile_current_version(
            transaction: &mut Transaction<'_, Postgres>,
            profile_id: Uuid,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<i64, AiProviderStoreError> {
            sqlx::query_scalar(
                "SELECT current_version
                 FROM ai_model_profiles
                 WHERE id = $1 AND user_id = $2 AND lab_id = $3
                   AND archived_at IS NULL AND deleted_at IS NULL
                 FOR SHARE",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::Storage)
        }

        async fn default_profile_ids(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<Vec<Uuid>, AiProviderStoreError> {
            let defaults: Option<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
                "SELECT default_conversation_profile_id, default_vision_profile_id
                 FROM ai_user_model_defaults
                 WHERE user_id = $1 AND deleted_at IS NULL
                 FOR UPDATE",
            )
            .bind(user_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let Some((conversation, vision)) = defaults else {
                return Ok(Vec::new());
            };
            let mut profile_ids = Vec::with_capacity(2);
            if let Some(profile_id) = conversation {
                Self::ensure_profile_owner(transaction, profile_id, user_id, lab_id).await?;
                profile_ids.push(profile_id);
            }
            if let Some(profile_id) = vision
                && !profile_ids.contains(&profile_id)
            {
                Self::ensure_profile_owner(transaction, profile_id, user_id, lab_id).await?;
                profile_ids.push(profile_id);
            }
            Ok(profile_ids)
        }

        async fn replace_default_profile_secrets(
            &self,
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            secret: &str,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            for profile_id in Self::default_profile_ids(transaction, user_id, lab_id).await? {
                let profile_version =
                    Self::active_profile_current_version(transaction, profile_id, user_id, lab_id)
                        .await?;
                let existed: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                        SELECT 1
                        FROM ai_model_profile_secrets
                        WHERE profile_id = $1 AND profile_version = $2
                     )",
                )
                .bind(profile_id)
                .bind(profile_version)
                .fetch_one(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                let (key_version, nonce, ciphertext) =
                    self.encrypt_profile_secret(user_id, profile_id, profile_version, secret)?;
                sqlx::query(
                    "INSERT INTO ai_model_profile_secrets (
                        profile_id, profile_version, key_version, nonce,
                        ciphertext, created_at, updated_at
                     ) VALUES ($1, $2, $3, $4, $5, now(), now())
                     ON CONFLICT (profile_id, profile_version) DO UPDATE
                     SET key_version = EXCLUDED.key_version,
                         nonce = EXCLUDED.nonce,
                         ciphertext = EXCLUDED.ciphertext,
                         updated_at = now()",
                )
                .bind(profile_id)
                .bind(profile_version)
                .bind(key_version)
                .bind(nonce.to_vec())
                .bind(ciphertext)
                .execute(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                write_human_profile_secret_audit(
                    transaction,
                    lab_id,
                    profile_id,
                    profile_version,
                    if existed { "update" } else { "create" },
                    audit,
                    existed,
                    true,
                    "AI model profile credential replaced",
                )
                .await?;
            }
            Ok(())
        }

        async fn preserve_default_profile_secrets(
            &self,
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            legacy_secret: Option<&SensitiveSecret>,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            let profile_ids = Self::default_profile_ids(transaction, user_id, lab_id).await?;
            if profile_ids.is_empty() {
                return Ok(());
            }
            for profile_id in profile_ids {
                let profile_version =
                    Self::active_profile_current_version(transaction, profile_id, user_id, lab_id)
                        .await?;
                let existing = sqlx::query(
                    "SELECT key_version, nonce, ciphertext
                     FROM ai_model_profile_secrets
                     WHERE profile_id = $1 AND profile_version = $2
                     FOR UPDATE",
                )
                .bind(profile_id)
                .bind(profile_version)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                if let Some(existing) = existing {
                    let key_version: i32 = existing
                        .try_get("key_version")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let nonce: Vec<u8> = existing
                        .try_get("nonce")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let ciphertext: Vec<u8> = existing
                        .try_get("ciphertext")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    self.decrypt_profile_secret(
                        user_id,
                        profile_id,
                        profile_version,
                        key_version,
                        &nonce,
                        &ciphertext,
                    )?;
                    continue;
                }
                if legacy_secret.is_none() {
                    continue;
                }

                let previous = sqlx::query(
                    "SELECT profile_version, key_version, nonce, ciphertext
                     FROM ai_model_profile_secrets
                     WHERE profile_id = $1 AND profile_version < $2
                     ORDER BY profile_version DESC
                     LIMIT 1
                     FOR UPDATE",
                )
                .bind(profile_id)
                .bind(profile_version)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                let copied_secret = if let Some(previous) = previous {
                    let previous_profile_version: i64 = previous
                        .try_get("profile_version")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let key_version: i32 = previous
                        .try_get("key_version")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let nonce: Vec<u8> = previous
                        .try_get("nonce")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let ciphertext: Vec<u8> = previous
                        .try_get("ciphertext")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    Some(self.decrypt_profile_secret(
                        user_id,
                        profile_id,
                        previous_profile_version,
                        key_version,
                        &nonce,
                        &ciphertext,
                    )?)
                } else {
                    None
                };
                let Some(secret) = copied_secret.as_ref().or(legacy_secret) else {
                    continue;
                };
                let (key_version, nonce, ciphertext) = self.encrypt_profile_secret(
                    user_id,
                    profile_id,
                    profile_version,
                    secret.as_str(),
                )?;
                sqlx::query(
                    "INSERT INTO ai_model_profile_secrets (
                        profile_id, profile_version, key_version, nonce,
                        ciphertext, created_at, updated_at
                     ) VALUES ($1, $2, $3, $4, $5, now(), now())",
                )
                .bind(profile_id)
                .bind(profile_version)
                .bind(key_version)
                .bind(nonce.to_vec())
                .bind(ciphertext)
                .execute(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                write_human_profile_secret_audit(
                    transaction,
                    lab_id,
                    profile_id,
                    profile_version,
                    "create",
                    audit,
                    false,
                    true,
                    "AI model profile credential synchronized",
                )
                .await?;
            }
            Ok(())
        }

        async fn clear_current_default_profile_secrets(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            audit: &AuditContext,
            reason: &'static str,
        ) -> Result<(), AiProviderStoreError> {
            for profile_id in Self::default_profile_ids(transaction, user_id, lab_id).await? {
                let profile_version =
                    Self::active_profile_current_version(transaction, profile_id, user_id, lab_id)
                        .await?;
                let deleted = sqlx::query(
                    "DELETE FROM ai_model_profile_secrets
                     WHERE profile_id = $1 AND profile_version = $2
                     RETURNING profile_version",
                )
                .bind(profile_id)
                .bind(profile_version)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                if deleted.is_some() {
                    write_human_profile_secret_audit(
                        transaction,
                        lab_id,
                        profile_id,
                        profile_version,
                        "delete",
                        audit,
                        true,
                        false,
                        reason,
                    )
                    .await?;
                }
            }
            Ok(())
        }

        async fn compatibility_profile_ids(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<Vec<Uuid>, AiProviderStoreError> {
            let mut profile_ids = Self::default_profile_ids(transaction, user_id, lab_id).await?;
            for vision in [false, true] {
                let profile_id =
                    Self::compatibility_profile_id(transaction, user_id, vision).await?;
                let owned: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                        SELECT 1
                        FROM ai_model_profiles
                        WHERE id = $1 AND user_id = $2 AND lab_id = $3
                     )",
                )
                .bind(profile_id)
                .bind(user_id)
                .bind(lab_id)
                .fetch_one(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                if owned && !profile_ids.contains(&profile_id) {
                    profile_ids.push(profile_id);
                }
            }
            Ok(profile_ids)
        }

        async fn clear_all_compatibility_profile_secrets(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            for profile_id in Self::compatibility_profile_ids(transaction, user_id, lab_id).await? {
                let deleted_versions: Vec<i64> = sqlx::query_scalar(
                    "DELETE FROM ai_model_profile_secrets
                     WHERE profile_id = $1
                     RETURNING profile_version",
                )
                .bind(profile_id)
                .fetch_all(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                for profile_version in deleted_versions {
                    write_human_profile_secret_audit(
                        transaction,
                        lab_id,
                        profile_id,
                        profile_version,
                        "delete",
                        audit,
                        true,
                        false,
                        "AI model profile credential cleared",
                    )
                    .await?;
                }
            }
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn append_profile_version_if_changed(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            profile_id: Uuid,
            config: &ProviderConfig,
            model: &str,
            supports_vision: bool,
            runtime: AssistantRuntimeConfig,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            let row = sqlx::query(
                "SELECT p.current_version, p.revision, v.protocol,
                        v.transport, v.base_url, v.normalized_base_url,
                        v.model_id, v.supports_vision, v.context_window_tokens,
                        v.max_input_tokens, v.max_output_tokens,
                        v.history_token_budget, v.history_turns, v.temperature,
                        v.timeout_ms
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 WHERE p.id = $1
                   AND p.user_id = $2
                   AND p.lab_id = $3
                   AND p.archived_at IS NULL
                   AND p.deleted_at IS NULL
                 FOR UPDATE OF p",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ProviderNotSelected)?;
            let current_version: i64 = row
                .try_get("current_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_protocol: String = row
                .try_get("protocol")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let normalized_base_url = normalized_url(&config.base_url);
            let unchanged = current_protocol
                == Self::protocol_name(AiProviderProtocol::OpenaiChatCompletions)
                && row
                    .try_get::<String, _>("transport")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == Self::transport_name(Self::transport_from_provider_kind(config.kind))
                && row
                    .try_get::<String, _>("base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == config.base_url
                && row
                    .try_get::<String, _>("normalized_base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == normalized_base_url
                && row
                    .try_get::<String, _>("model_id")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == model
                && row
                    .try_get::<bool, _>("supports_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == supports_vision
                && numeric_u32(&row, "context_window_tokens")? == runtime.context_window_tokens
                && numeric_u32(&row, "max_input_tokens")? == runtime.max_input_tokens
                && numeric_u32(&row, "max_output_tokens")? == runtime.max_output_tokens
                && numeric_u32(&row, "history_token_budget")? == runtime.history_token_budget
                && numeric_u32(&row, "history_turns")? == runtime.history_turns
                && row
                    .try_get::<f64, _>("temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == f64::from(runtime.temperature)
                && numeric_u64(&row, "timeout_ms")? == runtime.timeout_ms;
            if unchanged {
                return Ok(());
            }

            let next_version = current_version
                .checked_add(1)
                .ok_or(AiProviderStoreError::Storage)?;
            sqlx::query(
                "INSERT INTO ai_model_profile_versions (
                    profile_id, version, protocol, transport, base_url,
                    normalized_base_url, model_id, supports_vision,
                    context_window_tokens,
                    max_input_tokens, max_output_tokens, history_token_budget,
                    history_turns, temperature, timeout_ms, created_at
                 ) VALUES (
                    $1, $2, 'openai_chat_completions', $3, $4, $5, $6, $7,
                    $8, $9, $10, $11, $12, $13, $14, now()
                 )",
            )
            .bind(profile_id)
            .bind(next_version)
            .bind(Self::transport_name(Self::transport_from_provider_kind(
                config.kind,
            )))
            .bind(&config.base_url)
            .bind(&normalized_base_url)
            .bind(model)
            .bind(supports_vision)
            .bind(i64::from(runtime.context_window_tokens))
            .bind(i64::from(runtime.max_input_tokens))
            .bind(i64::from(runtime.max_output_tokens))
            .bind(i64::from(runtime.history_token_budget))
            .bind(
                i32::try_from(runtime.history_turns)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?,
            )
            .bind(f64::from(runtime.temperature))
            .bind(
                i64::try_from(runtime.timeout_ms)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?,
            )
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let updated = sqlx::query(
                "UPDATE ai_model_profiles
                 SET current_version = $2, updated_at = now(), revision = revision + 1
                 WHERE id = $1 AND current_version = $3",
            )
            .bind(profile_id)
            .bind(next_version)
            .bind(current_version)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if updated.rows_affected() != 1 {
                return Err(AiProviderStoreError::Storage);
            }
            write_profile_configuration_audit(
                transaction,
                lab_id,
                profile_id,
                current_version,
                next_version,
                supports_vision,
                audit,
            )
            .await
        }

        async fn compatibility_profile_id(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            vision: bool,
        ) -> Result<Uuid, AiProviderStoreError> {
            let namespace = if vision {
                "muriarc-ai-vision-profile:"
            } else {
                "muriarc-ai-text-profile:"
            };
            sqlx::query_scalar(
                "SELECT (
                    substr(md5($1::text || $2::text), 1, 8) || '-' ||
                    substr(md5($1::text || $2::text), 9, 4) || '-' ||
                    substr(md5($1::text || $2::text), 13, 4) || '-' ||
                    substr(md5($1::text || $2::text), 17, 4) || '-' ||
                    substr(md5($1::text || $2::text), 21, 12)
                 )::uuid",
            )
            .bind(namespace)
            .bind(user_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)
        }

        async fn profile_is_active_for_user(
            transaction: &mut Transaction<'_, Postgres>,
            profile_id: Uuid,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<bool, AiProviderStoreError> {
            sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1
                    FROM ai_model_profiles
                    WHERE id = $1 AND user_id = $2 AND lab_id = $3
                      AND archived_at IS NULL AND deleted_at IS NULL
                 )",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)
        }

        #[allow(clippy::too_many_arguments)]
        async fn create_compatibility_profile_if_missing(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            profile_id: Uuid,
            vision: bool,
            config: &ProviderConfig,
            model: &str,
            runtime: AssistantRuntimeConfig,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            let name = if vision {
                "Migrated vision model"
            } else {
                "Migrated default model"
            };
            let inserted = sqlx::query(
                "INSERT INTO ai_model_profiles (
                    id, lab_id, user_id, name, current_version,
                    created_at, updated_at, revision
                 ) VALUES ($1, $2, $3, $4, 1, now(), now(), 1)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(profile_id)
            .bind(lab_id)
            .bind(user_id)
            .bind(name)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if inserted.rows_affected() == 1 {
                sqlx::query(
                    "INSERT INTO ai_model_profile_versions (
                        profile_id, version, protocol, transport, base_url,
                        normalized_base_url, model_id, supports_vision,
                        context_window_tokens, max_input_tokens,
                        max_output_tokens, history_token_budget, history_turns,
                        temperature, timeout_ms, created_at
                     ) VALUES (
                        $1, 1, 'openai_chat_completions', $2, $3, $4, $5, $6,
                        $7, $8, $9, $10, $11, $12, $13, now()
                     )",
                )
                .bind(profile_id)
                .bind(Self::transport_name(Self::transport_from_provider_kind(
                    config.kind,
                )))
                .bind(&config.base_url)
                .bind(normalized_url(&config.base_url))
                .bind(model)
                .bind(vision)
                .bind(i64::from(runtime.context_window_tokens))
                .bind(i64::from(runtime.max_input_tokens))
                .bind(i64::from(runtime.max_output_tokens))
                .bind(i64::from(runtime.history_token_budget))
                .bind(
                    i32::try_from(runtime.history_turns)
                        .map_err(|_| AiProviderStoreError::InvalidSettings)?,
                )
                .bind(f64::from(runtime.temperature))
                .bind(
                    i64::try_from(runtime.timeout_ms)
                        .map_err(|_| AiProviderStoreError::InvalidSettings)?,
                )
                .execute(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                write_profile_configuration_audit(
                    transaction,
                    lab_id,
                    profile_id,
                    0,
                    1,
                    vision,
                    audit,
                )
                .await?;
            }
            if !Self::profile_is_active_for_user(transaction, profile_id, user_id, lab_id).await? {
                return Err(AiProviderStoreError::ProviderNotSelected);
            }
            Ok(())
        }

        async fn materialize_and_append_default_profile_versions(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
            lab_id: Uuid,
            config: &ProviderConfig,
            runtime: AssistantRuntimeConfig,
            vision_model: Option<&str>,
            audit: &AuditContext,
        ) -> Result<(), AiProviderStoreError> {
            let supports_vision = vision_model.is_some();
            let defaults = sqlx::query(
                "SELECT default_conversation_profile_id,
                        default_vision_profile_id, deleted_at
                 FROM ai_user_model_defaults
                 WHERE user_id = $1
                 FOR UPDATE",
            )
            .bind(user_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let old_conversation_profile_id = defaults
                .as_ref()
                .and_then(|row| row.try_get("default_conversation_profile_id").ok())
                .flatten();
            let old_vision_profile_id = defaults
                .as_ref()
                .and_then(|row| row.try_get("default_vision_profile_id").ok())
                .flatten();
            let defaults_deleted = defaults
                .as_ref()
                .and_then(|row| {
                    row.try_get::<Option<chrono::DateTime<Utc>>, _>("deleted_at")
                        .ok()
                })
                .flatten()
                .is_some();

            let conversation_profile_id = match old_conversation_profile_id {
                Some(profile_id)
                    if Self::profile_is_active_for_user(
                        transaction,
                        profile_id,
                        user_id,
                        lab_id,
                    )
                    .await? =>
                {
                    profile_id
                }
                _ => {
                    let profile_id =
                        Self::compatibility_profile_id(transaction, user_id, false).await?;
                    Self::create_compatibility_profile_if_missing(
                        transaction,
                        user_id,
                        lab_id,
                        profile_id,
                        false,
                        config,
                        &config.model,
                        runtime,
                        audit,
                    )
                    .await?;
                    profile_id
                }
            };

            let mut vision_profile_id = supports_vision.then_some(old_vision_profile_id).flatten();
            if supports_vision {
                let current_is_active = match vision_profile_id {
                    Some(profile_id) => {
                        Self::profile_is_active_for_user(transaction, profile_id, user_id, lab_id)
                            .await?
                    }
                    None => false,
                };
                if !current_is_active {
                    let profile_id =
                        Self::compatibility_profile_id(transaction, user_id, true).await?;
                    Self::create_compatibility_profile_if_missing(
                        transaction,
                        user_id,
                        lab_id,
                        profile_id,
                        true,
                        config,
                        vision_model.ok_or(AiProviderStoreError::InvalidSettings)?,
                        runtime,
                        audit,
                    )
                    .await?;
                    vision_profile_id = Some(profile_id);
                }
            }

            let defaults_changed = defaults.is_none()
                || defaults_deleted
                || old_conversation_profile_id != Some(conversation_profile_id)
                || old_vision_profile_id != vision_profile_id;
            if defaults_changed {
                sqlx::query(
                    "INSERT INTO ai_user_model_defaults (
                        user_id, default_conversation_profile_id,
                        default_vision_profile_id, created_at, updated_at,
                        deleted_at, revision
                     ) VALUES ($1, $2, $3, now(), now(), NULL, 1)
                     ON CONFLICT (user_id) DO UPDATE
                     SET default_conversation_profile_id =
                            EXCLUDED.default_conversation_profile_id,
                         default_vision_profile_id =
                            EXCLUDED.default_vision_profile_id,
                         updated_at = now(),
                         deleted_at = NULL,
                         revision = ai_user_model_defaults.revision + 1",
                )
                .bind(user_id)
                .bind(conversation_profile_id)
                .bind(vision_profile_id)
                .execute(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                write_model_defaults_audit(
                    transaction,
                    lab_id,
                    user_id,
                    old_conversation_profile_id,
                    old_vision_profile_id,
                    conversation_profile_id,
                    vision_profile_id,
                    defaults.is_none(),
                    audit,
                )
                .await?;
            }

            let shared_with_vision =
                supports_vision && vision_profile_id == Some(conversation_profile_id);
            if shared_with_vision && vision_model != Some(config.model.as_str()) {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            Self::append_profile_version_if_changed(
                transaction,
                user_id,
                lab_id,
                conversation_profile_id,
                config,
                &config.model,
                shared_with_vision,
                runtime,
                audit,
            )
            .await?;
            if supports_vision
                && let (Some(vision_profile_id), Some(vision_model)) =
                    (vision_profile_id, vision_model)
                && vision_profile_id != conversation_profile_id
            {
                Self::append_profile_version_if_changed(
                    transaction,
                    user_id,
                    lab_id,
                    vision_profile_id,
                    config,
                    vision_model,
                    true,
                    runtime,
                    audit,
                )
                .await?;
            }
            Ok(())
        }

        async fn selected_profile_binding(
            &self,
            user_id: Uuid,
            vision: bool,
        ) -> Result<AiModelProfileBinding, AiProviderStoreError> {
            let row = sqlx::query(
                "SELECT p.id AS profile_id, p.current_version AS profile_version
                 FROM ai_user_model_defaults d
                 JOIN ai_model_profiles p
                   ON p.id = CASE
                       WHEN $2 THEN d.default_vision_profile_id
                       ELSE d.default_conversation_profile_id
                   END
                  AND p.user_id = d.user_id
                  AND p.archived_at IS NULL
                  AND p.deleted_at IS NULL
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                  AND (NOT $2 OR v.supports_vision)
                 WHERE d.user_id = $1 AND d.deleted_at IS NULL",
            )
            .bind(user_id)
            .bind(vision)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ProviderNotSelected)?;
            let profile_id: Uuid = row
                .try_get("profile_id")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let profile_version: i64 = row
                .try_get("profile_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(AiModelProfileBinding {
                profile_id,
                profile_version,
            })
        }

        async fn row(
            &self,
            user_id: Uuid,
        ) -> Result<Option<sqlx::postgres::PgRow>, AiProviderStoreError> {
            sqlx::query("SELECT enabled, provider_config, provider_preset_id, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, revision FROM ai_provider_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(self.postgres.pool())
                .await
                .map_err(|_| AiProviderStoreError::Storage)
        }

        async fn locked_row(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
        ) -> Result<Option<sqlx::postgres::PgRow>, AiProviderStoreError> {
            sqlx::query("SELECT enabled, provider_config, provider_preset_id, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, revision FROM ai_provider_settings WHERE user_id = $1 FOR UPDATE")
                .bind(user_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)
        }

        async fn active_user_lab(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
        ) -> Result<Uuid, AiProviderStoreError> {
            sqlx::query_scalar(
                "SELECT lab_id
                 FROM users
                 WHERE id = $1 AND status = 'active' AND deleted_at IS NULL
                 FOR UPDATE",
            )
            .bind(user_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::Storage)
        }

        /// Copies the deployment Root's credential-free local conversation
        /// profile into a newly created or reactivated account.
        ///
        /// The copy is deliberately narrow: cloud transports, credentials,
        /// vision profiles, unapproved endpoints, and ambiguous pre-existing
        /// target settings are never inherited. The resulting rows are owned
        /// by `target_user_id`, so later changes remain isolated per user.
        pub(crate) async fn inherit_environment_root_local_default(
            transaction: &mut Transaction<'_, Postgres>,
            lab_id: Uuid,
            environment_root_user_id: Uuid,
            target_user_id: Uuid,
            audit: &AuditContext,
        ) -> Result<bool, AiProviderStoreError> {
            let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
            if environment_root_user_id == target_user_id {
                return Ok(false);
            }
            let target_is_unconfigured: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1
                    FROM users target
                    WHERE target.id = $1
                      AND target.lab_id = $2
                      AND target.deleted_at IS NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM ai_provider_settings settings
                          WHERE settings.user_id = target.id
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM ai_model_profiles profile
                          WHERE profile.user_id = target.id
                            AND profile.deleted_at IS NULL
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM ai_user_model_defaults defaults
                          WHERE defaults.user_id = target.id
                            AND defaults.deleted_at IS NULL
                      )
                )",
            )
            .bind(target_user_id)
            .bind(lab_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if !target_is_unconfigured {
                return Ok(false);
            }

            let source = sqlx::query(
                "SELECT settings.provider_config, settings.provider_preset_id,
                        settings.context_window_tokens,
                        settings.max_input_tokens, settings.max_output_tokens,
                        settings.history_token_budget, settings.history_turns,
                        settings.temperature, settings.timeout_ms,
                        version.protocol, version.transport, version.base_url,
                        version.normalized_base_url, version.model_id,
                        version.context_window_tokens AS profile_context_window_tokens,
                        version.max_input_tokens AS profile_max_input_tokens,
                        version.max_output_tokens AS profile_max_output_tokens,
                        version.history_token_budget AS profile_history_token_budget,
                        version.history_turns AS profile_history_turns,
                        version.temperature AS profile_temperature,
                        version.timeout_ms AS profile_timeout_ms
                 FROM users root
                 JOIN ai_provider_settings settings
                   ON settings.user_id = root.id
                  AND settings.enabled = TRUE
                  AND settings.secret_key_version IS NULL
                  AND settings.secret_nonce IS NULL
                  AND settings.secret_ciphertext IS NULL
                  AND settings.supports_vision = FALSE
                  AND settings.provider_config->>'kind' = 'local_http'
                 JOIN ai_user_model_defaults defaults
                   ON defaults.user_id = root.id
                  AND defaults.deleted_at IS NULL
                 JOIN ai_model_profiles profile
                   ON profile.id = defaults.default_conversation_profile_id
                  AND profile.user_id = root.id
                  AND profile.lab_id = root.lab_id
                  AND profile.archived_at IS NULL
                  AND profile.deleted_at IS NULL
                 JOIN ai_model_profile_versions version
                   ON version.profile_id = profile.id
                  AND version.version = profile.current_version
                  AND version.transport = 'local_http'
                  AND version.supports_vision = FALSE
                 JOIN ai_provider_endpoints endpoint
                   ON endpoint.lab_id = root.lab_id
                  AND endpoint.protocol = version.protocol
                  AND endpoint.normalized_base_url = version.normalized_base_url
                  AND endpoint.enabled = TRUE
                 WHERE root.id = $1
                   AND root.lab_id = $2
                   AND root.status = 'active'
                   AND root.deleted_at IS NULL
                   AND settings.provider_config->>'model' = version.model_id
                   AND rtrim(settings.provider_config->>'base_url', '/') =
                       version.normalized_base_url
                   AND NOT EXISTS (
                       SELECT 1
                       FROM ai_model_profile_secrets secret
                       WHERE secret.profile_id = profile.id
                         AND secret.profile_version = version.version
                   )
                 LIMIT 1",
            )
            .bind(environment_root_user_id)
            .bind(lab_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let Some(source) = source else {
                return Ok(false);
            };

            let profile_id = Uuid::new_v4();
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO ai_provider_settings (
                    user_id, enabled, provider_config, provider_preset_id,
                    secret_key_version, secret_nonce, secret_ciphertext,
                    supports_vision, vision_model,
                    context_window_tokens, max_input_tokens, max_output_tokens,
                    history_token_budget, history_turns, temperature, timeout_ms,
                    created_at, updated_at, revision
                 ) VALUES (
                    $1, TRUE, $2, $3, NULL, NULL, NULL, FALSE, NULL,
                    $4, $5, $6, $7, $8, $9, $10, $11, $11, 1
                 )",
            )
            .bind(target_user_id)
            .bind(
                source
                    .try_get::<Value, _>("provider_config")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<String, _>("provider_preset_id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("context_window_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("max_input_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("max_output_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("history_token_budget")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i32, _>("history_turns")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<f64, _>("temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("timeout_ms")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(now)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;

            sqlx::query(
                "INSERT INTO ai_model_profiles (
                    id, lab_id, user_id, name, current_version,
                    created_at, updated_at, archived_at, deleted_at, revision
                 ) VALUES (
                    $1, $2, $3, 'Managed local default', 1,
                    $4, $4, NULL, NULL, 1
                 )",
            )
            .bind(profile_id)
            .bind(lab_id)
            .bind(target_user_id)
            .bind(now)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            sqlx::query(
                "INSERT INTO ai_model_profile_versions (
                    profile_id, version, protocol, transport, base_url,
                    normalized_base_url, model_id, supports_vision,
                    context_window_tokens, max_input_tokens, max_output_tokens,
                    history_token_budget, history_turns, temperature, timeout_ms,
                    created_at
                 ) VALUES (
                    $1, 1, $2, $3, $4, $5, $6, FALSE,
                    $7, $8, $9, $10, $11, $12, $13, $14
                 )",
            )
            .bind(profile_id)
            .bind(
                source
                    .try_get::<String, _>("protocol")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<String, _>("transport")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<String, _>("base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<String, _>("normalized_base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<String, _>("model_id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("profile_context_window_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("profile_max_input_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("profile_max_output_tokens")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("profile_history_token_budget")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i32, _>("profile_history_turns")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<f64, _>("profile_temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(
                source
                    .try_get::<i64, _>("profile_timeout_ms")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            )
            .bind(now)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            sqlx::query(
                "INSERT INTO ai_user_model_defaults (
                    user_id, default_conversation_profile_id,
                    default_vision_profile_id, created_at, updated_at,
                    deleted_at, revision
                 ) VALUES ($1, $2, NULL, $3, $3, NULL, 1)",
            )
            .bind(target_user_id)
            .bind(profile_id)
            .bind(now)
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;

            for (entity_type, entity_id, after) in [
                (
                    "ai_provider_settings",
                    target_user_id,
                    json!({
                        "configured": true,
                        "enabled": true,
                        "credential_present": false,
                        "managed_local_default": true,
                        "revision": 1,
                    }),
                ),
                (
                    "ai_model_profile",
                    profile_id,
                    json!({
                        "name": "Managed local default",
                        "current_version": 1,
                        "transport": "local_http",
                        "base_url_present": true,
                        "model_id_present": true,
                        "supports_vision": false,
                        "credential_present": false,
                        "managed_local_default": true,
                        "revision": 1,
                    }),
                ),
                (
                    "ai_user_model_defaults",
                    target_user_id,
                    json!({
                        "default_conversation_profile_id": profile_id,
                        "default_vision_profile_id": null,
                        "managed_local_default": true,
                        "revision": 1,
                    }),
                ),
            ] {
                sqlx::query(
                    "INSERT INTO audit_entries (
                        id, lab_id, project_id, entity_type, entity_id, action,
                        actor_type, actor_user_id, actor_display_name, source,
                        request_id, reason, before_json, after_json, occurred_at,
                        operation_code, operation_version, operation_params_json
                     ) VALUES (
                        $1,$2,NULL,$3,$4,'create','human',$5,$6,$7,$8,$9,
                        NULL,$10,$11,'ai.managed_local_default.inherited',1,$12
                     )",
                )
                .bind(Uuid::new_v4())
                .bind(lab_id)
                .bind(entity_type)
                .bind(entity_id)
                .bind(actor_user_id)
                .bind(&audit.actor.display_name)
                .bind(write_source_name(audit.source))
                .bind(&audit.request_id)
                .bind(
                    audit
                        .reason
                        .as_deref()
                        .unwrap_or("managed local AI default inherited"),
                )
                .bind(after)
                .bind(now)
                .bind(json!({
                    "credential_material": "absent",
                    "source_profile_owner": "environment_root",
                }))
                .execute(&mut **transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            }
            Ok(true)
        }

        fn view(
            row: &sqlx::postgres::PgRow,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            let config = decode_config(row)?;
            Ok(AiProviderSettingsView {
                enabled: row
                    .try_get("enabled")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                provider_kind: config.kind,
                provider_preset_id: row
                    .try_get("provider_preset_id")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                model: config.model,
                base_url: config.base_url,
                has_key: row
                    .try_get::<Option<Vec<u8>>, _>("secret_ciphertext")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .is_some(),
                supports_vision: row
                    .try_get("supports_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                vision_model: row
                    .try_get("vision_model")
                    .map_err(|_| AiProviderStoreError::Storage)?,
                context_window_tokens: numeric_u32(row, "context_window_tokens")?,
                max_input_tokens: numeric_u32(row, "max_input_tokens")?,
                max_output_tokens: numeric_u32(row, "max_output_tokens")?,
                history_token_budget: numeric_u32(row, "history_token_budget")?,
                history_turns: numeric_u32(row, "history_turns")?,
                temperature: row
                    .try_get::<f64, _>("temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    as f32,
                timeout_ms: numeric_u64(row, "timeout_ms")?,
                revision: row
                    .try_get("revision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            })
        }

        fn profile_provider_config(
            protocol: AiProviderProtocol,
            transport: AiProviderTransport,
            model: String,
            base_url: String,
            timeout_ms: u64,
        ) -> Result<ProviderConfig, AiProviderStoreError> {
            let mut config = match transport {
                AiProviderTransport::OpenAiCompatible => {
                    ProviderConfig::openai_compatible(SERVER_PROVIDER_ID, model, base_url)
                }
                AiProviderTransport::LocalHttp => {
                    ProviderConfig::local_http(SERVER_PROVIDER_ID, model, base_url)
                }
            }
            .with_protocol(protocol);
            config.timeout_ms = timeout_ms;
            Ok(config)
        }

        async fn resolve_profile(
            &self,
            user_id: Uuid,
            binding: AiModelProfileBinding,
            requires_vision: bool,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            if binding.profile_id.is_nil() || binding.profile_version <= 0 {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let lab_id = self.active_user_lab_from_pool(user_id).await?;
            if !self.lab_enabled(lab_id).await? {
                return Err(AiProviderStoreError::LabDisabled);
            }
            let row = sqlx::query(
                "SELECT v.protocol, v.transport, v.base_url,
                        v.normalized_base_url, v.model_id, v.supports_vision,
                        v.context_window_tokens, v.max_input_tokens,
                        v.max_output_tokens, v.history_token_budget,
                        v.history_turns, v.temperature, v.timeout_ms,
                        s.key_version, s.nonce, s.ciphertext
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = $2
                 LEFT JOIN ai_model_profile_secrets s
                   ON s.profile_id = p.id AND s.profile_version = v.version
                 WHERE p.id = $1
                   AND p.user_id = $3
                   AND p.lab_id = $4
                   AND p.archived_at IS NULL
                   AND p.deleted_at IS NULL",
            )
            .bind(binding.profile_id)
            .bind(binding.profile_version)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ProviderNotSelected)?;
            let protocol = Self::protocol_from_db(
                row.try_get::<String, _>("protocol")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            let transport = Self::transport_from_db(
                row.try_get::<String, _>("transport")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            let supports_vision: bool = row
                .try_get("supports_vision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            if requires_vision && !supports_vision {
                return Err(AiProviderStoreError::Disabled);
            }
            let runtime = runtime_from_row(&row)?;
            let base_url: String = row
                .try_get("base_url")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let normalized_base_url: String = row
                .try_get("normalized_base_url")
                .map_err(|_| AiProviderStoreError::Storage)?;
            if normalized_base_url != normalized_url(&base_url) {
                return Err(AiProviderStoreError::Storage);
            }
            let model: String = row
                .try_get("model_id")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let config = Self::profile_provider_config(
                protocol,
                transport,
                model,
                base_url,
                runtime.timeout_ms,
            )?;
            self.validate_endpoint_for_lab(lab_id, protocol, &config)
                .await?;
            let provider = BuiltinProvider::from_config(config.clone())
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            let key_version: Option<i32> = row
                .try_get("key_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let nonce: Option<Vec<u8>> = row
                .try_get("nonce")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let ciphertext: Option<Vec<u8>> = row
                .try_get("ciphertext")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let api_key = match (key_version, nonce, ciphertext) {
                (Some(version), Some(nonce), Some(ciphertext)) => {
                    Some(self.decrypt_profile_secret(
                        user_id,
                        binding.profile_id,
                        binding.profile_version,
                        version,
                        &nonce,
                        &ciphertext,
                    )?)
                }
                (None, None, None) => None,
                _ => return Err(AiProviderStoreError::Encryption),
            };
            if config.kind == ProviderKind::OpenAiCompatible && api_key.is_none() {
                return Err(AiProviderStoreError::MissingCredential);
            }
            Ok(ResolvedAiProvider {
                provider,
                api_key,
                runtime,
                model_profile: binding,
                supports_vision,
            })
        }
    }
    #[async_trait]
    impl UserAiProviderStore for PostgresAiProviderStore {
        async fn get(&self, user_id: Uuid) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            match self.row(user_id).await? {
                Some(row) => Self::view(&row),
                None => Ok(AiProviderSettingsView::unconfigured()),
            }
        }

        async fn save(
            &self,
            user_id: Uuid,
            input: SaveAiProviderSettingsInput,
            audit: &AuditContext,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            let config = Self::config(&input)?;
            let runtime = input.runtime()?;
            let provider_preset_id = input.provider_preset_id.trim().to_owned();
            validate_preset_id(&provider_preset_id)?;
            let vision_model = if input.supports_vision {
                Some(
                    input
                        .vision_model
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or(AiProviderStoreError::InvalidSettings)?
                        .to_owned(),
                )
            } else {
                None
            };
            if let Some(model) = vision_model.as_ref() {
                let mut vision_config = config.clone();
                vision_config.model = model.clone();
                BuiltinProvider::from_config(vision_config)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            }
            let config_json =
                serde_json::to_value(&config).map_err(|_| AiProviderStoreError::Storage)?;
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            self.validate_endpoint_for_lab(
                lab_id,
                AiProviderProtocol::OpenaiChatCompletions,
                &config,
            )
            .await?;
            let current = Self::locked_row(&mut transaction, user_id).await?;
            let before = settings_audit_state(current.as_ref())?;
            let identity_matches = current
                .as_ref()
                .map(|row| credential_identity_matches(row, &config))
                .transpose()?
                .unwrap_or(false);
            let credential_action = match (input.api_key.is_some(), identity_matches) {
                (true, _) => "replace",
                (false, true) => "preserve",
                (false, false) => "clear_provider_change",
            };
            let (key_version, nonce, ciphertext) = match input.api_key.as_deref() {
                Some(secret) => {
                    let (nonce, ciphertext) = self.encrypt(user_id, secret)?;
                    (
                        Some(self.master_key.version),
                        Some(nonce.to_vec()),
                        Some(ciphertext),
                    )
                }
                None if identity_matches => match current.as_ref() {
                    Some(row) => (
                        row.try_get("secret_key_version")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                        row.try_get("secret_nonce")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                        row.try_get("secret_ciphertext")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                    ),
                    None => (None, None, None),
                },
                None => (None, None, None),
            };
            Self::materialize_and_append_default_profile_versions(
                &mut transaction,
                user_id,
                lab_id,
                &config,
                runtime,
                vision_model.as_deref(),
                audit,
            )
            .await?;
            match input.api_key.as_deref() {
                Some(secret) => {
                    self.replace_default_profile_secrets(
                        &mut transaction,
                        user_id,
                        lab_id,
                        secret,
                        audit,
                    )
                    .await?;
                }
                None if identity_matches => {
                    let legacy_secret = match (key_version, nonce.as_deref(), ciphertext.as_deref())
                    {
                        (Some(version), Some(nonce), Some(ciphertext)) => {
                            Some(self.decrypt(user_id, version, nonce, ciphertext)?)
                        }
                        (None, None, None) => None,
                        _ => return Err(AiProviderStoreError::Encryption),
                    };
                    self.preserve_default_profile_secrets(
                        &mut transaction,
                        user_id,
                        lab_id,
                        legacy_secret.as_ref(),
                        audit,
                    )
                    .await?;
                }
                None => {
                    Self::clear_current_default_profile_secrets(
                        &mut transaction,
                        user_id,
                        lab_id,
                        audit,
                        "AI model profile credential omitted after provider address change",
                    )
                    .await?;
                }
            }
            let row = sqlx::query("INSERT INTO ai_provider_settings (user_id, enabled, provider_config, provider_preset_id, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,now(),now(),1) ON CONFLICT (user_id) DO UPDATE SET enabled = EXCLUDED.enabled, provider_config = EXCLUDED.provider_config, provider_preset_id = EXCLUDED.provider_preset_id, secret_key_version = EXCLUDED.secret_key_version, secret_nonce = EXCLUDED.secret_nonce, secret_ciphertext = EXCLUDED.secret_ciphertext, supports_vision = EXCLUDED.supports_vision, vision_model = EXCLUDED.vision_model, context_window_tokens = EXCLUDED.context_window_tokens, max_input_tokens = EXCLUDED.max_input_tokens, max_output_tokens = EXCLUDED.max_output_tokens, history_token_budget = EXCLUDED.history_token_budget, history_turns = EXCLUDED.history_turns, temperature = EXCLUDED.temperature, timeout_ms = EXCLUDED.timeout_ms, updated_at = now(), revision = ai_provider_settings.revision + 1 RETURNING enabled, provider_config, provider_preset_id, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, revision")
                .bind(user_id)
                .bind(input.enabled)
                .bind(config_json)
                .bind(provider_preset_id)
                .bind(key_version)
                .bind(nonce)
                .bind(ciphertext)
                .bind(input.supports_vision)
                .bind(vision_model)
                .bind(i64::from(runtime.context_window_tokens))
                .bind(i64::from(runtime.max_input_tokens))
                .bind(i64::from(runtime.max_output_tokens))
                .bind(i64::from(runtime.history_token_budget))
                .bind(i32::try_from(runtime.history_turns).map_err(|_| AiProviderStoreError::InvalidSettings)?)
                .bind(f64::from(runtime.temperature))
                .bind(i64::try_from(runtime.timeout_ms).map_err(|_| AiProviderStoreError::InvalidSettings)?)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let view = Self::view(&row)?;
            let after = tagged_settings_audit_state(
                settings_audit_state(Some(&row))?,
                "save",
                credential_action,
            );
            write_settings_audit(
                &mut transaction,
                lab_id,
                user_id,
                if current.is_some() {
                    "update"
                } else {
                    "create"
                },
                before,
                after,
                audit,
                "AI provider settings saved",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(view)
        }

        async fn clear_key(
            &self,
            user_id: Uuid,
            audit: &AuditContext,
        ) -> Result<AiProviderSettingsView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            let current = Self::locked_row(&mut transaction, user_id).await?;
            let before = settings_audit_state(current.as_ref())?;
            let (view, after) = if current.is_some() {
                let row = sqlx::query("UPDATE ai_provider_settings SET secret_key_version = NULL, secret_nonce = NULL, secret_ciphertext = NULL, updated_at = now(), revision = revision + 1 WHERE user_id = $1 RETURNING enabled, provider_config, provider_preset_id, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, revision")
                    .bind(user_id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                (Self::view(&row)?, settings_audit_state(Some(&row))?)
            } else {
                (
                    AiProviderSettingsView::unconfigured(),
                    settings_audit_state(None)?,
                )
            };
            Self::clear_all_compatibility_profile_secrets(&mut transaction, user_id, lab_id, audit)
                .await?;
            write_settings_audit(
                &mut transaction,
                lab_id,
                user_id,
                "update",
                before,
                tagged_settings_audit_state(after, "clear", "clear"),
                audit,
                "AI provider credential cleared",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(view)
        }

        async fn resolve(&self, user_id: Uuid) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            let binding = self.selected_profile_binding(user_id, false).await?;
            self.resolve_profile(user_id, binding, false).await
        }

        async fn resolve_for_profile(
            &self,
            user_id: Uuid,
            binding: AiModelProfileBinding,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            self.resolve_profile(user_id, binding, false).await
        }

        async fn resolve_vision(
            &self,
            user_id: Uuid,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            let binding = self.selected_profile_binding(user_id, true).await?;
            self.resolve_profile(user_id, binding, true).await
        }

        async fn diagnostics(
            &self,
            user_id: Uuid,
            lab_id: Uuid,
        ) -> Result<AiProviderDiagnosticsView, AiProviderStoreError> {
            let actual_lab: Option<Uuid> = sqlx::query_scalar(
                "SELECT lab_id FROM users WHERE id = $1 AND status = 'active' AND deleted_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if actual_lab != Some(lab_id) {
                return Err(AiProviderStoreError::Storage);
            }
            let row = self.row(user_id).await?;
            let lab_enabled = self.lab_enabled(lab_id).await?;
            let user_enabled = row
                .as_ref()
                .and_then(|value| value.try_get("enabled").ok())
                .unwrap_or(true);
            let credential_configured = row
                .as_ref()
                .and_then(|value| {
                    value
                        .try_get::<Option<Vec<u8>>, _>("secret_ciphertext")
                        .ok()
                })
                .flatten()
                .is_some();
            let status = if !lab_enabled {
                "lab_disabled"
            } else if !user_enabled {
                "user_disabled"
            } else if !credential_configured {
                "waiting_for_personal_api_key"
            } else {
                "ready"
            };
            let (local_endpoint_count, cloud_endpoint_count) = self.endpoint_counts(lab_id).await?;
            Ok(AiProviderDiagnosticsView {
                runtime_configured: true,
                lab_enabled,
                user_enabled,
                provider_presets_available: true,
                status: status.to_owned(),
                provider_configured: row.is_some(),
                provider_enabled: user_enabled,
                credential_configured,
                supports_vision: row
                    .as_ref()
                    .and_then(|value| value.try_get("supports_vision").ok())
                    .unwrap_or(false),
                text_model_configured: true,
                vision_model_configured: row
                    .as_ref()
                    .and_then(|value| value.try_get::<Option<String>, _>("vision_model").ok())
                    .flatten()
                    .is_some(),
                local_endpoint_count,
                cloud_endpoint_count,
            })
        }

        async fn get_lab_settings(
            &self,
            lab_id: Uuid,
        ) -> Result<AiLabSettingsView, AiProviderStoreError> {
            let settings: Option<(bool, bool, String, i64)> = sqlx::query_as(
                "SELECT enabled, custom_url_approval_required, max_autonomy_mode, revision FROM ai_lab_settings WHERE lab_id = $1",
            )
            .bind(lab_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let counts: (i64, i64, i64) = sqlx::query_as(
                "SELECT count(*)::bigint, count(*) FILTER (WHERE enabled)::bigint, count(*) FILTER (WHERE enabled AND supports_vision)::bigint FROM ai_provider_settings s JOIN users u ON u.id = s.user_id WHERE u.lab_id = $1 AND u.deleted_at IS NULL",
            )
            .bind(lab_id)
            .fetch_one(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let (enabled, custom_url_approval_required, max_autonomy_mode, revision) =
                settings.unwrap_or((true, true, "full".to_owned(), 0));
            Ok(AiLabSettingsView {
                enabled,
                custom_url_approval_required,
                configured_user_count: counts.0,
                enabled_user_count: counts.1,
                vision_user_count: counts.2,
                revision,
                max_autonomy_mode: parse_autonomy_mode(&max_autonomy_mode)?,
            })
        }

        async fn save_lab_settings(
            &self,
            lab_id: Uuid,
            input: SaveAiLabSettingsInput,
            audit: &AuditContext,
        ) -> Result<AiLabSettingsView, AiProviderStoreError> {
            let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
            if audit.actor.actor_type != ActorType::Human {
                return Err(AiProviderStoreError::Storage);
            }
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            if Self::active_user_lab(&mut transaction, actor_user_id).await? != lab_id {
                return Err(AiProviderStoreError::Storage);
            }
            let before: Option<Value> = sqlx::query(
                "SELECT enabled, custom_url_approval_required, max_autonomy_mode, revision FROM ai_lab_settings WHERE lab_id = $1 FOR UPDATE",
            )
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .map(|row| {
                json!({
                    "enabled": row.try_get::<bool, _>("enabled").unwrap_or(true),
                    "custom_url_approval_required": row.try_get::<bool, _>("custom_url_approval_required").unwrap_or(true),
                    "max_autonomy_mode": row.try_get::<String, _>("max_autonomy_mode").unwrap_or_else(|_| "full".to_owned()),
                    "revision": row.try_get::<i64, _>("revision").unwrap_or(0),
                })
            });
            let row = sqlx::query(
                "INSERT INTO ai_lab_settings (lab_id, enabled, custom_url_approval_required, max_autonomy_mode, updated_by, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,$5,now(),now(),1) ON CONFLICT (lab_id) DO UPDATE SET enabled = EXCLUDED.enabled, custom_url_approval_required = EXCLUDED.custom_url_approval_required, max_autonomy_mode = EXCLUDED.max_autonomy_mode, updated_by = EXCLUDED.updated_by, updated_at = now(), revision = ai_lab_settings.revision + 1 RETURNING enabled, custom_url_approval_required, max_autonomy_mode, revision",
            )
            .bind(lab_id)
            .bind(input.enabled)
            .bind(input.custom_url_approval_required)
            .bind(autonomy_mode_name(input.max_autonomy_mode))
            .bind(actor_user_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let after = json!({
                "enabled": row.try_get::<bool, _>("enabled").map_err(|_| AiProviderStoreError::Storage)?,
                "custom_url_approval_required": row.try_get::<bool, _>("custom_url_approval_required").map_err(|_| AiProviderStoreError::Storage)?,
                "max_autonomy_mode": row.try_get::<String, _>("max_autonomy_mode").map_err(|_| AiProviderStoreError::Storage)?,
                "revision": row.try_get::<i64, _>("revision").map_err(|_| AiProviderStoreError::Storage)?,
            });
            sqlx::query(
                "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1,$2,NULL,'ai_lab_settings',$2,'update','human',$3,$4,$5,$6,$7,$8,$9,now())",
            )
            .bind(Uuid::new_v4())
            .bind(lab_id)
            .bind(actor_user_id)
            .bind(&audit.actor.display_name)
            .bind(write_source_name(audit.source))
            .bind(&audit.request_id)
            .bind("AI laboratory policy updated")
            .bind(before)
            .bind(after)
            .execute(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            self.get_lab_settings(lab_id).await
        }

        async fn list_provider_presets(
            &self,
            lab_id: Uuid,
        ) -> Result<Vec<AiProviderPresetView>, AiProviderStoreError> {
            let rows = sqlx::query(
                "SELECT id, provider_kind, protocol, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE lab_id = $1 ORDER BY enabled DESC, protocol, label, base_url",
            )
            .bind(lab_id)
            .fetch_all(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let mut presets = Self::builtin_provider_presets();
            for row in &rows {
                let endpoint = Self::endpoint_view(row)?;
                presets.push(AiProviderPresetView {
                    id: format!("endpoint:{}", endpoint.id),
                    display_name: endpoint.label,
                    provider_kind: endpoint.provider_kind,
                    recommended_base_url: endpoint.base_url,
                    models: Vec::new(),
                    supports_vision: false,
                    documentation_url: String::new(),
                    builtin: false,
                    enabled: endpoint.enabled,
                    default_preset: false,
                });
            }
            Ok(presets)
        }

        async fn list_provider_endpoints(
            &self,
            lab_id: Uuid,
        ) -> Result<Vec<AiProviderEndpointView>, AiProviderStoreError> {
            let rows = sqlx::query(
                "SELECT id, provider_kind, protocol, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE lab_id = $1 ORDER BY enabled DESC, protocol, label, base_url",
            )
            .bind(lab_id)
            .fetch_all(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let mut endpoints = Self::builtin_endpoints();
            endpoints.extend(
                rows.iter()
                    .map(Self::endpoint_view)
                    .collect::<Result<Vec<_>, _>>()?,
            );
            Ok(endpoints)
        }

        async fn save_provider_endpoint(
            &self,
            lab_id: Uuid,
            endpoint_id: Option<Uuid>,
            input: SaveAiProviderEndpointInput,
            audit: &AuditContext,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
            if audit.actor.actor_type != ActorType::Human {
                return Err(AiProviderStoreError::Storage);
            }
            if endpoint_id.is_some_and(Self::builtin_endpoint) {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let label = Self::validated_endpoint_label(&input.label)?;
            let config = Self::endpoint_config(&input)?;
            let normalized = normalized_url(&config.base_url);
            if input.protocol == AiProviderProtocol::OpenaiChatCompletions
                && config.kind == ProviderKind::OpenAiCompatible
                && [
                    OFFICIAL_DEEPSEEK_BASE_URL,
                    OFFICIAL_GLM_BASE_URL,
                    OFFICIAL_KIMI_BASE_URL,
                    OFFICIAL_OPENAI_BASE_URL,
                ]
                .into_iter()
                .any(|value| normalized == normalized_url(value))
            {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            if Self::active_user_lab(&mut transaction, actor_user_id).await? != lab_id {
                return Err(AiProviderStoreError::Storage);
            }
            let kind = Self::provider_kind_name(config.kind);
            let protocol = Self::protocol_name(input.protocol);
            let before_row = match endpoint_id {
                Some(id) => sqlx::query(
                    "SELECT id, provider_kind, protocol, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE id = $1 AND lab_id = $2 AND builtin = FALSE FOR UPDATE",
                )
                .bind(id)
                .bind(lab_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?,
                None => sqlx::query(
                    "SELECT id, provider_kind, protocol, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE lab_id = $1 AND protocol = $2 AND normalized_base_url = $3 FOR UPDATE",
                )
                .bind(lab_id)
                .bind(protocol)
                .bind(&normalized)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?,
            };
            if endpoint_id.is_some() && before_row.is_none() {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let before = before_row
                .as_ref()
                .map(Self::endpoint_view)
                .transpose()?
                .map(|view| Self::endpoint_audit_state(&view));
            let id = endpoint_id
                .or_else(|| {
                    before_row
                        .as_ref()
                        .and_then(|row| row.try_get::<Uuid, _>("id").ok())
                })
                .unwrap_or_else(Uuid::new_v4);
            let row = if endpoint_id.is_some() {
                sqlx::query(
                    "UPDATE ai_provider_endpoints SET provider_kind = $1, protocol = $2, label = $3, base_url = $4, normalized_base_url = $5, enabled = $6, updated_by = $7, updated_at = now(), revision = revision + 1 WHERE id = $8 AND lab_id = $9 AND builtin = FALSE RETURNING id, provider_kind, protocol, label, base_url, enabled, builtin, revision",
                )
                .bind(kind)
                .bind(protocol)
                .bind(&label)
                .bind(&config.base_url)
                .bind(&normalized)
                .bind(input.enabled)
                .bind(actor_user_id)
                .bind(id)
                .bind(lab_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?
            } else {
                sqlx::query(
                    "INSERT INTO ai_provider_endpoints (id, lab_id, provider_kind, protocol, label, base_url, normalized_base_url, enabled, builtin, created_by, updated_by, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,FALSE,$9,$9,now(),now(),1) ON CONFLICT (lab_id, protocol, normalized_base_url) DO UPDATE SET provider_kind = EXCLUDED.provider_kind, label = EXCLUDED.label, base_url = EXCLUDED.base_url, enabled = EXCLUDED.enabled, updated_by = EXCLUDED.updated_by, updated_at = now(), revision = ai_provider_endpoints.revision + 1 RETURNING id, provider_kind, protocol, label, base_url, enabled, builtin, revision",
                )
                .bind(id)
                .bind(lab_id)
                .bind(kind)
                .bind(protocol)
                .bind(&label)
                .bind(&config.base_url)
                .bind(&normalized)
                .bind(input.enabled)
                .bind(actor_user_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?
            };
            let view = Self::endpoint_view(&row)?;
            let after = Self::endpoint_audit_state(&view);
            write_endpoint_audit(
                &mut transaction,
                lab_id,
                audit,
                EndpointAuditChange {
                    endpoint_id: view.id,
                    action: if before.is_some() { "update" } else { "create" },
                    before,
                    after: Some(after),
                    reason: "AI provider endpoint saved",
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(view)
        }

        async fn disable_provider_endpoint(
            &self,
            lab_id: Uuid,
            endpoint_id: Uuid,
            audit: &AuditContext,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
            if audit.actor.actor_type != ActorType::Human || Self::builtin_endpoint(endpoint_id) {
                return Err(AiProviderStoreError::Storage);
            }
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            if Self::active_user_lab(&mut transaction, actor_user_id).await? != lab_id {
                return Err(AiProviderStoreError::Storage);
            }
            let current = sqlx::query(
                "SELECT id, provider_kind, protocol, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE id = $1 AND lab_id = $2 AND builtin = FALSE FOR UPDATE",
            )
            .bind(endpoint_id)
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::InvalidSettings)?;
            let before_view = Self::endpoint_view(&current)?;
            let row = sqlx::query(
                "UPDATE ai_provider_endpoints SET enabled = FALSE, updated_by = $1, updated_at = now(), revision = revision + 1 WHERE id = $2 AND lab_id = $3 AND builtin = FALSE RETURNING id, provider_kind, protocol, label, base_url, enabled, builtin, revision",
            )
            .bind(actor_user_id)
            .bind(endpoint_id)
            .bind(lab_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let view = Self::endpoint_view(&row)?;
            write_endpoint_audit(
                &mut transaction,
                lab_id,
                audit,
                EndpointAuditChange {
                    endpoint_id: view.id,
                    action: "update",
                    before: Some(Self::endpoint_audit_state(&before_view)),
                    after: Some(Self::endpoint_audit_state(&view)),
                    reason: "AI provider endpoint disabled",
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(view)
        }

        async fn list_model_profiles(
            &self,
            user_id: Uuid,
            include_archived: bool,
        ) -> Result<Vec<AiModelProfileView>, AiProviderStoreError> {
            self.active_user_lab_from_pool(user_id).await?;
            self.model_profile_views(user_id, include_archived).await
        }

        async fn get_model_profile(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            self.active_user_lab_from_pool(user_id).await?;
            self.model_profile_view_by_id(user_id, profile_id).await
        }

        async fn create_model_profile(
            &self,
            user_id: Uuid,
            input: SaveAiModelProfileInput,
            audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            if input.expected_revision.is_some() {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let runtime = input.runtime()?;
            let (name, config) = Self::validated_model_configuration(
                &input.name,
                input.protocol,
                input.transport,
                &input.base_url,
                &input.model_id,
                input.supports_vision,
                runtime,
            )?;
            let api_key = input.trimmed_key();
            if input.transport == AiProviderTransport::OpenAiCompatible && api_key.is_none() {
                return Err(AiProviderStoreError::CredentialRequired);
            }
            if let Some(secret) = api_key {
                ProviderCredentials::bearer(secret)
                    .map_err(|_| AiProviderStoreError::InvalidCredential)?;
            }

            let profile_id = Uuid::new_v4();
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            self.validate_endpoint_for_lab(lab_id, input.protocol, &config)
                .await?;
            sqlx::query(
                "INSERT INTO ai_model_profiles (
                    id, lab_id, user_id, name, current_version,
                    created_at, updated_at, archived_at, deleted_at, revision
                 ) VALUES ($1,$2,$3,$4,1,now(),now(),NULL,NULL,1)",
            )
            .bind(profile_id)
            .bind(lab_id)
            .bind(user_id)
            .bind(&name)
            .execute(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            Self::insert_model_version(
                &mut transaction,
                profile_id,
                1,
                input.protocol,
                input.transport,
                &config,
                input.supports_vision,
                runtime,
            )
            .await?;
            if let Some(secret) = api_key {
                self.save_model_version_secret(
                    &mut transaction,
                    user_id,
                    lab_id,
                    profile_id,
                    1,
                    secret,
                    audit,
                    "AI model profile credential created",
                )
                .await?;
            }
            write_model_profile_management_audit(
                &mut transaction,
                lab_id,
                profile_id,
                "create",
                audit,
                None,
                model_profile_audit_state(
                    &name,
                    1,
                    input.protocol,
                    input.transport,
                    input.supports_vision,
                    api_key.is_some(),
                    false,
                    1,
                ),
                "AI model profile created",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            self.model_profile_view_by_id(user_id, profile_id).await
        }

        async fn update_model_profile(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            input: SaveAiModelProfileInput,
            audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            let expected_revision = input
                .expected_revision
                .filter(|revision| *revision > 0)
                .ok_or(AiProviderStoreError::InvalidSettings)?;
            let runtime = input.runtime()?;
            let (name, config) = Self::validated_model_configuration(
                &input.name,
                input.protocol,
                input.transport,
                &input.base_url,
                &input.model_id,
                input.supports_vision,
                runtime,
            )?;
            if let Some(secret) = input.trimmed_key() {
                ProviderCredentials::bearer(secret)
                    .map_err(|_| AiProviderStoreError::InvalidCredential)?;
            }

            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            self.validate_endpoint_for_lab(lab_id, input.protocol, &config)
                .await?;
            let current = sqlx::query(
                "SELECT p.name, p.current_version, p.archived_at, p.revision,
                        v.protocol, v.transport, v.base_url,
                        v.normalized_base_url, v.model_id, v.supports_vision,
                        v.context_window_tokens, v.max_input_tokens,
                        v.max_output_tokens, v.history_token_budget,
                        v.history_turns, v.temperature, v.timeout_ms,
                        s.key_version, s.nonce, s.ciphertext
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 LEFT JOIN ai_model_profile_secrets s
                   ON s.profile_id = p.id AND s.profile_version = v.version
                 WHERE p.id = $1 AND p.user_id = $2 AND p.lab_id = $3
                   AND p.deleted_at IS NULL
                 FOR UPDATE OF p",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ModelProfileNotFound)?;
            if current
                .try_get::<Option<DateTime<Utc>>, _>("archived_at")
                .map_err(|_| AiProviderStoreError::Storage)?
                .is_some()
            {
                return Err(AiProviderStoreError::ModelProfileNotFound);
            }
            let current_revision: i64 = current
                .try_get("revision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            if current_revision != expected_revision {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            if !input.supports_vision {
                let default_vision_profile_id: Option<Option<Uuid>> = sqlx::query_scalar(
                    "SELECT default_vision_profile_id
                     FROM ai_user_model_defaults
                     WHERE user_id = $1 AND deleted_at IS NULL
                     FOR SHARE",
                )
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
                if default_vision_profile_id.flatten() == Some(profile_id) {
                    return Err(AiProviderStoreError::InvalidSettings);
                }
            }
            let current_version: i64 = current
                .try_get("current_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_name: String = current
                .try_get("name")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_protocol: String = current
                .try_get("protocol")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_transport: String = current
                .try_get("transport")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_normalized_base_url: String = current
                .try_get("normalized_base_url")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_supports_vision: bool = current
                .try_get("supports_vision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let identity_matches = current_protocol == Self::protocol_name(input.protocol)
                && current_transport == Self::transport_name(input.transport)
                && current_normalized_base_url == normalized_url(&config.base_url);
            if !identity_matches && input.trimmed_key().is_none() {
                return Err(AiProviderStoreError::CredentialRequired);
            }

            let key_version: Option<i32> = current
                .try_get("key_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let nonce: Option<Vec<u8>> = current
                .try_get("nonce")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let ciphertext: Option<Vec<u8>> = current
                .try_get("ciphertext")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let configuration_unchanged = current_name == name
                && identity_matches
                && current
                    .try_get::<String, _>("base_url")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == config.base_url
                && current
                    .try_get::<String, _>("model_id")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == config.model
                && current_supports_vision == input.supports_vision
                && numeric_u32(&current, "context_window_tokens")? == runtime.context_window_tokens
                && numeric_u32(&current, "max_input_tokens")? == runtime.max_input_tokens
                && numeric_u32(&current, "max_output_tokens")? == runtime.max_output_tokens
                && numeric_u32(&current, "history_token_budget")? == runtime.history_token_budget
                && numeric_u32(&current, "history_turns")? == runtime.history_turns
                && current
                    .try_get::<f64, _>("temperature")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    == f64::from(runtime.temperature)
                && numeric_u64(&current, "timeout_ms")? == runtime.timeout_ms;
            if configuration_unchanged {
                if let Some(secret) = input.trimmed_key() {
                    let existed = match (&key_version, &nonce, &ciphertext) {
                        (Some(_), Some(_), Some(_)) => true,
                        (None, None, None) => false,
                        _ => return Err(AiProviderStoreError::Encryption),
                    };
                    let (next_key_version, next_nonce, next_ciphertext) =
                        self.encrypt_profile_secret(user_id, profile_id, current_version, secret)?;
                    sqlx::query(
                        "INSERT INTO ai_model_profile_secrets (
                            profile_id, profile_version, key_version, nonce,
                            ciphertext, created_at, updated_at
                         ) VALUES ($1,$2,$3,$4,$5,now(),now())
                         ON CONFLICT (profile_id, profile_version) DO UPDATE
                         SET key_version = EXCLUDED.key_version,
                             nonce = EXCLUDED.nonce,
                             ciphertext = EXCLUDED.ciphertext,
                             updated_at = now()",
                    )
                    .bind(profile_id)
                    .bind(current_version)
                    .bind(next_key_version)
                    .bind(next_nonce.to_vec())
                    .bind(next_ciphertext)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                    write_human_profile_secret_audit(
                        &mut transaction,
                        lab_id,
                        profile_id,
                        current_version,
                        if existed { "update" } else { "create" },
                        audit,
                        existed,
                        true,
                        "AI model profile credential rotated without changing configuration",
                    )
                    .await?;
                }
                transaction
                    .commit()
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                return self.model_profile_view_by_id(user_id, profile_id).await;
            }
            let copied_secret = if input.trimmed_key().is_none() && identity_matches {
                match (key_version, nonce, ciphertext) {
                    (Some(key_version), Some(nonce), Some(ciphertext)) => {
                        Some(self.decrypt_profile_secret(
                            user_id,
                            profile_id,
                            current_version,
                            key_version,
                            &nonce,
                            &ciphertext,
                        )?)
                    }
                    (None, None, None) => None,
                    _ => return Err(AiProviderStoreError::Encryption),
                }
            } else {
                None
            };
            let next_version = current_version
                .checked_add(1)
                .ok_or(AiProviderStoreError::Storage)?;
            Self::insert_model_version(
                &mut transaction,
                profile_id,
                next_version,
                input.protocol,
                input.transport,
                &config,
                input.supports_vision,
                runtime,
            )
            .await?;
            let secret = input
                .trimmed_key()
                .or_else(|| copied_secret.as_ref().map(|secret| secret.as_str()));
            if let Some(secret) = secret {
                self.save_model_version_secret(
                    &mut transaction,
                    user_id,
                    lab_id,
                    profile_id,
                    next_version,
                    secret,
                    audit,
                    "AI model profile credential bound to a new immutable version",
                )
                .await?;
            }
            let updated = sqlx::query(
                "UPDATE ai_model_profiles
                 SET name = $1, current_version = $2, updated_at = now(),
                     revision = revision + 1
                 WHERE id = $3 AND user_id = $4 AND revision = $5
                   AND archived_at IS NULL AND deleted_at IS NULL",
            )
            .bind(&name)
            .bind(next_version)
            .bind(profile_id)
            .bind(user_id)
            .bind(expected_revision)
            .execute(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if updated.rows_affected() != 1 {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            let before = model_profile_audit_state(
                &current_name,
                current_version,
                Self::protocol_from_db(&current_protocol)?,
                Self::transport_from_db(&current_transport)?,
                current_supports_vision,
                key_version.is_some(),
                false,
                current_revision,
            );
            let after = model_profile_audit_state(
                &name,
                next_version,
                input.protocol,
                input.transport,
                input.supports_vision,
                secret.is_some(),
                false,
                current_revision + 1,
            );
            write_model_profile_management_audit(
                &mut transaction,
                lab_id,
                profile_id,
                "update",
                audit,
                Some(before),
                after,
                "AI model profile immutable version appended",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            self.model_profile_view_by_id(user_id, profile_id).await
        }

        async fn validate_model_profile(
            &self,
            user_id: Uuid,
            input: ValidateAiModelProfileInput,
        ) -> Result<AiModelValidationView, AiProviderStoreError> {
            let profile_binding_hint = input.profile_binding_hint()?;
            let runtime = input.runtime()?;
            let (_, config) = Self::validated_model_configuration(
                "Unsaved model validation",
                input.protocol,
                input.transport,
                &input.base_url,
                &input.model_id,
                input.supports_vision,
                runtime,
            )?;
            let lab_id = self.active_user_lab_from_pool(user_id).await?;
            self.validate_endpoint_for_lab(lab_id, input.protocol, &config)
                .await?;

            let reusable_secret = match (input.trimmed_key(), profile_binding_hint) {
                (None, Some((profile_id, profile_version))) => {
                    let row = sqlx::query(
                        "SELECT v.protocol, v.transport, v.normalized_base_url,
                                s.key_version, s.nonce, s.ciphertext
                         FROM ai_model_profiles p
                         JOIN ai_model_profile_versions v
                           ON v.profile_id = p.id
                          AND v.version = p.current_version
                          AND v.version = $3
                         LEFT JOIN ai_model_profile_secrets s
                           ON s.profile_id = p.id AND s.profile_version = v.version
                         WHERE p.id = $1 AND p.user_id = $2
                           AND p.archived_at IS NULL AND p.deleted_at IS NULL",
                    )
                    .bind(profile_id)
                    .bind(user_id)
                    .bind(profile_version)
                    .fetch_optional(self.postgres.pool())
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                    match row {
                        Some(row) => {
                            let identity_matches = row
                                .try_get::<String, _>("protocol")
                                .map_err(|_| AiProviderStoreError::Storage)?
                                == Self::protocol_name(input.protocol)
                                && row
                                    .try_get::<String, _>("transport")
                                    .map_err(|_| AiProviderStoreError::Storage)?
                                    == Self::transport_name(input.transport)
                                && row
                                    .try_get::<String, _>("normalized_base_url")
                                    .map_err(|_| AiProviderStoreError::Storage)?
                                    == normalized_url(&config.base_url);
                            let key_version: Option<i32> = row
                                .try_get("key_version")
                                .map_err(|_| AiProviderStoreError::Storage)?;
                            let nonce: Option<Vec<u8>> = row
                                .try_get("nonce")
                                .map_err(|_| AiProviderStoreError::Storage)?;
                            let ciphertext: Option<Vec<u8>> = row
                                .try_get("ciphertext")
                                .map_err(|_| AiProviderStoreError::Storage)?;
                            match (identity_matches, key_version, nonce, ciphertext) {
                                (true, Some(key_version), Some(nonce), Some(ciphertext)) => {
                                    Some(self.decrypt_profile_secret(
                                        user_id,
                                        profile_id,
                                        profile_version,
                                        key_version,
                                        &nonce,
                                        &ciphertext,
                                    )?)
                                }
                                (true, None, None, None) | (false, _, _, _) => None,
                                _ => return Err(AiProviderStoreError::Encryption),
                            }
                        }
                        None => None,
                    }
                }
                _ => None,
            };
            let secret = input
                .trimmed_key()
                .or_else(|| reusable_secret.as_ref().map(SensitiveSecret::as_str));
            if input.transport == AiProviderTransport::OpenAiCompatible && secret.is_none() {
                return Ok(AiModelValidationView {
                    ok: false,
                    latency_ms: 0,
                    error_code: Some("missing_credential"),
                });
            }
            let credentials = match secret {
                Some(secret) => ProviderCredentials::bearer(secret)
                    .map_err(|_| AiProviderStoreError::InvalidCredential)?,
                None => ProviderCredentials::none(),
            };
            let provider = BuiltinProvider::from_config(config)
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            let request = CompletionRequest::provider_connection_check();
            let started = Instant::now();
            let result = provider.complete(request, credentials).await;
            Ok(AiModelValidationView {
                ok: result.is_ok(),
                latency_ms: started.elapsed().as_millis(),
                error_code: result.err().map(model_validation_error_code),
            })
        }

        async fn clear_model_profile_key(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            let owned: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM ai_model_profiles
                 WHERE id = $1 AND user_id = $2 AND lab_id = $3
                   AND deleted_at IS NULL
                 FOR UPDATE",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if owned.is_none() {
                return Err(AiProviderStoreError::ModelProfileNotFound);
            }
            let deleted_versions: Vec<i64> = sqlx::query_scalar(
                "DELETE FROM ai_model_profile_secrets
                 WHERE profile_id = $1
                 RETURNING profile_version",
            )
            .bind(profile_id)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            for profile_version in deleted_versions {
                write_human_profile_secret_audit(
                    &mut transaction,
                    lab_id,
                    profile_id,
                    profile_version,
                    "delete",
                    audit,
                    true,
                    false,
                    "AI model profile credentials cleared for every immutable version",
                )
                .await?;
            }
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            self.model_profile_view_by_id(user_id, profile_id).await
        }

        async fn archive_model_profile(
            &self,
            user_id: Uuid,
            profile_id: Uuid,
            revision: i64,
            audit: &AuditContext,
        ) -> Result<AiModelProfileView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            if revision <= 0 {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            let current = sqlx::query(
                "SELECT p.name, p.current_version, p.archived_at, p.revision,
                        v.protocol, v.transport, v.supports_vision,
                        EXISTS(
                            SELECT 1 FROM ai_model_profile_secrets s
                            WHERE s.profile_id = p.id
                              AND s.profile_version = p.current_version
                        ) AS has_key
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 WHERE p.id = $1 AND p.user_id = $2 AND p.lab_id = $3
                   AND p.deleted_at IS NULL
                 FOR UPDATE OF p",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::ModelProfileNotFound)?;
            let current_revision: i64 = current
                .try_get("revision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            if current_revision != revision {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            if current
                .try_get::<Option<DateTime<Utc>>, _>("archived_at")
                .map_err(|_| AiProviderStoreError::Storage)?
                .is_some()
            {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            let updated = sqlx::query(
                "UPDATE ai_model_profiles
                 SET archived_at = now(), updated_at = now(),
                     revision = revision + 1
                 WHERE id = $1 AND user_id = $2 AND revision = $3
                   AND archived_at IS NULL AND deleted_at IS NULL",
            )
            .bind(profile_id)
            .bind(user_id)
            .bind(revision)
            .execute(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if updated.rows_affected() != 1 {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            let defaults_before = sqlx::query(
                "SELECT default_conversation_profile_id,
                        default_vision_profile_id, revision
                 FROM ai_user_model_defaults
                 WHERE user_id = $1 AND deleted_at IS NULL
                 FOR UPDATE",
            )
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            if let Some(defaults_before) = defaults_before {
                let conversation: Option<Uuid> = defaults_before
                    .try_get("default_conversation_profile_id")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let vision: Option<Uuid> = defaults_before
                    .try_get("default_vision_profile_id")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                if conversation == Some(profile_id) || vision == Some(profile_id) {
                    let defaults_revision: i64 = defaults_before
                        .try_get("revision")
                        .map_err(|_| AiProviderStoreError::Storage)?;
                    let next_conversation = (conversation != Some(profile_id))
                        .then_some(conversation)
                        .flatten();
                    let next_vision = (vision != Some(profile_id)).then_some(vision).flatten();
                    sqlx::query(
                        "UPDATE ai_user_model_defaults
                         SET default_conversation_profile_id = $1,
                             default_vision_profile_id = $2,
                             updated_at = now(), revision = revision + 1
                         WHERE user_id = $3 AND revision = $4
                           AND deleted_at IS NULL",
                    )
                    .bind(next_conversation)
                    .bind(next_vision)
                    .bind(user_id)
                    .bind(defaults_revision)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| AiProviderStoreError::Storage)?;
                    write_model_defaults_management_audit(
                        &mut transaction,
                        lab_id,
                        user_id,
                        audit,
                        Some(model_defaults_audit_state(
                            conversation,
                            vision,
                            defaults_revision,
                        )),
                        model_defaults_audit_state(
                            next_conversation,
                            next_vision,
                            defaults_revision + 1,
                        ),
                        "Archived AI model profile removed from user defaults",
                    )
                    .await?;
                }
            }
            let name: String = current
                .try_get("name")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let current_version: i64 = current
                .try_get("current_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let protocol = Self::protocol_from_db(
                current
                    .try_get::<String, _>("protocol")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            let transport = Self::transport_from_db(
                current
                    .try_get::<String, _>("transport")
                    .map_err(|_| AiProviderStoreError::Storage)?
                    .as_str(),
            )?;
            let supports_vision: bool = current
                .try_get("supports_vision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let has_key: bool = current
                .try_get("has_key")
                .map_err(|_| AiProviderStoreError::Storage)?;
            write_model_profile_management_audit(
                &mut transaction,
                lab_id,
                profile_id,
                "archive",
                audit,
                Some(model_profile_audit_state(
                    &name,
                    current_version,
                    protocol,
                    transport,
                    supports_vision,
                    has_key,
                    false,
                    revision,
                )),
                model_profile_audit_state(
                    &name,
                    current_version,
                    protocol,
                    transport,
                    supports_vision,
                    has_key,
                    true,
                    revision + 1,
                ),
                "AI model profile archived",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            self.model_profile_view_by_id(user_id, profile_id).await
        }

        async fn get_model_defaults(
            &self,
            user_id: Uuid,
        ) -> Result<AiModelDefaultsView, AiProviderStoreError> {
            self.active_user_lab_from_pool(user_id).await?;
            let row = sqlx::query(
                "SELECT default_conversation_profile_id,
                        default_vision_profile_id, revision
                 FROM ai_user_model_defaults
                 WHERE user_id = $1 AND deleted_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            match row {
                Some(row) => Ok(AiModelDefaultsView {
                    default_conversation_profile_id: row
                        .try_get("default_conversation_profile_id")
                        .map_err(|_| AiProviderStoreError::Storage)?,
                    default_vision_profile_id: row
                        .try_get("default_vision_profile_id")
                        .map_err(|_| AiProviderStoreError::Storage)?,
                    revision: row
                        .try_get("revision")
                        .map_err(|_| AiProviderStoreError::Storage)?,
                }),
                None => Ok(AiModelDefaultsView {
                    default_conversation_profile_id: None,
                    default_vision_profile_id: None,
                    revision: 0,
                }),
            }
        }

        async fn save_model_defaults(
            &self,
            user_id: Uuid,
            input: SaveAiModelDefaultsInput,
            audit: &AuditContext,
        ) -> Result<AiModelDefaultsView, AiProviderStoreError> {
            validate_settings_audit(user_id, audit)?;
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            for (profile_id, requires_vision) in [
                (input.default_conversation_profile_id, false),
                (input.default_vision_profile_id, true),
            ] {
                let Some(profile_id) = profile_id else {
                    continue;
                };
                let row = sqlx::query(
                    "SELECT v.supports_vision
                     FROM ai_model_profiles p
                     JOIN ai_model_profile_versions v
                       ON v.profile_id = p.id AND v.version = p.current_version
                     WHERE p.id = $1 AND p.user_id = $2 AND p.lab_id = $3
                       AND p.archived_at IS NULL AND p.deleted_at IS NULL
                     FOR SHARE OF p",
                )
                .bind(profile_id)
                .bind(user_id)
                .bind(lab_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?
                .ok_or(AiProviderStoreError::ModelProfileNotFound)?;
                let supports_vision: bool = row
                    .try_get("supports_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                if requires_vision && !supports_vision {
                    return Err(AiProviderStoreError::InvalidSettings);
                }
            }
            let current = sqlx::query(
                "SELECT default_conversation_profile_id,
                        default_vision_profile_id, deleted_at, revision
                 FROM ai_user_model_defaults
                 WHERE user_id = $1
                 FOR UPDATE",
            )
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let current_deleted = current
                .as_ref()
                .map(|row| {
                    row.try_get::<Option<DateTime<Utc>>, _>("deleted_at")
                        .map_err(|_| AiProviderStoreError::Storage)
                })
                .transpose()?
                .flatten()
                .is_some();
            let before = current
                .as_ref()
                .filter(|_| !current_deleted)
                .map(|row| {
                    Ok::<_, AiProviderStoreError>(model_defaults_audit_state(
                        row.try_get("default_conversation_profile_id")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                        row.try_get("default_vision_profile_id")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                        row.try_get("revision")
                            .map_err(|_| AiProviderStoreError::Storage)?,
                    ))
                })
                .transpose()?;
            let current_revision: Option<i64> = current
                .as_ref()
                .filter(|_| !current_deleted)
                .map(|row| row.try_get::<i64, _>("revision"))
                .transpose()
                .map_err(|_| AiProviderStoreError::Storage)?;
            if input.expected_revision != current_revision.unwrap_or(0) {
                return Err(AiProviderStoreError::RevisionConflict);
            }
            let row = sqlx::query(
                "INSERT INTO ai_user_model_defaults (
                    user_id, default_conversation_profile_id,
                    default_vision_profile_id, created_at, updated_at,
                    deleted_at, revision
                 ) VALUES ($1,$2,$3,now(),now(),NULL,1)
                 ON CONFLICT (user_id) DO UPDATE
                 SET default_conversation_profile_id =
                        EXCLUDED.default_conversation_profile_id,
                     default_vision_profile_id =
                        EXCLUDED.default_vision_profile_id,
                     updated_at = now(), deleted_at = NULL,
                     revision = ai_user_model_defaults.revision + 1
                 RETURNING revision",
            )
            .bind(user_id)
            .bind(input.default_conversation_profile_id)
            .bind(input.default_vision_profile_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let next_revision: i64 = row
                .try_get("revision")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let after = model_defaults_audit_state(
                input.default_conversation_profile_id,
                input.default_vision_profile_id,
                next_revision,
            );
            write_model_defaults_management_audit(
                &mut transaction,
                lab_id,
                user_id,
                audit,
                before,
                after,
                "AI model defaults saved",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(AiModelDefaultsView {
                default_conversation_profile_id: input.default_conversation_profile_id,
                default_vision_profile_id: input.default_vision_profile_id,
                revision: next_revision,
            })
        }
    }

    fn preset(
        id: &str,
        display_name: &str,
        recommended_base_url: &str,
        documentation_url: &str,
        default_preset: bool,
        supports_vision: bool,
        models: &[(&str, &str, u32, u32, bool)],
    ) -> AiProviderPresetView {
        AiProviderPresetView {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            provider_kind: ProviderKind::OpenAiCompatible,
            recommended_base_url: recommended_base_url.to_owned(),
            models: models
                .iter()
                .map(
                    |(id, display_name, context, output, vision)| AiProviderModelPresetView {
                        id: (*id).to_owned(),
                        display_name: (*display_name).to_owned(),
                        context_window_tokens: *context,
                        max_output_tokens: *output,
                        supports_vision: *vision,
                    },
                )
                .collect(),
            supports_vision,
            documentation_url: documentation_url.to_owned(),
            builtin: true,
            enabled: true,
            default_preset,
        }
    }

    fn validate_preset_id(value: &str) -> Result<(), AiProviderStoreError> {
        if value.is_empty()
            || value.len() > 160
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
        {
            Err(AiProviderStoreError::ProviderNotSelected)
        } else {
            Ok(())
        }
    }

    fn numeric_i64(row: &sqlx::postgres::PgRow, column: &str) -> Result<i64, AiProviderStoreError> {
        row.try_get::<i64, _>(column)
            .or_else(|_| row.try_get::<i32, _>(column).map(i64::from))
            .map_err(|_| AiProviderStoreError::Storage)
    }

    fn numeric_u32(row: &sqlx::postgres::PgRow, column: &str) -> Result<u32, AiProviderStoreError> {
        u32::try_from(numeric_i64(row, column)?).map_err(|_| AiProviderStoreError::Storage)
    }

    fn numeric_u64(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64, AiProviderStoreError> {
        u64::try_from(numeric_i64(row, column)?).map_err(|_| AiProviderStoreError::Storage)
    }

    fn runtime_from_row(
        row: &sqlx::postgres::PgRow,
    ) -> Result<AssistantRuntimeConfig, AiProviderStoreError> {
        AssistantRuntimeConfig {
            context_window_tokens: numeric_u32(row, "context_window_tokens")?,
            max_input_tokens: numeric_u32(row, "max_input_tokens")?,
            max_output_tokens: numeric_u32(row, "max_output_tokens")?,
            history_token_budget: numeric_u32(row, "history_token_budget")?,
            history_turns: numeric_u32(row, "history_turns")?,
            temperature: row
                .try_get::<f64, _>("temperature")
                .map_err(|_| AiProviderStoreError::Storage)? as f32,
            timeout_ms: numeric_u64(row, "timeout_ms")?,
        }
        .validate()
        .map_err(|_| AiProviderStoreError::InvalidSettings)
    }

    fn credential_identity_matches(
        row: &sqlx::postgres::PgRow,
        config: &ProviderConfig,
    ) -> Result<bool, AiProviderStoreError> {
        let current = decode_config(row)?;
        Ok(current.kind == config.kind
            && normalized_url(&current.base_url) == normalized_url(&config.base_url))
    }

    fn validate_settings_audit(
        user_id: Uuid,
        audit: &AuditContext,
    ) -> Result<(), AiProviderStoreError> {
        if audit.actor.actor_type != ActorType::Human || audit.actor.user_id != Some(user_id) {
            Err(AiProviderStoreError::Storage)
        } else {
            Ok(())
        }
    }

    struct EndpointAuditChange {
        endpoint_id: Uuid,
        action: &'static str,
        before: Option<Value>,
        after: Option<Value>,
        reason: &'static str,
    }

    async fn write_endpoint_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        audit: &AuditContext,
        change: EndpointAuditChange,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        sqlx::query(
            "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1,$2,NULL,'ai_provider_endpoint',$3,$4,'human',$5,$6,$7,$8,$9,$10,$11,now())",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(change.endpoint_id)
        .bind(change.action)
        .bind(actor_user_id)
        .bind(&audit.actor.display_name)
        .bind(write_source_name(audit.source))
        .bind(&audit.request_id)
        .bind(change.reason)
        .bind(change.before)
        .bind(change.after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    fn settings_audit_state(
        row: Option<&sqlx::postgres::PgRow>,
    ) -> Result<Value, AiProviderStoreError> {
        let Some(row) = row else {
            return Ok(json!({
                "configured": false,
                "enabled": false,
                "credential_present": false,
                "revision": 0,
            }));
        };
        Ok(json!({
            "configured": true,
            "enabled": row.try_get::<bool, _>("enabled").map_err(|_| AiProviderStoreError::Storage)?,
            "credential_present": row.try_get::<Option<Vec<u8>>, _>("secret_ciphertext").map_err(|_| AiProviderStoreError::Storage)?.is_some(),
            "revision": row.try_get::<i64, _>("revision").map_err(|_| AiProviderStoreError::Storage)?,
        }))
    }

    fn tagged_settings_audit_state(
        mut state: Value,
        operation: &'static str,
        credential_action: &'static str,
    ) -> Value {
        if let Some(object) = state.as_object_mut() {
            object.insert("operation".to_owned(), json!(operation));
            object.insert("credential_action".to_owned(), json!(credential_action));
        }
        state
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_profile_secret_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        profile_id: Uuid,
        profile_version: i64,
        action: &'static str,
        actor_type: &'static str,
        actor_user_id: Option<Uuid>,
        actor_display_name: &str,
        source: &'static str,
        request_id: Option<&str>,
        before_present: bool,
        after_present: bool,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        let before = json!({
            "profile_version": profile_version,
            "credential_present": before_present,
            "secret_material_redacted": true,
        });
        let after = json!({
            "profile_version": profile_version,
            "credential_present": after_present,
            "secret_material_redacted": true,
            "aad_binding": "user_profile_version_master_key_version",
        });
        sqlx::query(
            "INSERT INTO audit_entries (
                id, lab_id, project_id, entity_type, entity_id, action,
                actor_type, actor_user_id, actor_display_name, source,
                request_id, reason, before_json, after_json, occurred_at
             ) VALUES (
                $1, $2, NULL, 'ai_model_profile', $3, $4,
                $5, $6, $7, $8, $9, $10, $11, $12, now()
             )",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(profile_id)
        .bind(action)
        .bind(actor_type)
        .bind(actor_user_id)
        .bind(actor_display_name)
        .bind(source)
        .bind(request_id)
        .bind(reason)
        .bind(before)
        .bind(after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_human_profile_secret_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        profile_id: Uuid,
        profile_version: i64,
        action: &'static str,
        audit: &AuditContext,
        before_present: bool,
        after_present: bool,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        write_profile_secret_audit(
            transaction,
            lab_id,
            profile_id,
            profile_version,
            action,
            "human",
            audit.actor.user_id,
            &audit.actor.display_name,
            write_source_name(audit.source),
            audit.request_id.as_deref(),
            before_present,
            after_present,
            reason,
        )
        .await
    }

    async fn write_profile_configuration_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        profile_id: Uuid,
        previous_version: i64,
        next_version: i64,
        supports_vision: bool,
        audit: &AuditContext,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        let before = json!({
            "profile_version": previous_version,
            "configuration_present": previous_version > 0,
        });
        let after = json!({
            "profile_version": next_version,
            "protocol": "openai_chat_completions",
            "base_url_present": true,
            "model_id_present": true,
            "supports_vision": supports_vision,
            "configuration_secret_material_redacted": true,
        });
        let (action, reason) = if previous_version == 0 {
            ("create", "AI model profile configuration initialized")
        } else {
            ("update", "AI model profile configuration version appended")
        };
        sqlx::query(
            "INSERT INTO audit_entries (
                id, lab_id, project_id, entity_type, entity_id, action,
                actor_type, actor_user_id, actor_display_name, source,
                 request_id, reason, before_json, after_json, occurred_at
             ) VALUES (
                $1, $2, NULL, 'ai_model_profile', $3, $4,
                'human', $5, $6, $7, $8, $9, $10, $11, now()
             )",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(profile_id)
        .bind(action)
        .bind(actor_user_id)
        .bind(&audit.actor.display_name)
        .bind(write_source_name(audit.source))
        .bind(&audit.request_id)
        .bind(reason)
        .bind(before)
        .bind(after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_model_defaults_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        user_id: Uuid,
        old_conversation_profile_id: Option<Uuid>,
        old_vision_profile_id: Option<Uuid>,
        conversation_profile_id: Uuid,
        vision_profile_id: Option<Uuid>,
        created: bool,
        audit: &AuditContext,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        let before = json!({
            "configured": !created,
            "default_conversation_profile_id": old_conversation_profile_id,
            "default_vision_profile_id": old_vision_profile_id,
        });
        let after = json!({
            "configured": true,
            "default_conversation_profile_id": conversation_profile_id,
            "default_vision_profile_id": vision_profile_id,
        });
        sqlx::query(
            "INSERT INTO audit_entries (
                id, lab_id, project_id, entity_type, entity_id, action,
                actor_type, actor_user_id, actor_display_name, source,
                request_id, reason, before_json, after_json, occurred_at
             ) VALUES (
                $1, $2, NULL, 'ai_user_model_defaults', $3, $4,
                'human', $5, $6, $7, $8, $9, $10, $11, now()
             )",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(user_id)
        .bind(if created { "create" } else { "update" })
        .bind(actor_user_id)
        .bind(&audit.actor.display_name)
        .bind(write_source_name(audit.source))
        .bind(&audit.request_id)
        .bind("AI model profile defaults initialized")
        .bind(before)
        .bind(after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn model_profile_audit_state(
        name: &str,
        current_version: i64,
        protocol: AiProviderProtocol,
        transport: AiProviderTransport,
        supports_vision: bool,
        credential_present: bool,
        archived: bool,
        revision: i64,
    ) -> Value {
        json!({
            "name": name,
            "current_version": current_version,
            "protocol": PostgresAiProviderStore::protocol_name(protocol),
            "transport": PostgresAiProviderStore::transport_name(transport),
            "base_url_present": true,
            "model_id_present": true,
            "supports_vision": supports_vision,
            "credential_present": credential_present,
            "secret_material_redacted": true,
            "archived": archived,
            "revision": revision,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_model_profile_management_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        profile_id: Uuid,
        action: &'static str,
        audit: &AuditContext,
        before: Option<Value>,
        after: Value,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        sqlx::query(
            "INSERT INTO audit_entries (
                id, lab_id, project_id, entity_type, entity_id, action,
                actor_type, actor_user_id, actor_display_name, source,
                request_id, reason, before_json, after_json, occurred_at
             ) VALUES (
                $1,$2,NULL,'ai_model_profile',$3,$4,'human',$5,$6,$7,$8,$9,$10,$11,now()
             )",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(profile_id)
        .bind(action)
        .bind(actor_user_id)
        .bind(&audit.actor.display_name)
        .bind(write_source_name(audit.source))
        .bind(&audit.request_id)
        .bind(reason)
        .bind(before)
        .bind(after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    fn model_defaults_audit_state(
        conversation_profile_id: Option<Uuid>,
        vision_profile_id: Option<Uuid>,
        revision: i64,
    ) -> Value {
        json!({
            "default_conversation_profile_id": conversation_profile_id,
            "default_vision_profile_id": vision_profile_id,
            "revision": revision,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_model_defaults_management_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        user_id: Uuid,
        audit: &AuditContext,
        before: Option<Value>,
        after: Value,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        sqlx::query(
            "INSERT INTO audit_entries (
                id, lab_id, project_id, entity_type, entity_id, action,
                actor_type, actor_user_id, actor_display_name, source,
                request_id, reason, before_json, after_json, occurred_at
             ) VALUES (
                $1,$2,NULL,'ai_user_model_defaults',$3,$4,'human',$5,$6,$7,$8,$9,$10,$11,now()
             )",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(user_id)
        .bind(if before.is_some() { "update" } else { "create" })
        .bind(actor_user_id)
        .bind(&audit.actor.display_name)
        .bind(write_source_name(audit.source))
        .bind(&audit.request_id)
        .bind(reason)
        .bind(before)
        .bind(after)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    fn model_validation_error_code(error: ProviderError) -> &'static str {
        match error {
            ProviderError::InvalidConfig(_) | ProviderError::InvalidRequest(_) => {
                "invalid_provider"
            }
            ProviderError::RequestTooLarge { .. } => "context_exceeded",
            ProviderError::ResponseTooLarge { .. } => "response_too_large",
            ProviderError::Transport {
                kind: TransportFailure::Timeout,
            } => "request_timeout",
            ProviderError::Transport {
                kind: TransportFailure::Connection,
            } => "provider_unreachable",
            ProviderError::Transport {
                kind: TransportFailure::Request,
            } => "provider_transport_error",
            ProviderError::HttpStatus {
                status: 401 | 403, ..
            } => "api_key_rejected",
            ProviderError::HttpStatus { status: 404, .. } => "model_not_found",
            ProviderError::HttpStatus { .. } => "provider_http_error",
            ProviderError::MalformedResponse | ProviderError::EmptyResponse => {
                "response_format_incompatible"
            }
            ProviderError::OutputBudgetExhausted => "output_budget_exhausted",
            ProviderError::MockExhausted | ProviderError::MockUnavailable => "provider_unavailable",
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_settings_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        user_id: Uuid,
        action: &'static str,
        before: Value,
        after: Value,
        audit: &AuditContext,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        sqlx::query("INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, 'ai_provider_settings', $3, $4, 'human', $5, $6, $7, $8, $9, $10, $11, $12)")
            .bind(Uuid::new_v4())
            .bind(lab_id)
            .bind(user_id)
            .bind(action)
            .bind(user_id)
            .bind(&audit.actor.display_name)
            .bind(write_source_name(audit.source))
            .bind(&audit.request_id)
            .bind(reason)
            .bind(before)
            .bind(after)
            .bind(Utc::now())
            .execute(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
        Ok(())
    }

    const fn write_source_name(source: WriteSource) -> &'static str {
        match source {
            WriteSource::Desktop => "desktop",
            WriteSource::Web => "web",
            WriteSource::Api => "api",
            WriteSource::Mcp => "mcp",
            WriteSource::Ai => "ai",
            WriteSource::Migration => "migration",
        }
    }

    fn decode_config(row: &sqlx::postgres::PgRow) -> Result<ProviderConfig, AiProviderStoreError> {
        let value: serde_json::Value = row
            .try_get("provider_config")
            .map_err(|_| AiProviderStoreError::Storage)?;
        serde_json::from_value(value).map_err(|_| AiProviderStoreError::Storage)
    }

    fn normalized_url(value: &str) -> String {
        value.trim().trim_end_matches('/').to_owned()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use muriarc_core::{
            Actor, AiConversation, AiOperationStore, AuditFilter, EntityType, Lab, MuriArcStore,
            RecordMeta, User,
        };

        fn master() -> AiMasterKey {
            AiMasterKey {
                bytes: Zeroizing::new(vec![7_u8; KEY_BYTES]),
                version: 1,
            }
        }

        fn master_with(byte: u8, version: i32) -> AiMasterKey {
            AiMasterKey {
                bytes: Zeroizing::new(vec![byte; KEY_BYTES]),
                version,
            }
        }

        #[tokio::test]
        async fn legacy_ciphertext_is_user_bound_and_secret_is_redacted() {
            let store = PostgresAiProviderStore::new(
                PostgresStore::from_pool(
                    sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap(),
                ),
                master(),
            );
            let user = Uuid::new_v4();
            let (nonce, ciphertext) = store.encrypt(user, "super-secret-key").unwrap();
            assert!(!String::from_utf8_lossy(&ciphertext).contains("super-secret-key"));
            let decrypted = store.decrypt(user, 1, &nonce, &ciphertext).unwrap();
            assert_eq!(decrypted.as_str(), "super-secret-key");
            assert!(!format!("{decrypted:?}").contains("super-secret-key"));
            assert!(
                store
                    .decrypt(Uuid::new_v4(), 1, &nonce, &ciphertext)
                    .is_err()
            );

            let profile_id = Uuid::new_v4();
            let (profile_key_version, profile_nonce, profile_ciphertext) = store
                .reencrypt_legacy_secret_for_profile(user, profile_id, 1, 1, &nonce, &ciphertext)
                .unwrap();
            assert_eq!(
                store
                    .decrypt_profile_secret(
                        user,
                        profile_id,
                        1,
                        profile_key_version,
                        &profile_nonce,
                        &profile_ciphertext,
                    )
                    .unwrap()
                    .as_str(),
                "super-secret-key"
            );
            assert_eq!(
                store
                    .decrypt(user, 1, &nonce, &ciphertext)
                    .unwrap()
                    .as_str(),
                "super-secret-key"
            );
        }

        #[tokio::test]
        async fn profile_ciphertext_is_bound_to_user_profile_version_and_master_key_version() {
            let store = PostgresAiProviderStore::new(
                PostgresStore::from_pool(
                    sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap(),
                ),
                master(),
            );
            let user_id = Uuid::new_v4();
            let profile_id = Uuid::new_v4();
            let profile_version = 7;
            let (key_version, nonce, ciphertext) = store
                .encrypt_profile_secret(user_id, profile_id, profile_version, "profile-secret-key")
                .unwrap();

            assert_eq!(key_version, 1);
            assert!(!String::from_utf8_lossy(&ciphertext).contains("profile-secret-key"));
            let decrypted = store
                .decrypt_profile_secret(
                    user_id,
                    profile_id,
                    profile_version,
                    key_version,
                    &nonce,
                    &ciphertext,
                )
                .unwrap();
            assert_eq!(decrypted.as_str(), "profile-secret-key");
            assert!(!format!("{decrypted:?}").contains("profile-secret-key"));

            assert!(
                store
                    .decrypt_profile_secret(
                        Uuid::new_v4(),
                        profile_id,
                        profile_version,
                        key_version,
                        &nonce,
                        &ciphertext,
                    )
                    .is_err()
            );
            assert!(
                store
                    .decrypt_profile_secret(
                        user_id,
                        Uuid::new_v4(),
                        profile_version,
                        key_version,
                        &nonce,
                        &ciphertext,
                    )
                    .is_err()
            );
            assert!(
                store
                    .decrypt_profile_secret(
                        user_id,
                        profile_id,
                        profile_version + 1,
                        key_version,
                        &nonce,
                        &ciphertext,
                    )
                    .is_err()
            );
            assert!(
                store
                    .decrypt_profile_secret(
                        user_id,
                        profile_id,
                        profile_version,
                        key_version + 1,
                        &nonce,
                        &ciphertext,
                    )
                    .is_err()
            );
            assert!(
                store
                    .decrypt(user_id, key_version, &nonce, &ciphertext)
                    .is_err()
            );
        }

        #[tokio::test]
        async fn master_key_rotation_fails_closed_until_secrets_are_explicitly_reencrypted() {
            let old_store = PostgresAiProviderStore::new(
                PostgresStore::from_pool(
                    sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap(),
                ),
                master_with(7, 1),
            );
            let rotated_store = PostgresAiProviderStore::new(
                PostgresStore::from_pool(
                    sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap(),
                ),
                master_with(9, 2),
            );
            let wrong_old_store = PostgresAiProviderStore::new(
                PostgresStore::from_pool(
                    sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap(),
                ),
                master_with(8, 1),
            );
            let user_id = Uuid::new_v4();
            let profile_id = Uuid::new_v4();
            let profile_version = 3;
            let secret = "master-key-rotation-secret";
            let (old_key_version, old_nonce, old_ciphertext) = old_store
                .encrypt_profile_secret(user_id, profile_id, profile_version, secret)
                .unwrap();

            let rotation_error = rotated_store
                .decrypt_profile_secret(
                    user_id,
                    profile_id,
                    profile_version,
                    old_key_version,
                    &old_nonce,
                    &old_ciphertext,
                )
                .unwrap_err();
            assert!(matches!(rotation_error, AiProviderStoreError::Encryption));
            assert!(!format!("{rotation_error:?}").contains(secret));

            let wrong_key_error = wrong_old_store
                .decrypt_profile_secret(
                    user_id,
                    profile_id,
                    profile_version,
                    old_key_version,
                    &old_nonce,
                    &old_ciphertext,
                )
                .unwrap_err();
            assert!(matches!(wrong_key_error, AiProviderStoreError::Encryption));
            assert!(!format!("{wrong_key_error:?}").contains(secret));

            let plaintext = old_store
                .decrypt_profile_secret(
                    user_id,
                    profile_id,
                    profile_version,
                    old_key_version,
                    &old_nonce,
                    &old_ciphertext,
                )
                .unwrap();
            let (new_key_version, new_nonce, new_ciphertext) = rotated_store
                .encrypt_profile_secret(user_id, profile_id, profile_version, plaintext.as_str())
                .unwrap();
            assert_eq!(new_key_version, 2);
            assert!(
                old_store
                    .decrypt_profile_secret(
                        user_id,
                        profile_id,
                        profile_version,
                        new_key_version,
                        &new_nonce,
                        &new_ciphertext,
                    )
                    .is_err()
            );
            assert_eq!(
                rotated_store
                    .decrypt_profile_secret(
                        user_id,
                        profile_id,
                        profile_version,
                        new_key_version,
                        &new_nonce,
                        &new_ciphertext,
                    )
                    .unwrap()
                    .as_str(),
                secret
            );
        }

        fn settings_input(
            preset_id: &str,
            model: &str,
            base_url: &str,
            api_key: Option<&str>,
        ) -> SaveAiProviderSettingsInput {
            SaveAiProviderSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                provider_preset_id: preset_id.to_owned(),
                model: model.to_owned(),
                base_url: base_url.to_owned(),
                supports_vision: false,
                vision_model: None,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: api_key.map(str::to_owned),
            }
        }

        fn user_audit(user: &User, request_id: &str) -> AuditContext {
            AuditContext {
                actor: Actor::human(user.id, user.display_name.clone()),
                source: WriteSource::Web,
                request_id: Some(request_id.to_owned()),
                reason: Some("test personal AI settings isolation".to_owned()),
            }
        }

        fn model_input(
            name: &str,
            base_url: &str,
            model_id: &str,
            api_key: Option<&str>,
            expected_revision: Option<i64>,
        ) -> SaveAiModelProfileInput {
            SaveAiModelProfileInput {
                name: name.to_owned(),
                protocol: AiProviderProtocol::OpenaiChatCompletions,
                transport: AiProviderTransport::OpenAiCompatible,
                base_url: base_url.to_owned(),
                model_id: model_id.to_owned(),
                supports_vision: true,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: api_key.map(str::to_owned),
                expected_revision,
            }
        }

        #[tokio::test]
        async fn model_management_lifecycle_is_versioned_isolated_and_audited() {
            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(format!("AI model API lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-model-api-{}@example.test", Uuid::new_v4()),
                "AI model API owner",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();
            let store = PostgresAiProviderStore::new(postgres.clone(), master());

            let created = store
                .create_model_profile(
                    user.id,
                    model_input(
                        "Primary model",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "自由填写/模型-v1",
                        Some("phase-two-profile-key"),
                        None,
                    ),
                    &user_audit(&user, "model-create"),
                )
                .await
                .unwrap();
            assert_eq!(created.current_version, 1);
            assert!(created.has_key);
            assert_eq!(created.model_id, "自由填写/模型-v1");
            assert_eq!(
                store
                    .list_model_profiles(user.id, false)
                    .await
                    .unwrap()
                    .iter()
                    .filter(|profile| profile.id == created.id)
                    .count(),
                1
            );
            let key_rotated = store
                .update_model_profile(
                    user.id,
                    created.id,
                    model_input(
                        "Primary model",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "自由填写/模型-v1",
                        Some("phase-two-profile-key-rotated"),
                        Some(created.revision),
                    ),
                    &user_audit(&user, "model-key-only-rotation"),
                )
                .await
                .unwrap();
            assert_eq!(key_rotated.current_version, created.current_version);
            assert_eq!(key_rotated.revision, created.revision);
            let version_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_versions WHERE profile_id = $1",
            )
            .bind(created.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(version_count, 1);
            let rotated_secret = sqlx::query(
                "SELECT key_version, nonce, ciphertext
                 FROM ai_model_profile_secrets
                 WHERE profile_id = $1 AND profile_version = $2",
            )
            .bind(created.id)
            .bind(created.current_version)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(
                store
                    .decrypt_profile_secret(
                        user.id,
                        created.id,
                        created.current_version,
                        rotated_secret.try_get("key_version").unwrap(),
                        &rotated_secret.try_get::<Vec<u8>, _>("nonce").unwrap(),
                        &rotated_secret.try_get::<Vec<u8>, _>("ciphertext").unwrap(),
                    )
                    .unwrap()
                    .as_str(),
                "phase-two-profile-key-rotated"
            );

            let requires_new_key = store
                .update_model_profile(
                    user.id,
                    created.id,
                    model_input(
                        "Primary model",
                        OFFICIAL_OPENAI_BASE_URL,
                        "自由填写/模型-v2",
                        None,
                        Some(created.revision),
                    ),
                    &user_audit(&user, "model-provider-change-without-key"),
                )
                .await
                .unwrap_err();
            assert!(matches!(
                requires_new_key,
                AiProviderStoreError::CredentialRequired
            ));

            let updated = store
                .update_model_profile(
                    user.id,
                    created.id,
                    model_input(
                        "Primary model renamed",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "自由填写/模型-v2",
                        None,
                        Some(created.revision),
                    ),
                    &user_audit(&user, "model-update"),
                )
                .await
                .unwrap();
            assert_eq!(updated.current_version, 2);
            assert_eq!(updated.revision, 2);
            assert!(updated.has_key);
            assert_eq!(updated.name, "Primary model renamed");
            let stale = store
                .update_model_profile(
                    user.id,
                    created.id,
                    model_input(
                        "Stale",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "stale",
                        None,
                        Some(created.revision),
                    ),
                    &user_audit(&user, "model-stale-update"),
                )
                .await
                .unwrap_err();
            assert!(matches!(stale, AiProviderStoreError::RevisionConflict));

            let defaults = store
                .save_model_defaults(
                    user.id,
                    SaveAiModelDefaultsInput {
                        default_conversation_profile_id: Some(created.id),
                        default_vision_profile_id: Some(created.id),
                        expected_revision: 0,
                    },
                    &user_audit(&user, "model-defaults"),
                )
                .await
                .unwrap();
            assert_eq!(defaults.revision, 1);
            let with_defaults = store.get_model_profile(user.id, created.id).await.unwrap();
            assert!(with_defaults.is_default_conversation);
            assert!(with_defaults.is_default_vision);
            let mut would_disable_default_vision = model_input(
                "Primary model renamed",
                OFFICIAL_DEEPSEEK_BASE_URL,
                "自由填写/模型-v3",
                None,
                Some(updated.revision),
            );
            would_disable_default_vision.supports_vision = false;
            let disable_error = store
                .update_model_profile(
                    user.id,
                    created.id,
                    would_disable_default_vision,
                    &user_audit(&user, "model-disable-default-vision"),
                )
                .await
                .unwrap_err();
            assert!(matches!(
                disable_error,
                AiProviderStoreError::InvalidSettings
            ));
            assert_eq!(
                store
                    .get_model_profile(user.id, created.id)
                    .await
                    .unwrap()
                    .revision,
                updated.revision
            );
            let legacy_config = ProviderConfig::openai_compatible(
                "legacy-disabled-row",
                "legacy-model",
                OFFICIAL_DEEPSEEK_BASE_URL,
            );
            sqlx::query(
                "INSERT INTO ai_provider_settings (
                    user_id, enabled, provider_config,
                    created_at, updated_at, revision
                 ) VALUES ($1,FALSE,$2,now(),now(),1)",
            )
            .bind(user.id)
            .bind(serde_json::to_value(legacy_config).unwrap())
            .execute(postgres.pool())
            .await
            .unwrap();
            let resolved_without_legacy_enable = store.resolve(user.id).await.unwrap();
            assert_eq!(
                resolved_without_legacy_enable.model_profile.profile_id,
                created.id
            );
            assert_eq!(
                resolved_without_legacy_enable.model_profile.profile_version,
                updated.current_version
            );
            assert_eq!(
                resolved_without_legacy_enable
                    .api_key
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "phase-two-profile-key-rotated"
            );

            let cleared = store
                .clear_model_profile_key(user.id, created.id, &user_audit(&user, "model-clear-key"))
                .await
                .unwrap();
            assert!(!cleared.has_key);
            let remaining_secrets: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_secrets WHERE profile_id = $1",
            )
            .bind(created.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(remaining_secrets, 0);
            let updated_without_credential = store
                .update_model_profile(
                    user.id,
                    created.id,
                    model_input(
                        "Primary model without credential",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "自由填写/模型-v3",
                        None,
                        Some(cleared.revision),
                    ),
                    &user_audit(&user, "model-update-after-key-clear"),
                )
                .await
                .unwrap();
            assert_eq!(updated_without_credential.current_version, 3);
            assert_eq!(updated_without_credential.revision, 3);
            assert!(!updated_without_credential.has_key);

            let archived = store
                .archive_model_profile(
                    user.id,
                    created.id,
                    updated_without_credential.revision,
                    &user_audit(&user, "model-archive"),
                )
                .await
                .unwrap();
            assert!(archived.archived_at.is_some());
            assert!(!archived.is_default_conversation);
            assert!(!archived.is_default_vision);
            assert!(
                store
                    .list_model_profiles(user.id, false)
                    .await
                    .unwrap()
                    .iter()
                    .all(|profile| profile.id != created.id)
            );
            assert!(
                store
                    .list_model_profiles(user.id, true)
                    .await
                    .unwrap()
                    .iter()
                    .any(|profile| profile.id == created.id)
            );
            let defaults_after_archive = store.get_model_defaults(user.id).await.unwrap();
            assert_eq!(defaults_after_archive.default_conversation_profile_id, None);
            assert_eq!(defaults_after_archive.default_vision_profile_id, None);

            let audit_contains_secret: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM audit_entries
                    WHERE entity_id = $1
                      AND (
                        COALESCE(before_json::text, '') LIKE '%phase-two-profile-key%'
                        OR COALESCE(after_json::text, '') LIKE '%phase-two-profile-key%'
                        OR COALESCE(before_json::text, '') LIKE '%phase-two-profile-key-rotated%'
                        OR COALESCE(after_json::text, '') LIKE '%phase-two-profile-key-rotated%'
                      )
                 )",
            )
            .bind(created.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert!(!audit_contains_secret);
        }

        #[tokio::test]
        async fn unsaved_model_validation_uses_local_mock_without_persisting() {
            use axum::{Json, Router, routing::post};

            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let mock = Router::new()
                .route(
                    "/chat/completions",
                    post(|| async {
                        Json(json!({
                            "id": "mock-validation",
                            "choices": [{
                                "index": 0,
                                "message": {"role": "assistant", "content": "OK"},
                                "finish_reason": "stop"
                            }],
                            "usage": {
                                "prompt_tokens": 8,
                                "completion_tokens": 1,
                                "total_tokens": 9
                            }
                        }))
                    }),
                )
                .route(
                    "/responses",
                    post(|| async {
                        Json(json!({
                            "id": "mock-responses-validation",
                            "model": "unsaved-local-model",
                            "status": "completed",
                            "output": [{
                                "type": "message",
                                "id": "mock-message",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": "OK",
                                    "annotations": []
                                }]
                            }],
                            "usage": {
                                "input_tokens": 8,
                                "output_tokens": 1,
                                "total_tokens": 9
                            }
                        }))
                    }),
                )
                .route(
                    "/messages",
                    post(|| async {
                        Json(json!({
                            "id": "mock-anthropic-validation",
                            "type": "message",
                            "role": "assistant",
                            "model": "unsaved-local-model",
                            "content": [{"type": "text", "text": "OK"}],
                            "stop_reason": "end_turn",
                            "usage": {"input_tokens": 8, "output_tokens": 1}
                        }))
                    }),
                );
            let mock_task = tokio::spawn(async move {
                axum::serve(listener, mock).await.unwrap();
            });

            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(format!("AI validation lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-validation-{}@example.test", Uuid::new_v4()),
                "AI validation owner",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();
            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let base_url = format!("http://{address}");
            for protocol in [
                AiProviderProtocol::OpenaiChatCompletions,
                AiProviderProtocol::OpenaiResponses,
                AiProviderProtocol::AnthropicMessages,
            ] {
                store
                    .save_provider_endpoint(
                        lab.id,
                        None,
                        SaveAiProviderEndpointInput {
                            provider_kind: ProviderKind::LocalHttp,
                            protocol,
                            label: format!("Phase two {protocol:?} validation mock"),
                            base_url: base_url.clone(),
                            enabled: true,
                        },
                        &user_audit(&user, "validation-endpoint"),
                    )
                    .await
                    .unwrap();
            }
            let reusable_profile = store
                .create_model_profile(
                    user.id,
                    model_input(
                        "Validation credential source",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        "validation-source-model",
                        Some("validation-source-key"),
                        None,
                    ),
                    &user_audit(&user, "validation-source-profile"),
                )
                .await
                .unwrap();
            let profile_count_before: i64 =
                sqlx::query_scalar("SELECT count(*) FROM ai_model_profiles WHERE user_id = $1")
                    .bind(user.id)
                    .fetch_one(postgres.pool())
                    .await
                    .unwrap();
            for protocol in [
                AiProviderProtocol::OpenaiChatCompletions,
                AiProviderProtocol::OpenaiResponses,
                AiProviderProtocol::AnthropicMessages,
            ] {
                let result = store
                    .validate_model_profile(
                        user.id,
                        ValidateAiModelProfileInput {
                            protocol,
                            transport: AiProviderTransport::LocalHttp,
                            base_url: base_url.clone(),
                            model_id: "unsaved-local-model".to_owned(),
                            supports_vision: false,
                            context_window_tokens: default_context_window_tokens(),
                            max_input_tokens: default_max_input_tokens(),
                            max_output_tokens: default_max_output_tokens(),
                            history_token_budget: default_history_token_budget(),
                            history_turns: default_history_turns(),
                            temperature: default_temperature(),
                            timeout_ms: default_timeout_ms(),
                            api_key: None,
                            profile_id: None,
                            current_version: None,
                        },
                    )
                    .await
                    .unwrap();
                assert!(result.ok, "{protocol:?}: {:?}", result.error_code);
            }
            let missing_cloud_credential = store
                .validate_model_profile(
                    user.id,
                    ValidateAiModelProfileInput {
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        transport: AiProviderTransport::OpenAiCompatible,
                        base_url: OFFICIAL_DEEPSEEK_BASE_URL.to_owned(),
                        model_id: "unsaved-cloud-model".to_owned(),
                        supports_vision: false,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: None,
                        profile_id: None,
                        current_version: None,
                    },
                )
                .await
                .unwrap();
            assert!(!missing_cloud_credential.ok);
            assert_eq!(
                missing_cloud_credential.error_code,
                Some("missing_credential")
            );
            assert_eq!(missing_cloud_credential.latency_ms, 0);

            let stale_binding = store
                .validate_model_profile(
                    user.id,
                    ValidateAiModelProfileInput {
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        transport: AiProviderTransport::OpenAiCompatible,
                        base_url: OFFICIAL_DEEPSEEK_BASE_URL.to_owned(),
                        model_id: "validation-source-model".to_owned(),
                        supports_vision: false,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: None,
                        profile_id: Some(reusable_profile.id),
                        current_version: Some(reusable_profile.current_version + 1),
                    },
                )
                .await
                .unwrap();
            assert_eq!(stale_binding.error_code, Some("missing_credential"));
            assert_eq!(stale_binding.latency_ms, 0);

            let identity_mismatch = store
                .validate_model_profile(
                    user.id,
                    ValidateAiModelProfileInput {
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        transport: AiProviderTransport::OpenAiCompatible,
                        base_url: OFFICIAL_OPENAI_BASE_URL.to_owned(),
                        model_id: "validation-source-model".to_owned(),
                        supports_vision: false,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: None,
                        profile_id: Some(reusable_profile.id),
                        current_version: Some(reusable_profile.current_version),
                    },
                )
                .await
                .unwrap();
            assert_eq!(identity_mismatch.error_code, Some("missing_credential"));
            assert_eq!(identity_mismatch.latency_ms, 0);

            store
                .archive_model_profile(
                    user.id,
                    reusable_profile.id,
                    reusable_profile.revision,
                    &user_audit(&user, "validation-source-archive"),
                )
                .await
                .unwrap();
            let archived_binding = store
                .validate_model_profile(
                    user.id,
                    ValidateAiModelProfileInput {
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        transport: AiProviderTransport::OpenAiCompatible,
                        base_url: OFFICIAL_DEEPSEEK_BASE_URL.to_owned(),
                        model_id: "validation-source-model".to_owned(),
                        supports_vision: false,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: None,
                        profile_id: Some(reusable_profile.id),
                        current_version: Some(reusable_profile.current_version),
                    },
                )
                .await
                .unwrap();
            assert_eq!(archived_binding.error_code, Some("missing_credential"));
            assert_eq!(archived_binding.latency_ms, 0);

            let archived_hint_does_not_block_local_validation = store
                .validate_model_profile(
                    user.id,
                    ValidateAiModelProfileInput {
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        transport: AiProviderTransport::LocalHttp,
                        base_url: base_url.clone(),
                        model_id: "unsaved-local-model".to_owned(),
                        supports_vision: false,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: None,
                        profile_id: Some(reusable_profile.id),
                        current_version: Some(reusable_profile.current_version),
                    },
                )
                .await
                .unwrap();
            assert!(
                archived_hint_does_not_block_local_validation.ok,
                "{:?}",
                archived_hint_does_not_block_local_validation.error_code
            );

            let profile_count_after: i64 =
                sqlx::query_scalar("SELECT count(*) FROM ai_model_profiles WHERE user_id = $1")
                    .bind(user.id)
                    .fetch_one(postgres.pool())
                    .await
                    .unwrap();
            assert_eq!(profile_count_after, profile_count_before);
            mock_task.abort();
        }

        async fn insert_test_profile_defaults(
            postgres: &PostgresStore,
            user: &User,
            with_vision: bool,
        ) -> (Uuid, Option<Uuid>) {
            let conversation_profile_id = Uuid::new_v4();
            let vision_profile_id = with_vision.then(Uuid::new_v4);
            let mut transaction = postgres.pool().begin().await.unwrap();
            for (profile_id, name, model, supports_vision) in [
                (
                    conversation_profile_id,
                    "Test conversation model",
                    "test-conversation-model",
                    false,
                ),
                (
                    vision_profile_id.unwrap_or(Uuid::nil()),
                    "Test vision model",
                    "test-vision-model",
                    true,
                ),
            ] {
                if profile_id.is_nil() {
                    continue;
                }
                sqlx::query(
                    "INSERT INTO ai_model_profiles (
                        id, lab_id, user_id, name, current_version,
                        created_at, updated_at, revision
                     ) VALUES ($1, $2, $3, $4, 1, now(), now(), 1)",
                )
                .bind(profile_id)
                .bind(user.lab_id)
                .bind(user.id)
                .bind(name)
                .execute(&mut *transaction)
                .await
                .unwrap();
                sqlx::query(
                    "INSERT INTO ai_model_profile_versions (
                        profile_id, version, protocol, transport, base_url,
                        normalized_base_url, model_id, supports_vision,
                        context_window_tokens, max_input_tokens, max_output_tokens,
                        history_token_budget, history_turns, temperature,
                        timeout_ms, created_at
                     ) VALUES (
                        $1, 1, 'openai_chat_completions',
                        'open_ai_compatible',
                        'https://api.deepseek.com', 'https://api.deepseek.com',
                        $2, $3, 131072, 65536, 4096, 32768, 20, 0, 120000, now()
                     )",
                )
                .bind(profile_id)
                .bind(model)
                .bind(supports_vision)
                .execute(&mut *transaction)
                .await
                .unwrap();
            }
            sqlx::query(
                "INSERT INTO ai_user_model_defaults (
                    user_id, default_conversation_profile_id,
                    default_vision_profile_id, created_at, updated_at, revision
                 ) VALUES ($1, $2, $3, now(), now(), 1)",
            )
            .bind(user.id)
            .bind(conversation_profile_id)
            .bind(vision_profile_id)
            .execute(&mut *transaction)
            .await
            .unwrap();
            transaction.commit().await.unwrap();
            (conversation_profile_id, vision_profile_id)
        }

        #[tokio::test]
        async fn root_editor_and_viewer_provider_settings_are_strictly_isolated() {
            use muriarc_ai::AiProvider;

            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(format!("AI isolation lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let root = User::new(
                lab.id,
                format!("root-ai-{}@example.test", Uuid::new_v4()),
                "Root AI owner",
                now,
            )
            .unwrap();
            let editor = User::new(
                lab.id,
                format!("editor-ai-{}@example.test", Uuid::new_v4()),
                "Editor AI owner",
                now,
            )
            .unwrap();
            let viewer = User::new(
                lab.id,
                format!("viewer-ai-{}@example.test", Uuid::new_v4()),
                "Viewer AI owner",
                now,
            )
            .unwrap();
            let newcomer = User::new(
                lab.id,
                format!("new-ai-{}@example.test", Uuid::new_v4()),
                "New AI user",
                now,
            )
            .unwrap();
            for user in [&root, &editor, &viewer, &newcomer] {
                postgres.create_user(user, &bootstrap).await.unwrap();
            }

            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let cases = [
                (
                    &root,
                    "deepseek",
                    "deepseek-chat",
                    OFFICIAL_DEEPSEEK_BASE_URL,
                    "root-deepseek-key-a",
                ),
                (
                    &editor,
                    "zhipu-glm",
                    "glm-5.2",
                    OFFICIAL_GLM_BASE_URL,
                    "editor-glm-key-b",
                ),
                (
                    &viewer,
                    "moonshot-kimi",
                    "kimi-k3",
                    OFFICIAL_KIMI_BASE_URL,
                    "viewer-kimi-key-c",
                ),
            ];
            for (user, preset, model, base_url, api_key) in cases {
                insert_test_profile_defaults(&postgres, user, false).await;
                let saved = store
                    .save(
                        user.id,
                        settings_input(preset, model, base_url, Some(api_key)),
                        &user_audit(user, &format!("save-{preset}")),
                    )
                    .await
                    .unwrap();
                assert!(saved.has_key);
                assert_eq!(saved.provider_preset_id, preset);
                assert_eq!(saved.model, model);
                let serialized = serde_json::to_value(saved).unwrap();
                assert!(serialized.get("apiKey").is_none());
                assert!(serialized.get("secretCiphertext").is_none());

                let resolved = store.resolve(user.id).await.unwrap();
                assert_eq!(resolved.provider.model(), model);
                assert_eq!(resolved.api_key.as_ref().unwrap().as_str(), api_key);
            }

            let root_updated = store
                .save(
                    root.id,
                    settings_input(
                        "deepseek",
                        "deepseek-reasoner",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        None,
                    ),
                    &user_audit(&root, "root-model-update"),
                )
                .await
                .unwrap();
            assert!(root_updated.has_key);
            assert_eq!(root_updated.model, "deepseek-reasoner");
            assert_eq!(
                store
                    .resolve(root.id)
                    .await
                    .unwrap()
                    .api_key
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "root-deepseek-key-a"
            );
            assert_eq!(store.get(editor.id).await.unwrap().model, "glm-5.2");
            assert_eq!(store.get(viewer.id).await.unwrap().model, "kimi-k3");

            let editor_switched = store
                .save(
                    editor.id,
                    settings_input("moonshot-kimi", "kimi-k3", OFFICIAL_KIMI_BASE_URL, None),
                    &user_audit(&editor, "editor-provider-switch"),
                )
                .await
                .unwrap();
            assert!(!editor_switched.has_key);
            assert!(matches!(
                store.resolve(editor.id).await,
                Err(AiProviderStoreError::MissingCredential)
            ));
            assert_eq!(
                store
                    .resolve(viewer.id)
                    .await
                    .unwrap()
                    .api_key
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "viewer-kimi-key-c"
            );

            let cross_user_write = store
                .save(
                    editor.id,
                    settings_input(
                        "zhipu-glm",
                        "glm-5.2",
                        OFFICIAL_GLM_BASE_URL,
                        Some("must-not-be-written"),
                    ),
                    &user_audit(&root, "root-cannot-write-editor-secret"),
                )
                .await;
            assert!(matches!(
                cross_user_write,
                Err(AiProviderStoreError::Storage)
            ));

            let defaults = store.get(newcomer.id).await.unwrap();
            assert!(defaults.enabled);
            assert_eq!(defaults.provider_preset_id, "deepseek");
            assert!(!defaults.has_key);
            let diagnostics = store.diagnostics(newcomer.id, lab.id).await.unwrap();
            assert!(diagnostics.runtime_configured);
            assert!(diagnostics.lab_enabled);
            assert!(diagnostics.user_enabled);
            assert!(diagnostics.provider_presets_available);
            assert_eq!(diagnostics.status, "waiting_for_personal_api_key");

            drop(store);
            let restarted_store = PostgresAiProviderStore::new(postgres, master());
            let after_restart = restarted_store.resolve(root.id).await.unwrap();
            assert_eq!(after_restart.provider.model(), "deepseek-reasoner");
            assert_eq!(
                after_restart.api_key.as_ref().unwrap().as_str(),
                "root-deepseek-key-a"
            );
        }

        #[tokio::test]
        async fn new_user_save_materializes_defaults_versions_secrets_and_turn_binding() {
            use muriarc_ai::AiProvider;

            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(format!("AI first-save lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-first-save-{}@example.test", Uuid::new_v4()),
                "AI first save researcher",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();
            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let mut first_input = settings_input(
                "deepseek",
                "first-save-model-v1",
                OFFICIAL_DEEPSEEK_BASE_URL,
                Some("first-save-profile-key"),
            );
            first_input.supports_vision = true;
            first_input.vision_model = Some("first-save-vision-v1".to_owned());
            store
                .save(user.id, first_input, &user_audit(&user, "first-save-v1"))
                .await
                .unwrap();

            let (conversation_profile_id, vision_profile_id): (Uuid, Option<Uuid>) =
                sqlx::query_as(
                    "SELECT default_conversation_profile_id,
                            default_vision_profile_id
                     FROM ai_user_model_defaults
                     WHERE user_id = $1 AND deleted_at IS NULL",
                )
                .bind(user.id)
                .fetch_one(postgres.pool())
                .await
                .unwrap();
            let vision_profile_id = vision_profile_id.unwrap();
            assert_ne!(conversation_profile_id, vision_profile_id);
            let first = store.resolve(user.id).await.unwrap();
            let first_conversation_binding = first.model_profile;
            assert_eq!(first.model_profile.profile_id, conversation_profile_id);
            assert_eq!(first.model_profile.profile_version, 1);
            assert_eq!(first.provider.model(), "first-save-model-v1");
            assert_eq!(
                first.api_key.as_ref().unwrap().as_str(),
                "first-save-profile-key"
            );
            let vision = store.resolve_vision(user.id).await.unwrap();
            let first_vision_binding = vision.model_profile;
            assert_eq!(vision.model_profile.profile_id, vision_profile_id);
            assert_eq!(vision.model_profile.profile_version, 1);
            assert_eq!(vision.provider.model(), "first-save-vision-v1");
            let secret_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_secrets
                 WHERE profile_id = ANY($1)",
            )
            .bind(vec![conversation_profile_id, vision_profile_id])
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(secret_count, 2);

            let turn = AiConversation {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id: None,
                user_id: user.id,
                title: "First profile-bound turn".to_owned(),
                model_profile: Some(first.model_profile),
                legacy_read_only: false,
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(Utc::now()),
            };
            postgres
                .create_ai_conversation(
                    &turn,
                    &user_audit(&user, "create-first-profile-bound-turn"),
                )
                .await
                .unwrap();
            assert_eq!(
                postgres
                    .get_ai_conversation(turn.id)
                    .await
                    .unwrap()
                    .model_profile,
                Some(first.model_profile)
            );

            let mut repeated = settings_input(
                "deepseek",
                "first-save-model-v1",
                OFFICIAL_DEEPSEEK_BASE_URL,
                None,
            );
            repeated.supports_vision = true;
            repeated.vision_model = Some("first-save-vision-v1".to_owned());
            store
                .save(user.id, repeated, &user_audit(&user, "repeat-first-save"))
                .await
                .unwrap();
            let version_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_versions
                 WHERE profile_id = $1",
            )
            .bind(conversation_profile_id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(version_count, 1);

            let mut model_update = settings_input(
                "deepseek",
                "first-save-model-v2",
                OFFICIAL_DEEPSEEK_BASE_URL,
                None,
            );
            model_update.supports_vision = true;
            model_update.vision_model = Some("first-save-vision-v1".to_owned());
            store
                .save(
                    user.id,
                    model_update,
                    &user_audit(&user, "first-save-model-v2"),
                )
                .await
                .unwrap();
            let current = store.resolve(user.id).await.unwrap();
            assert_eq!(current.model_profile.profile_version, 2);
            assert_eq!(current.provider.model(), "first-save-model-v2");
            assert_eq!(
                store
                    .resolve_for_profile(user.id, first_conversation_binding)
                    .await
                    .unwrap()
                    .api_key
                    .unwrap()
                    .as_str(),
                "first-save-profile-key"
            );

            let mut provider_change =
                settings_input("openai", "gpt-test", OFFICIAL_OPENAI_BASE_URL, None);
            provider_change.supports_vision = true;
            provider_change.vision_model = Some("gpt-vision-test".to_owned());
            store
                .save(
                    user.id,
                    provider_change,
                    &user_audit(&user, "first-save-provider-change"),
                )
                .await
                .unwrap();
            let secret_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_secrets
                 WHERE profile_id = ANY($1)",
            )
            .bind(vec![conversation_profile_id, vision_profile_id])
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(secret_count, 3);
            assert!(matches!(
                store.resolve(user.id).await,
                Err(AiProviderStoreError::MissingCredential)
            ));
            assert!(matches!(
                store.resolve_vision(user.id).await,
                Err(AiProviderStoreError::MissingCredential)
            ));
            assert_eq!(
                store
                    .resolve_for_profile(user.id, first_conversation_binding)
                    .await
                    .unwrap()
                    .api_key
                    .unwrap()
                    .as_str(),
                "first-save-profile-key"
            );
            assert_eq!(
                store
                    .resolve_profile(user.id, first_vision_binding, true)
                    .await
                    .unwrap()
                    .api_key
                    .unwrap()
                    .as_str(),
                "first-save-profile-key"
            );

            let mut provider_change_with_key = settings_input(
                "openai",
                "gpt-test",
                OFFICIAL_OPENAI_BASE_URL,
                Some("second-provider-key"),
            );
            provider_change_with_key.supports_vision = true;
            provider_change_with_key.vision_model = Some("gpt-vision-test".to_owned());
            store
                .save(
                    user.id,
                    provider_change_with_key,
                    &user_audit(&user, "save-second-provider-key"),
                )
                .await
                .unwrap();
            let second_provider = store.resolve(user.id).await.unwrap();
            assert_eq!(second_provider.provider.model(), "gpt-test");
            assert_eq!(
                second_provider.api_key.unwrap().as_str(),
                "second-provider-key"
            );
            assert_eq!(
                store
                    .resolve_for_profile(user.id, first_conversation_binding)
                    .await
                    .unwrap()
                    .api_key
                    .unwrap()
                    .as_str(),
                "first-save-profile-key"
            );

            let mut disable_vision =
                settings_input("openai", "gpt-test", OFFICIAL_OPENAI_BASE_URL, None);
            disable_vision.vision_model = Some("residual-model-must-be-ignored".to_owned());
            let disabled = store
                .save(
                    user.id,
                    disable_vision,
                    &user_audit(&user, "disable-default-vision"),
                )
                .await
                .unwrap();
            assert!(!disabled.supports_vision);
            assert_eq!(disabled.vision_model, None);
            let default_vision_profile_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT default_vision_profile_id
                 FROM ai_user_model_defaults
                 WHERE user_id = $1 AND deleted_at IS NULL",
            )
            .bind(user.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(default_vision_profile_id, None);
            assert!(matches!(
                store.resolve_vision(user.id).await,
                Err(AiProviderStoreError::ProviderNotSelected)
            ));
            assert_eq!(
                store
                    .resolve_profile(user.id, first_vision_binding, true)
                    .await
                    .unwrap()
                    .api_key
                    .unwrap()
                    .as_str(),
                "first-save-profile-key"
            );

            let clear_audit = user_audit(&user, "clear-all-profile-secrets");
            let cleared = store.clear_key(user.id, &clear_audit).await.unwrap();
            assert!(!cleared.has_key);
            let secret_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_secrets
                 WHERE profile_id = ANY($1)",
            )
            .bind(vec![conversation_profile_id, vision_profile_id])
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(secret_count, 0);
            assert!(matches!(
                store
                    .resolve_for_profile(user.id, first_conversation_binding)
                    .await,
                Err(AiProviderStoreError::MissingCredential)
            ));
            assert!(matches!(
                store
                    .resolve_profile(user.id, first_vision_binding, true)
                    .await,
                Err(AiProviderStoreError::MissingCredential)
            ));
            let secret_delete_audits: Vec<Value> = sqlx::query_scalar(
                "SELECT after_json
                 FROM audit_entries
                 WHERE entity_type = 'ai_model_profile'
                   AND action = 'delete'
                   AND request_id = $1
                 ORDER BY entity_id, occurred_at, id",
            )
            .bind("clear-all-profile-secrets")
            .fetch_all(postgres.pool())
            .await
            .unwrap();
            assert_eq!(secret_delete_audits.len(), 5);
            assert!(secret_delete_audits.iter().all(|payload| {
                payload["profile_version"]
                    .as_i64()
                    .is_some_and(|value| value > 0)
                    && payload["aad_binding"] == "user_profile_version_master_key_version"
                    && payload["credential_present"] == false
                    && payload["secret_material_redacted"] == true
            }));
        }

        #[tokio::test]
        async fn https_local_http_transport_round_trips_and_resolves_exactly() {
            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(
                format!("AI HTTPS local transport lab {}", Uuid::new_v4()),
                now,
            )
            .unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-local-https-{}@example.test", Uuid::new_v4()),
                "AI HTTPS local transport researcher",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();

            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let base_url = "https://local-gateway.example.test/v1";
            let audit = user_audit(&user, "save-local-https-transport");
            let endpoint = store
                .save_provider_endpoint(
                    lab.id,
                    None,
                    SaveAiProviderEndpointInput {
                        provider_kind: ProviderKind::LocalHttp,
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        label: "HTTPS local gateway".to_owned(),
                        base_url: base_url.to_owned(),
                        enabled: true,
                    },
                    &audit,
                )
                .await
                .unwrap();
            let mut input = settings_input(&endpoint.id.to_string(), "local-model", base_url, None);
            input.provider_kind = ProviderKind::LocalHttp;
            let saved = store.save(user.id, input, &audit).await.unwrap();
            assert!(!saved.has_key);

            let resolved = store.resolve(user.id).await.unwrap();
            assert_eq!(resolved.provider.config().kind, ProviderKind::LocalHttp);
            assert_eq!(resolved.provider.config().base_url, base_url);
            assert!(resolved.api_key.is_none());
            let persisted: (String, String) = sqlx::query_as(
                "SELECT v.transport, v.base_url
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 WHERE p.id = $1",
            )
            .bind(resolved.model_profile.profile_id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(persisted.0, "local_http");
            assert_eq!(persisted.1, base_url);
        }

        #[tokio::test]
        async fn legacy_save_appends_versions_and_old_binding_resolves_original_config() {
            use muriarc_ai::AiProvider;

            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab =
                Lab::new(format!("AI immutable profile lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-immutable-profile-{}@example.test", Uuid::new_v4()),
                "AI immutable profile researcher",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();
            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let (profile_id, _) = insert_test_profile_defaults(&postgres, &user, false).await;

            store
                .save(
                    user.id,
                    settings_input(
                        "deepseek",
                        "immutable-model-v1",
                        OFFICIAL_DEEPSEEK_BASE_URL,
                        Some("immutable-profile-key"),
                    ),
                    &user_audit(&user, "save-immutable-v1"),
                )
                .await
                .unwrap();
            let original = store.resolve(user.id).await.unwrap();
            let original_binding = original.model_profile;
            assert_eq!(original_binding.profile_id, profile_id);
            assert_eq!(original.provider.model(), "immutable-model-v1");
            assert_eq!(original.runtime.max_output_tokens, 4_096);

            let mut updated = settings_input(
                "deepseek",
                "immutable-model-v2",
                OFFICIAL_DEEPSEEK_BASE_URL,
                None,
            );
            updated.max_output_tokens = 8_192;
            store
                .save(user.id, updated, &user_audit(&user, "save-immutable-v2"))
                .await
                .unwrap();

            let still_original = store
                .resolve_for_profile(user.id, original_binding)
                .await
                .unwrap();
            assert_eq!(still_original.model_profile, original_binding);
            assert_eq!(still_original.provider.model(), "immutable-model-v1");
            assert_eq!(still_original.runtime.max_output_tokens, 4_096);
            assert_eq!(
                still_original.api_key.unwrap().as_str(),
                "immutable-profile-key"
            );

            let current = store.resolve(user.id).await.unwrap();
            assert_eq!(current.model_profile.profile_id, profile_id);
            assert!(current.model_profile.profile_version > original_binding.profile_version);
            assert_eq!(current.provider.model(), "immutable-model-v2");
            assert_eq!(current.runtime.max_output_tokens, 8_192);
            assert_eq!(current.api_key.unwrap().as_str(), "immutable-profile-key");

            let version_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_model_profile_versions WHERE profile_id = $1",
            )
            .bind(profile_id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(version_count, 3);
        }

        #[tokio::test]
        async fn startup_migrates_text_and_vision_secrets_idempotently_without_leaking() {
            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(
                format!("AI profile secret migration lab {}", Uuid::new_v4()),
                now,
            )
            .unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-secret-migration-{}@example.test", Uuid::new_v4()),
                "AI secret migration researcher",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();

            let sensitive_key = "profile-migration-secret-that-must-stay-redacted";
            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            let mut input = settings_input(
                "deepseek",
                "deepseek-chat",
                OFFICIAL_DEEPSEEK_BASE_URL,
                Some(sensitive_key),
            );
            input.supports_vision = true;
            input.vision_model = Some("deepseek-vision-test".to_owned());
            store
                .save(user.id, input, &user_audit(&user, "save-legacy-secret"))
                .await
                .unwrap();
            let legacy_before: (Option<i32>, Option<Vec<u8>>, Option<Vec<u8>>) = sqlx::query_as(
                "SELECT secret_key_version, secret_nonce, secret_ciphertext
                     FROM ai_provider_settings WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            let (conversation_profile_id, vision_profile_id): (Uuid, Option<Uuid>) =
                sqlx::query_as(
                    "SELECT default_conversation_profile_id,
                            default_vision_profile_id
                     FROM ai_user_model_defaults
                     WHERE user_id = $1 AND deleted_at IS NULL",
                )
                .bind(user.id)
                .fetch_one(postgres.pool())
                .await
                .unwrap();
            let vision_profile_id = vision_profile_id.unwrap();
            let profile_ids = vec![conversation_profile_id, vision_profile_id];
            sqlx::query("DELETE FROM ai_model_profile_secrets WHERE profile_id = ANY($1)")
                .bind(&profile_ids)
                .execute(postgres.pool())
                .await
                .unwrap();
            sqlx::query(
                "DELETE FROM audit_entries
                 WHERE entity_type = 'ai_model_profile' AND entity_id = ANY($1)",
            )
            .bind(&profile_ids)
            .execute(postgres.pool())
            .await
            .unwrap();

            let migrated = store.migrate_legacy_profile_secrets().await.unwrap();
            assert!(migrated >= 2);
            for profile_id in [conversation_profile_id, vision_profile_id] {
                let (profile_version, key_version, nonce, ciphertext): (
                    i64,
                    i32,
                    Vec<u8>,
                    Vec<u8>,
                ) = sqlx::query_as(
                    "SELECT profile_version, key_version, nonce, ciphertext
                         FROM ai_model_profile_secrets WHERE profile_id = $1",
                )
                .bind(profile_id)
                .fetch_one(postgres.pool())
                .await
                .unwrap();
                assert_eq!(
                    store
                        .decrypt_profile_secret(
                            user.id,
                            profile_id,
                            profile_version,
                            key_version,
                            &nonce,
                            &ciphertext,
                        )
                        .unwrap()
                        .as_str(),
                    sensitive_key
                );
            }
            let legacy_after: (Option<i32>, Option<Vec<u8>>, Option<Vec<u8>>) = sqlx::query_as(
                "SELECT secret_key_version, secret_nonce, secret_ciphertext
                     FROM ai_provider_settings WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(legacy_after, legacy_before);

            let text = store.resolve(user.id).await.unwrap();
            assert_eq!(text.model_profile.profile_id, conversation_profile_id);
            assert_eq!(text.model_profile.profile_version, 1);
            assert_eq!(text.api_key.unwrap().as_str(), sensitive_key);
            let vision = store.resolve_vision(user.id).await.unwrap();
            assert_eq!(vision.model_profile.profile_id, vision_profile_id);
            assert_eq!(vision.model_profile.profile_version, 1);
            assert_eq!(vision.api_key.unwrap().as_str(), sensitive_key);

            let audit_payloads: Vec<String> = sqlx::query_scalar(
                "SELECT coalesce(reason, '') || coalesce(before_json::text, '')
                        || coalesce(after_json::text, '')
                 FROM audit_entries
                 WHERE entity_type = 'ai_model_profile'
                   AND entity_id = ANY($1)
                 ORDER BY occurred_at, id",
            )
            .bind(vec![conversation_profile_id, vision_profile_id])
            .fetch_all(postgres.pool())
            .await
            .unwrap();
            assert_eq!(audit_payloads.len(), 2);
            assert!(
                audit_payloads
                    .iter()
                    .all(|payload| !payload.contains(sensitive_key))
            );
            assert!(
                audit_payloads
                    .iter()
                    .all(|payload| payload.contains("secret_material_redacted"))
            );

            let migrated_again = store.migrate_legacy_profile_secrets().await.unwrap();
            assert_eq!(migrated_again, 0);
            let audit_count_after: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_entries
                 WHERE entity_type = 'ai_model_profile'
                   AND entity_id = ANY($1)",
            )
            .bind(vec![conversation_profile_id, vision_profile_id])
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(audit_count_after, 2);
        }

        #[test]
        fn master_key_requires_exactly_32_base64_bytes() {
            let encoded = general_purpose::STANDARD.encode([9_u8; KEY_BYTES]);
            assert!(AiMasterKey::from_base64(&encoded, 1).is_ok());
            assert!(AiMasterKey::from_base64("not-a-key", 1).is_err());
        }

        #[test]
        fn every_profile_protocol_is_forwarded_to_provider_construction() {
            for protocol in [
                AiProviderProtocol::OpenaiChatCompletions,
                AiProviderProtocol::OpenaiResponses,
                AiProviderProtocol::AnthropicMessages,
            ] {
                let config = PostgresAiProviderStore::profile_provider_config(
                    protocol,
                    AiProviderTransport::OpenAiCompatible,
                    "test-model".to_owned(),
                    OFFICIAL_OPENAI_BASE_URL.to_owned(),
                    default_timeout_ms(),
                )
                .unwrap();
                assert_eq!(config.protocol, protocol);
            }
        }

        #[test]
        fn https_profile_uses_persisted_local_http_transport_without_scheme_inference() {
            let base_url = "https://local-gateway.example.test/v1";
            let config = PostgresAiProviderStore::profile_provider_config(
                AiProviderProtocol::OpenaiChatCompletions,
                AiProviderTransport::LocalHttp,
                "local-model".to_owned(),
                base_url.to_owned(),
                default_timeout_ms(),
            )
            .unwrap();
            assert_eq!(config.kind, ProviderKind::LocalHttp);
            assert_eq!(config.base_url, base_url);
        }

        #[tokio::test]
        async fn postgres_save_and_clear_are_atomically_audited_without_sensitive_settings() {
            let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
                return;
            };
            assert!(
                database_url.contains("muriarc_test"),
                "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
            );
            let postgres = PostgresStore::connect(&database_url).await.unwrap();
            postgres.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new(format!("AI audit lab {}", Uuid::new_v4()), now).unwrap();
            postgres.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(
                lab.id,
                format!("ai-audit-{}@example.test", Uuid::new_v4()),
                "AI audit researcher",
                now,
            )
            .unwrap();
            postgres.create_user(&user, &bootstrap).await.unwrap();

            let sensitive_url = "https://private-provider.example.test/v1";
            let sensitive_model = "private-model-name";
            let sensitive_key = "secret-provider-key-that-must-never-be-audited";
            let store = PostgresAiProviderStore::new(postgres.clone(), master());
            insert_test_profile_defaults(&postgres, &user, false).await;
            let audit = AuditContext {
                actor: Actor::human(user.id, user.display_name.clone()),
                source: WriteSource::Web,
                request_id: Some("ai-settings-audit-test".to_owned()),
                reason: Some(format!("must not copy {sensitive_key}")),
            };
            let endpoint = store
                .save_provider_endpoint(
                    lab.id,
                    None,
                    SaveAiProviderEndpointInput {
                        enabled: true,
                        provider_kind: ProviderKind::OpenAiCompatible,
                        protocol: AiProviderProtocol::OpenaiChatCompletions,
                        label: "Private compatible API".to_owned(),
                        base_url: sensitive_url.to_owned(),
                    },
                    &audit,
                )
                .await
                .unwrap();
            assert!(endpoint.enabled);
            let saved = store
                .save(
                    user.id,
                    SaveAiProviderSettingsInput {
                        enabled: true,
                        provider_kind: ProviderKind::OpenAiCompatible,
                        provider_preset_id: endpoint.id.to_string(),
                        model: sensitive_model.to_owned(),
                        base_url: sensitive_url.to_owned(),
                        supports_vision: false,
                        vision_model: None,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: Some(sensitive_key.to_owned()),
                    },
                    &audit,
                )
                .await
                .unwrap();
            assert!(saved.enabled);
            assert!(saved.has_key);

            let saved_audit: Value = sqlx::query_scalar(
                "SELECT after_json FROM audit_entries WHERE entity_type = 'ai_provider_settings' AND entity_id = $1 AND after_json->>'operation' = 'save' LIMIT 1",
            )
            .bind(user.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(saved_audit["operation"], "save");
            assert_eq!(saved_audit["credential_action"], "replace");
            assert_eq!(saved_audit["credential_present"], true);

            let cleared = store.clear_key(user.id, &audit).await.unwrap();
            assert!(cleared.enabled);
            assert!(!cleared.has_key);
            let audit_payloads: Vec<String> = sqlx::query_scalar(
                "SELECT coalesce(reason, '') || coalesce(before_json::text, '') || coalesce(after_json::text, '') FROM audit_entries WHERE entity_type = 'ai_provider_settings' AND entity_id = $1 ORDER BY occurred_at, id",
            )
            .bind(user.id)
            .fetch_all(postgres.pool())
            .await
            .unwrap();
            assert_eq!(audit_payloads.len(), 2);
            let audit_payloads = audit_payloads.join("\n");
            assert!(!audit_payloads.contains(sensitive_key));
            assert!(!audit_payloads.contains(sensitive_model));
            assert!(!audit_payloads.contains(sensitive_url));
            assert!(!audit_payloads.contains("secret_ciphertext"));
            assert!(audit_payloads.contains("credential_action"));
            assert!(audit_payloads.contains("clear"));
            let decoded_audits = postgres
                .list_audit_entries(&AuditFilter {
                    lab_id: lab.id,
                    project_id: None,
                    entity_id: Some(user.id),
                })
                .await
                .unwrap();
            assert_eq!(
                decoded_audits
                    .iter()
                    .filter(|entry| entry.entity_type == EntityType::AiProviderSettings)
                    .count(),
                2
            );

            let failed = store
                .save(
                    user.id,
                    SaveAiProviderSettingsInput {
                        enabled: true,
                        provider_kind: ProviderKind::OpenAiCompatible,
                        provider_preset_id: "custom-openai-compatible".to_owned(),
                        model: "rejected-model".to_owned(),
                        base_url: "https://not-allowlisted.example.test/v1".to_owned(),
                        supports_vision: false,
                        vision_model: None,
                        context_window_tokens: default_context_window_tokens(),
                        max_input_tokens: default_max_input_tokens(),
                        max_output_tokens: default_max_output_tokens(),
                        history_token_budget: default_history_token_budget(),
                        history_turns: default_history_turns(),
                        temperature: default_temperature(),
                        timeout_ms: default_timeout_ms(),
                        api_key: Some("another-key-that-must-not-be-recorded".to_owned()),
                    },
                    &audit,
                )
                .await;
            assert!(matches!(
                failed,
                Err(AiProviderStoreError::CloudUrlForbidden)
            ));
            let audit_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_entries WHERE entity_type = 'ai_provider_settings' AND entity_id = $1",
            )
            .bind(user.id)
            .fetch_one(postgres.pool())
            .await
            .unwrap();
            assert_eq!(audit_count, 2);
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::{AiMasterKey, PostgresAiProviderStore};
