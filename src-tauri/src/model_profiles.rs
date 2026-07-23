use std::time::Instant;

use chrono::{DateTime, Utc};
use muriarc_ai::{
    AiProvider, AssistantRuntimeConfig, BuiltinProvider, ChatMessage, CompletionRequest,
    ProviderConfig, ProviderCredentials, ProviderError, TransportFailure,
};
use muriarc_core::{
    AiModelProfile, AiModelProfileBinding, AiModelProfileFilter, AiModelProfileStore,
    AiModelProfileVersion, AiProviderProtocol, AiProviderTransport, AiUserModelDefaults,
    RecordMeta, StoreError,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    application::{DesktopError, DesktopState},
    settings::{AiSecret, AppendModelProfileWithSecret, SettingsError},
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveAiModelProfileInput {
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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ValidateAiModelProfileInput {
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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveAiModelDefaultsInput {
    pub default_conversation_profile_id: Option<Uuid>,
    pub default_vision_profile_id: Option<Uuid>,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiModelProfileView {
    pub id: Uuid,
    pub name: String,
    pub current_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    pub revision: i64,
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
    pub is_default_conversation: bool,
    pub is_default_vision: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiModelDefaultsView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_conversation_profile_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_vision_profile_id: Option<Uuid>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiModelValidationResult {
    pub ok: bool,
    pub latency_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

#[derive(Clone)]
struct ModelProfileConfiguration {
    protocol: AiProviderProtocol,
    transport: AiProviderTransport,
    base_url: String,
    normalized_base_url: String,
    model_id: String,
    supports_vision: bool,
    runtime: AssistantRuntimeConfig,
}

impl ModelProfileConfiguration {
    fn from_save(input: &SaveAiModelProfileInput) -> Result<Self, DesktopError> {
        Self::new(
            input.protocol,
            input.transport,
            &input.base_url,
            &input.model_id,
            input.supports_vision,
            input.context_window_tokens,
            input.max_input_tokens,
            input.max_output_tokens,
            input.history_token_budget,
            input.history_turns,
            input.temperature,
            input.timeout_ms,
        )
    }

    fn from_validation(input: &ValidateAiModelProfileInput) -> Result<Self, DesktopError> {
        Self::new(
            input.protocol,
            input.transport,
            &input.base_url,
            &input.model_id,
            input.supports_vision,
            input.context_window_tokens,
            input.max_input_tokens,
            input.max_output_tokens,
            input.history_token_budget,
            input.history_turns,
            input.temperature,
            input.timeout_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        protocol: AiProviderProtocol,
        transport: AiProviderTransport,
        base_url: &str,
        model_id: &str,
        supports_vision: bool,
        context_window_tokens: u32,
        max_input_tokens: u32,
        max_output_tokens: u32,
        history_token_budget: u32,
        history_turns: u32,
        temperature: f32,
        timeout_ms: u64,
    ) -> Result<Self, DesktopError> {
        let base_url = base_url.trim().to_owned();
        let model_id = model_id.trim().to_owned();
        if model_id.chars().any(char::is_control) {
            return Err(validation(
                "AI model identifier contains control characters",
            ));
        }
        let runtime = AssistantRuntimeConfig {
            context_window_tokens,
            max_input_tokens,
            max_output_tokens,
            history_token_budget,
            history_turns,
            temperature,
            timeout_ms,
        }
        .validate()
        .map_err(|_| SettingsError::InvalidFile)?;
        let value = Self {
            protocol,
            transport,
            normalized_base_url: normalize_base_url(&base_url),
            base_url,
            model_id,
            supports_vision,
            runtime,
        };
        BuiltinProvider::from_config(value.provider_config("desktop-model-profile-validation"))
            .map_err(SettingsError::InvalidProvider)?;
        Ok(value)
    }

    fn provider_config(&self, provider_id: impl Into<String>) -> ProviderConfig {
        let provider_id = provider_id.into();
        let mut config = match self.transport {
            AiProviderTransport::OpenAiCompatible => {
                ProviderConfig::openai_compatible_with_protocol(
                    provider_id,
                    self.protocol,
                    self.model_id.clone(),
                    self.base_url.clone(),
                )
            }
            AiProviderTransport::LocalHttp => ProviderConfig::local_http_with_protocol(
                provider_id,
                self.protocol,
                self.model_id.clone(),
                self.base_url.clone(),
            ),
        };
        config.timeout_ms = self.runtime.timeout_ms;
        config
    }

    fn version(
        &self,
        profile_id: Uuid,
        version: i64,
        created_at: DateTime<Utc>,
    ) -> AiModelProfileVersion {
        AiModelProfileVersion {
            profile_id,
            version,
            protocol: self.protocol,
            transport: self.transport,
            base_url: self.base_url.clone(),
            normalized_base_url: self.normalized_base_url.clone(),
            model_id: self.model_id.clone(),
            supports_vision: self.supports_vision,
            context_window_tokens: self.runtime.context_window_tokens,
            max_input_tokens: self.runtime.max_input_tokens,
            max_output_tokens: self.runtime.max_output_tokens,
            history_token_budget: self.runtime.history_token_budget,
            history_turns: self.runtime.history_turns,
            temperature: self.runtime.temperature,
            timeout_ms: self.runtime.timeout_ms,
            created_at,
        }
    }

    fn matches(&self, version: &AiModelProfileVersion) -> bool {
        version.protocol == self.protocol
            && version.transport == self.transport
            && version.base_url == self.base_url
            && version.normalized_base_url == self.normalized_base_url
            && version.model_id == self.model_id
            && version.supports_vision == self.supports_vision
            && version.context_window_tokens == self.runtime.context_window_tokens
            && version.max_input_tokens == self.runtime.max_input_tokens
            && version.max_output_tokens == self.runtime.max_output_tokens
            && version.history_token_budget == self.runtime.history_token_budget
            && version.history_turns == self.runtime.history_turns
            && version.temperature == self.runtime.temperature
            && version.timeout_ms == self.runtime.timeout_ms
    }

    fn credential_identity_matches(&self, version: &AiModelProfileVersion) -> bool {
        version.protocol == self.protocol
            && version.transport == self.transport
            && version.normalized_base_url == self.normalized_base_url
    }
}

impl DesktopState {
    pub(crate) async fn list_ai_model_profiles(
        &self,
    ) -> Result<Vec<AiModelProfileView>, DesktopError> {
        let profiles = self
            .domain_store()
            .list_ai_model_profiles(&AiModelProfileFilter {
                lab_id: self.local_lab_id(),
                user_id: self.local_user_id(),
                include_archived: false,
            })
            .await?;
        let defaults = self
            .domain_store()
            .get_ai_user_model_defaults(self.local_user_id())
            .await?;
        let mut views = Vec::with_capacity(profiles.len());
        for profile in profiles {
            views.push(self.model_profile_view(profile, defaults.as_ref()).await?);
        }
        Ok(views)
    }

    pub(crate) async fn get_ai_model_profile(
        &self,
        id: Uuid,
    ) -> Result<AiModelProfileView, DesktopError> {
        let profile = self.owned_model_profile(id).await?;
        let defaults = self
            .domain_store()
            .get_ai_user_model_defaults(self.local_user_id())
            .await?;
        self.model_profile_view(profile, defaults.as_ref()).await
    }

    pub(crate) async fn create_ai_model_profile(
        &self,
        input: SaveAiModelProfileInput,
    ) -> Result<AiModelProfileView, DesktopError> {
        let _operation = self
            .model_profile_settings()
            .profile_coordinator()
            .lock()
            .await;
        if input.expected_revision.is_some() {
            return Err(validation(
                "new AI model profiles must not include an expected revision",
            ));
        }
        let name = validated_name(&input.name)?;
        let configuration = ModelProfileConfiguration::from_save(&input)?;
        let api_key = validated_api_key(input.api_key.as_deref())?;
        if configuration.transport == AiProviderTransport::OpenAiCompatible && api_key.is_none() {
            return Err(SettingsError::MissingCredential.into());
        }
        let now = Utc::now();
        let profile = AiModelProfile {
            id: Uuid::new_v4(),
            lab_id: self.local_lab_id(),
            user_id: self.local_user_id(),
            name,
            current_version: 1,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let version = configuration.version(profile.id, 1, now);
        let audit = self.audit("create_ai_model_profile").await?;
        self.model_profile_settings()
            .create_model_profile_with_secret(
                self.domain_store(),
                &profile,
                &version,
                api_key,
                &audit,
            )
            .await?;
        self.get_ai_model_profile(profile.id).await
    }

    pub(crate) async fn update_ai_model_profile(
        &self,
        id: Uuid,
        input: SaveAiModelProfileInput,
    ) -> Result<AiModelProfileView, DesktopError> {
        let _operation = self
            .model_profile_settings()
            .profile_coordinator()
            .lock()
            .await;
        let mut profile = self.owned_model_profile(id).await?;
        if profile.archived_at.is_some() {
            return Err(validation("archived AI model profiles cannot be edited"));
        }
        let expected_revision = input
            .expected_revision
            .ok_or_else(|| validation("an expected revision is required"))?;
        if profile.meta.revision != expected_revision {
            return Err(
                StoreError::Conflict("AI model profile changed concurrently".to_owned()).into(),
            );
        }
        let name = validated_name(&input.name)?;
        let configuration = ModelProfileConfiguration::from_save(&input)?;
        let api_key = validated_api_key(input.api_key.as_deref())?;
        let current = self
            .domain_store()
            .get_ai_model_profile_version(profile.id, profile.current_version)
            .await?;
        if !configuration.supports_vision
            && self
                .domain_store()
                .get_ai_user_model_defaults(self.local_user_id())
                .await?
                .is_some_and(|defaults| defaults.default_vision_profile_id == Some(profile.id))
        {
            return Err(validation(
                "clear the default vision model before disabling vision support",
            ));
        }
        let configuration_changed = !configuration.matches(&current);
        let name_changed = profile.name != name;
        let current_binding = AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: profile.current_version,
        };
        let audit = self.audit("update_ai_model_profile").await?;
        if !configuration_changed && !name_changed {
            if let Some(api_key) = api_key {
                self.model_profile_settings()
                    .rotate_model_profile_secret(
                        self.domain_store(),
                        current_binding,
                        api_key,
                        &audit,
                    )
                    .await?;
            }
            return self.get_ai_model_profile(profile.id).await;
        }
        let identity_matches = configuration.credential_identity_matches(&current);
        if !identity_matches && api_key.is_none() {
            return Err(SettingsError::MissingCredential.into());
        }
        let next_version = profile
            .current_version
            .checked_add(1)
            .ok_or_else(|| validation("AI model profile version overflow"))?;
        if profile.meta.revision == i64::MAX {
            return Err(validation("AI model profile revision overflow"));
        }
        let now = Utc::now();
        profile.name = name;
        profile.current_version = next_version;
        profile.meta.touch(now);
        let version = configuration.version(profile.id, next_version, now);
        self.model_profile_settings()
            .append_model_profile_with_secret(
                self.domain_store(),
                AppendModelProfileWithSecret {
                    profile: &profile,
                    version: &version,
                    expected_revision,
                    api_key,
                    preserve_from: identity_matches.then_some(current_binding),
                },
                &audit,
            )
            .await?;
        self.get_ai_model_profile(profile.id).await
    }

    pub(crate) async fn validate_ai_model_profile(
        &self,
        input: ValidateAiModelProfileInput,
    ) -> Result<AiModelValidationResult, DesktopError> {
        if input.profile_id.is_some() != input.current_version.is_some() {
            return Err(validation(
                "profileId and currentVersion must be provided together",
            ));
        }
        let configuration = ModelProfileConfiguration::from_validation(&input)?;
        let explicit_key = validated_api_key(input.api_key.as_deref())?;
        let stored_key = if explicit_key.is_none() {
            self.validation_profile_key(&input, &configuration).await?
        } else {
            None
        };
        let api_key = explicit_key.or_else(|| stored_key.as_ref().map(AiSecret::as_str));
        if configuration.transport == AiProviderTransport::OpenAiCompatible && api_key.is_none() {
            return Ok(AiModelValidationResult {
                ok: false,
                latency_ms: 0,
                error_code: Some("missing_credential"),
            });
        }
        let provider = BuiltinProvider::from_config(
            configuration.provider_config("desktop-unsaved-model-validation"),
        )
        .map_err(SettingsError::InvalidProvider)?;
        let credentials = api_key
            .map(ProviderCredentials::bearer)
            .transpose()
            .map_err(|_| SettingsError::InvalidCredential)?
            .unwrap_or_else(ProviderCredentials::none);
        let mut request = CompletionRequest::new(vec![ChatMessage::user("Reply with exactly OK.")]);
        request.temperature = Some(0.0);
        request.max_output_tokens = Some(8);
        let started = Instant::now();
        match provider.complete(request, credentials).await {
            Ok(_) => Ok(AiModelValidationResult {
                ok: true,
                latency_ms: started.elapsed().as_millis(),
                error_code: None,
            }),
            Err(error) => Ok(AiModelValidationResult {
                ok: false,
                latency_ms: started.elapsed().as_millis(),
                error_code: Some(provider_error_code(&error)),
            }),
        }
    }

    pub(crate) async fn clear_ai_model_profile_key(
        &self,
        id: Uuid,
    ) -> Result<AiModelProfileView, DesktopError> {
        let _operation = self
            .model_profile_settings()
            .profile_coordinator()
            .lock()
            .await;
        let profile = self.owned_model_profile(id).await?;
        let audit = self.audit("clear_ai_model_profile_key").await?;
        self.model_profile_settings()
            .clear_model_profile_secrets(
                self.domain_store(),
                AiModelProfileBinding {
                    profile_id: profile.id,
                    profile_version: profile.current_version,
                },
                &audit,
            )
            .await?;
        self.get_ai_model_profile(profile.id).await
    }

    pub(crate) async fn archive_ai_model_profile(
        &self,
        id: Uuid,
        expected_revision: i64,
    ) -> Result<AiModelProfileView, DesktopError> {
        let _operation = self
            .model_profile_settings()
            .profile_coordinator()
            .lock()
            .await;
        let mut profile = self.owned_model_profile(id).await?;
        if profile.archived_at.is_some() {
            return Err(
                StoreError::Conflict("AI model profile is already archived".to_owned()).into(),
            );
        }
        if profile.meta.revision != expected_revision {
            return Err(
                StoreError::Conflict("AI model profile changed concurrently".to_owned()).into(),
            );
        }
        let audit = self.audit("archive_ai_model_profile").await?;
        if profile.meta.revision == i64::MAX {
            return Err(validation("AI model profile revision overflow"));
        }
        let now = Utc::now();
        profile.archived_at = Some(now);
        profile.meta.touch(now);
        self.domain_store()
            .archive_ai_model_profile(&profile, expected_revision, &audit)
            .await?;
        self.get_ai_model_profile(profile.id).await
    }

    pub(crate) async fn get_ai_model_defaults(&self) -> Result<AiModelDefaultsView, DesktopError> {
        let defaults = self
            .domain_store()
            .get_ai_user_model_defaults(self.local_user_id())
            .await?;
        Ok(defaults_view(defaults.as_ref()))
    }

    pub(crate) async fn save_ai_model_defaults(
        &self,
        input: SaveAiModelDefaultsInput,
    ) -> Result<AiModelDefaultsView, DesktopError> {
        let _operation = self
            .model_profile_settings()
            .profile_coordinator()
            .lock()
            .await;
        let current = self
            .domain_store()
            .get_ai_user_model_defaults(self.local_user_id())
            .await?;
        let current_revision = current.as_ref().map_or(0, |value| value.meta.revision);
        if input.expected_revision != current_revision {
            return Err(
                StoreError::Conflict("AI model defaults changed concurrently".to_owned()).into(),
            );
        }
        if current_revision == i64::MAX {
            return Err(validation("AI model defaults revision overflow"));
        }
        let now = Utc::now();
        let mut defaults = current.unwrap_or_else(|| AiUserModelDefaults {
            user_id: self.local_user_id(),
            default_conversation_profile_id: None,
            default_vision_profile_id: None,
            meta: RecordMeta::new(now),
        });
        defaults.default_conversation_profile_id = input.default_conversation_profile_id;
        defaults.default_vision_profile_id = input.default_vision_profile_id;
        let expected_revision = (current_revision > 0).then_some(current_revision);
        if expected_revision.is_some() {
            defaults.meta.touch(now);
        }
        let audit = self.audit("save_ai_model_defaults").await?;
        self.domain_store()
            .save_ai_user_model_defaults(&defaults, expected_revision, &audit)
            .await?;
        Ok(defaults_view(Some(&defaults)))
    }

    async fn owned_model_profile(&self, id: Uuid) -> Result<AiModelProfile, DesktopError> {
        let profile = self.domain_store().get_ai_model_profile(id).await?;
        if profile.lab_id != self.local_lab_id()
            || profile.user_id != self.local_user_id()
            || profile.meta.deleted_at.is_some()
        {
            return Err(StoreError::NotFound {
                entity: "ai_model_profile",
                id,
            }
            .into());
        }
        Ok(profile)
    }

    async fn model_profile_view(
        &self,
        profile: AiModelProfile,
        defaults: Option<&AiUserModelDefaults>,
    ) -> Result<AiModelProfileView, DesktopError> {
        let version = self
            .domain_store()
            .get_ai_model_profile_version(profile.id, profile.current_version)
            .await?;
        let binding = AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: profile.current_version,
        };
        Ok(AiModelProfileView {
            id: profile.id,
            name: profile.name,
            current_version: profile.current_version,
            archived_at: profile.archived_at,
            revision: profile.meta.revision,
            protocol: version.protocol,
            transport: version.transport,
            base_url: version.base_url,
            model_id: version.model_id,
            supports_vision: version.supports_vision,
            context_window_tokens: version.context_window_tokens,
            max_input_tokens: version.max_input_tokens,
            max_output_tokens: version.max_output_tokens,
            history_token_budget: version.history_token_budget,
            history_turns: version.history_turns,
            temperature: version.temperature,
            timeout_ms: version.timeout_ms,
            has_key: self
                .model_profile_settings()
                .profile_has_key(self.domain_store(), binding)
                .await?,
            is_default_conversation: defaults
                .is_some_and(|value| value.default_conversation_profile_id == Some(profile.id)),
            is_default_vision: defaults
                .is_some_and(|value| value.default_vision_profile_id == Some(profile.id)),
            created_at: profile.meta.created_at,
            updated_at: profile.meta.updated_at,
        })
    }

    async fn validation_profile_key(
        &self,
        input: &ValidateAiModelProfileInput,
        configuration: &ModelProfileConfiguration,
    ) -> Result<Option<AiSecret>, DesktopError> {
        let (Some(profile_id), Some(profile_version)) = (input.profile_id, input.current_version)
        else {
            if input.profile_id.is_some() || input.current_version.is_some() {
                return Err(validation(
                    "profileId and currentVersion must be provided together",
                ));
            }
            return Ok(None);
        };
        let profile = self.owned_model_profile(profile_id).await?;
        if profile.archived_at.is_some() || profile.current_version != profile_version {
            return Ok(None);
        }
        let current = self
            .domain_store()
            .get_ai_model_profile_version(profile_id, profile_version)
            .await?;
        if !configuration.credential_identity_matches(&current) {
            return Ok(None);
        }
        self.model_profile_settings()
            .profile_key(
                self.domain_store(),
                AiModelProfileBinding {
                    profile_id,
                    profile_version,
                },
            )
            .await
            .map_err(Into::into)
    }
}

fn defaults_view(defaults: Option<&AiUserModelDefaults>) -> AiModelDefaultsView {
    AiModelDefaultsView {
        default_conversation_profile_id: defaults
            .and_then(|value| value.default_conversation_profile_id),
        default_vision_profile_id: defaults.and_then(|value| value.default_vision_profile_id),
        revision: defaults.map_or(0, |value| value.meta.revision),
    }
}

fn validated_name(value: &str) -> Result<String, DesktopError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 120 || value.chars().any(char::is_control) {
        return Err(validation("AI model profile name is invalid"));
    }
    Ok(value.to_owned())
}

fn validated_api_key(value: Option<&str>) -> Result<Option<&str>, DesktopError> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    value
        .map(ProviderCredentials::bearer)
        .transpose()
        .map_err(|_| SettingsError::InvalidCredential)?;
    Ok(value)
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn validation(message: &str) -> DesktopError {
    StoreError::Validation(message.to_owned()).into()
}

fn provider_error_code(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::InvalidConfig(_) | ProviderError::InvalidRequest(_) => "invalid_provider",
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    use tempfile::tempdir;

    use super::*;

    fn save_input(
        protocol: AiProviderProtocol,
        transport: AiProviderTransport,
        base_url: impl Into<String>,
        model_id: impl Into<String>,
        supports_vision: bool,
        api_key: Option<&str>,
        expected_revision: Option<i64>,
    ) -> SaveAiModelProfileInput {
        SaveAiModelProfileInput {
            name: "Lab model".to_owned(),
            protocol,
            transport,
            base_url: base_url.into(),
            model_id: model_id.into(),
            supports_vision,
            context_window_tokens: 32_768,
            max_input_tokens: 24_576,
            max_output_tokens: 4_096,
            history_token_budget: 12_288,
            history_turns: 12,
            temperature: 0.0,
            timeout_ms: 2_000,
            api_key: api_key.map(str::to_owned),
            expected_revision,
        }
    }

    async fn state() -> (tempfile::TempDir, DesktopState) {
        let directory = tempdir().unwrap();
        let state = DesktopState::initialize(directory.path().join("muriarc.sqlite3"))
            .await
            .unwrap();
        (directory, state)
    }

    #[tokio::test]
    async fn desktop_profiles_append_versions_and_enforce_credential_identity() {
        let (_directory, state) = state().await;
        let created = state
            .create_ai_model_profile(save_input(
                AiProviderProtocol::OpenaiChatCompletions,
                AiProviderTransport::OpenAiCompatible,
                "https://models.example.test/v1",
                "lab-chat",
                false,
                Some("secret-one"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(created.current_version, 1);
        assert!(created.has_key);

        let updated = state
            .update_ai_model_profile(
                created.id,
                save_input(
                    AiProviderProtocol::OpenaiChatCompletions,
                    AiProviderTransport::OpenAiCompatible,
                    "https://models.example.test/v1/",
                    "自由模型标识-二",
                    false,
                    None,
                    Some(created.revision),
                ),
            )
            .await
            .unwrap();
        assert_eq!(updated.current_version, 2);
        assert!(updated.has_key);

        let rejected = state
            .update_ai_model_profile(
                created.id,
                save_input(
                    AiProviderProtocol::OpenaiResponses,
                    AiProviderTransport::OpenAiCompatible,
                    "https://models.example.test/v1",
                    "自由模型标识-二",
                    false,
                    None,
                    Some(updated.revision),
                ),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            rejected,
            DesktopError::Settings(SettingsError::MissingCredential)
        ));

        let changed_protocol = state
            .update_ai_model_profile(
                created.id,
                save_input(
                    AiProviderProtocol::OpenaiResponses,
                    AiProviderTransport::OpenAiCompatible,
                    "https://models.example.test/v1",
                    "自由模型标识-二",
                    false,
                    Some("secret-two"),
                    Some(updated.revision),
                ),
            )
            .await
            .unwrap();
        assert_eq!(changed_protocol.current_version, 3);
        assert_eq!(
            changed_protocol.protocol,
            AiProviderProtocol::OpenaiResponses
        );

        let cleared = state.clear_ai_model_profile_key(created.id).await.unwrap();
        assert!(!cleared.has_key);
        for version in 1..=cleared.current_version {
            assert!(
                state
                    .model_profile_settings()
                    .profile_key(
                        state.domain_store(),
                        AiModelProfileBinding {
                            profile_id: cleared.id,
                            profile_version: version,
                        },
                    )
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn defaults_and_archive_keep_one_explicit_vision_default() {
        let (_directory, state) = state().await;
        let created = state
            .create_ai_model_profile(save_input(
                AiProviderProtocol::AnthropicMessages,
                AiProviderTransport::LocalHttp,
                "http://127.0.0.1:11434/v1",
                "vision-local",
                true,
                None,
                None,
            ))
            .await
            .unwrap();
        let before = state.get_ai_model_defaults().await.unwrap();
        let saved = state
            .save_ai_model_defaults(SaveAiModelDefaultsInput {
                default_conversation_profile_id: Some(created.id),
                default_vision_profile_id: Some(created.id),
                expected_revision: before.revision,
            })
            .await
            .unwrap();
        assert_eq!(saved.default_conversation_profile_id, Some(created.id));
        assert_eq!(saved.default_vision_profile_id, Some(created.id));

        let archived = state
            .archive_ai_model_profile(created.id, created.revision)
            .await
            .unwrap();
        assert!(archived.archived_at.is_some());
        assert!(!archived.is_default_conversation);
        assert!(!archived.is_default_vision);
        let after = state.get_ai_model_defaults().await.unwrap();
        assert_eq!(after.default_conversation_profile_id, None);
        assert_eq!(after.default_vision_profile_id, None);
        assert!(
            state
                .list_ai_model_profiles()
                .await
                .unwrap()
                .iter()
                .all(|profile| profile.id != created.id)
        );
    }

    #[tokio::test]
    async fn unsaved_validation_calls_provider_without_persisting_profile() {
        let (_directory, state) = state().await;
        let before = state.list_ai_model_profiles().await.unwrap();
        let (base_url, server) = spawn_chat_server();
        let result = state
            .validate_ai_model_profile(ValidateAiModelProfileInput {
                protocol: AiProviderProtocol::OpenaiChatCompletions,
                transport: AiProviderTransport::LocalHttp,
                base_url,
                model_id: "unsaved-local".to_owned(),
                supports_vision: false,
                context_window_tokens: 16_384,
                max_input_tokens: 12_288,
                max_output_tokens: 2_048,
                history_token_budget: 8_192,
                history_turns: 8,
                temperature: 0.0,
                timeout_ms: 2_000,
                api_key: None,
                profile_id: None,
                current_version: None,
            })
            .await
            .unwrap();
        server.join().unwrap();
        assert!(result.ok);
        assert_eq!(state.list_ai_model_profiles().await.unwrap(), before);
    }

    #[tokio::test]
    async fn validation_requires_profile_locator_fields_as_a_pair_even_with_an_explicit_key() {
        let (_directory, state) = state().await;
        let error = state
            .validate_ai_model_profile(ValidateAiModelProfileInput {
                protocol: AiProviderProtocol::OpenaiChatCompletions,
                transport: AiProviderTransport::OpenAiCompatible,
                base_url: "https://provider.example.test/v1".to_owned(),
                model_id: "paired-locator".to_owned(),
                supports_vision: false,
                context_window_tokens: 16_384,
                max_input_tokens: 12_288,
                max_output_tokens: 2_048,
                history_token_budget: 8_192,
                history_turns: 8,
                temperature: 0.0,
                timeout_ms: 2_000,
                api_key: Some("explicit-secret".to_owned()),
                profile_id: Some(Uuid::new_v4()),
                current_version: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation");
    }

    #[test]
    fn validation_provider_errors_match_the_server_contract() {
        for status in [401, 403] {
            assert_eq!(
                provider_error_code(&ProviderError::HttpStatus {
                    status,
                    request_id: None,
                }),
                "api_key_rejected"
            );
        }
        assert_eq!(
            provider_error_code(&ProviderError::HttpStatus {
                status: 404,
                request_id: None,
            }),
            "model_not_found"
        );
        assert_eq!(
            provider_error_code(&ProviderError::HttpStatus {
                status: 500,
                request_id: None,
            }),
            "provider_http_error"
        );
        assert_eq!(
            provider_error_code(&ProviderError::RequestTooLarge { limit: 1024 }),
            "context_exceeded"
        );
    }

    fn spawn_chat_server() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if complete_http_request(&request) {
                    break;
                }
            }
            assert!(
                String::from_utf8_lossy(&request).starts_with("POST /v1/chat/completions HTTP/1.1")
            );
            let body = serde_json::json!({
                "id": "validation",
                "model": "unsaved-local",
                "choices": [{
                    "message": {"content": "OK", "tool_calls": []},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 4, "completion_tokens": 1, "total_tokens": 5}
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{address}/v1"), handle)
    }

    fn complete_http_request(request: &[u8]) -> bool {
        let Some(header_end) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
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
        request.len() >= header_end + 4 + content_length
    }
}
