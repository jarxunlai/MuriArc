use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use muriarc_ai::{
    AssistantRuntimeConfig, BuiltinProvider, ProviderConfig, ProviderConfigError,
    ProviderCredentials, ProviderKind,
};
use muriarc_core::{
    AiModelCredentialState, AiModelProfile, AiModelProfileBinding, AiModelProfileSecretRef,
    AiModelProfileSecretRefStore, AiModelProfileStore, AiModelProfileVersion, AiProviderProtocol,
    AiProviderTransport, AiUserModelDefaults, AuditContext, LOCAL_LAB_ID, LOCAL_USER_ID,
    RecordMeta, StoreError,
};
use serde::{Deserialize, Serialize};
use tauri::async_runtime::Mutex as AsyncMutex;
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

const SETTINGS_SCHEMA_VERSION: u32 = 2;
const PROVIDER_ID: &str = "desktop-user-provider";
const KEYRING_SERVICE: &str = concat!(env!("MURIARC_BUNDLE_IDENTIFIER"), ".ai");
const LEGACY_KEYRING_ACCOUNT: &str = "local-user-provider-api-key";
const PROFILE_KEYRING_ACCOUNT_PREFIX: &str = "local-user-model-profile";
pub(crate) const MIGRATED_LOCAL_PROFILE_ID: Uuid = LOCAL_USER_ID;
pub(crate) const MIGRATED_LOCAL_VISION_PROFILE_ID: Uuid =
    Uuid::from_u128(0x4d55_5249_4152_f300_0000_0000_0000_0002);

#[derive(Debug, Error)]
pub(crate) enum SettingsError {
    #[error("AI provider configuration is invalid")]
    InvalidProvider(#[source] ProviderConfigError),
    #[error("AI provider credential is invalid")]
    InvalidCredential,
    #[error("settings file is not valid MuriArc configuration")]
    InvalidFile,
    #[error("settings storage is unavailable")]
    Storage,
    #[error("OS credential store is unavailable")]
    CredentialStore,
    #[error("AI assistant is disabled")]
    Disabled,
    #[error("the selected cloud provider requires an API key")]
    MissingCredential,
    #[error("AI model profile storage is unavailable")]
    ModelProfileStore(#[from] StoreError),
    #[error("this Desktop build does not support the model profile protocol")]
    UnsupportedProtocol,
}

impl SettingsError {
    pub(crate) fn is_validation(&self) -> bool {
        matches!(
            self,
            Self::InvalidProvider(_)
                | Self::InvalidCredential
                | Self::InvalidFile
                | Self::Disabled
                | Self::MissingCredential
                | Self::UnsupportedProtocol
        )
    }
}

pub(crate) trait SecretStore: Send + Sync {
    fn get_secret(&self) -> Result<Option<String>, SettingsError>;
    fn contains_secret_entry(&self) -> Result<bool, SettingsError> {
        Ok(self.get_secret()?.is_some())
    }
    fn set_secret(&self, secret: &str) -> Result<(), SettingsError>;
    fn clear_secret(&self) -> Result<(), SettingsError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KeyringSecretStore {
    profile_id: Option<Uuid>,
    profile_version: Option<i64>,
    migrate_legacy: bool,
}

impl KeyringSecretStore {
    pub(crate) fn for_profile(profile_id: Uuid) -> Self {
        Self {
            profile_id: Some(profile_id),
            profile_version: None,
            migrate_legacy: matches!(
                profile_id,
                MIGRATED_LOCAL_PROFILE_ID | MIGRATED_LOCAL_VISION_PROFILE_ID
            ),
        }
    }

    pub(crate) fn for_profile_version(profile_id: Uuid, profile_version: i64) -> Self {
        Self {
            profile_id: Some(profile_id),
            profile_version: Some(profile_version),
            migrate_legacy: false,
        }
    }

    fn account(self) -> String {
        match (self.profile_id, self.profile_version) {
            (Some(profile_id), Some(profile_version)) => {
                format!("{PROFILE_KEYRING_ACCOUNT_PREFIX}-{profile_id}-v{profile_version}-api-key")
            }
            (Some(profile_id), None) => {
                format!("{PROFILE_KEYRING_ACCOUNT_PREFIX}-{profile_id}-api-key")
            }
            (None, None) => LEGACY_KEYRING_ACCOUNT.to_owned(),
            (None, Some(_)) => unreachable!("a profile version always has a profile"),
        }
    }

    fn selected_entry(self) -> Result<keyring::Entry, SettingsError> {
        keyring::Entry::new(KEYRING_SERVICE, &self.account())
            .map_err(|_| SettingsError::CredentialStore)
    }

    /// Copies the legacy single-provider credential into the deterministic
    /// migrated profile's own keyring item when that item does not exist.
    ///
    /// The legacy item is deliberately retained. Callers must invoke this only
    /// for the deterministic profile created from the legacy desktop settings;
    /// ordinary new profiles must never inherit the legacy credential.
    pub(crate) fn migrate_legacy_secret(&self) -> Result<bool, SettingsError> {
        if !self.migrate_legacy {
            return Ok(false);
        }
        copy_secret_if_missing(&Self::default(), self)
    }
}

impl SecretStore for KeyringSecretStore {
    fn get_secret(&self) -> Result<Option<String>, SettingsError> {
        if self.migrate_legacy && !self.contains_secret_entry()? {
            self.migrate_legacy_secret()?;
        }
        match self.selected_entry()?.get_password() {
            Ok(secret) if secret.is_empty() => Ok(None),
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(SettingsError::CredentialStore),
        }
    }

    fn contains_secret_entry(&self) -> Result<bool, SettingsError> {
        match self.selected_entry()?.get_password() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(SettingsError::CredentialStore),
        }
    }

    fn set_secret(&self, secret: &str) -> Result<(), SettingsError> {
        self.selected_entry()?
            .set_password(secret)
            .map_err(|_| SettingsError::CredentialStore)
    }

    fn clear_secret(&self) -> Result<(), SettingsError> {
        if self.migrate_legacy {
            return self
                .selected_entry()?
                .set_password("")
                .map_err(|_| SettingsError::CredentialStore);
        }
        match self.selected_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(SettingsError::CredentialStore),
        }
    }
}

fn copy_secret_if_missing(
    source: &dyn SecretStore,
    destination: &dyn SecretStore,
) -> Result<bool, SettingsError> {
    if destination.contains_secret_entry()? {
        return Ok(false);
    }
    let Some(secret) = source
        .get_secret()?
        .filter(|secret| !secret.is_empty())
        .map(Zeroizing::new)
    else {
        return Ok(false);
    };
    destination.set_secret(secret.as_str())?;
    Ok(true)
}

pub(crate) trait VersionedSecretStore: Send + Sync {
    fn get_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<Option<String>, SettingsError>;
    fn contains_secret_entry(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<bool, SettingsError>;
    fn set_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
        secret: &str,
    ) -> Result<(), SettingsError>;
    fn clear_secret(&self, profile_id: Uuid, profile_version: i64) -> Result<(), SettingsError>;
}

#[cfg(not(test))]
#[derive(Debug, Clone, Copy, Default)]
struct KeyringVersionedSecretStore;

#[cfg(not(test))]
impl VersionedSecretStore for KeyringVersionedSecretStore {
    fn get_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<Option<String>, SettingsError> {
        KeyringSecretStore::for_profile_version(profile_id, profile_version).get_secret()
    }

    fn contains_secret_entry(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<bool, SettingsError> {
        KeyringSecretStore::for_profile_version(profile_id, profile_version).contains_secret_entry()
    }

    fn set_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
        secret: &str,
    ) -> Result<(), SettingsError> {
        KeyringSecretStore::for_profile_version(profile_id, profile_version).set_secret(secret)
    }

    fn clear_secret(&self, profile_id: Uuid, profile_version: i64) -> Result<(), SettingsError> {
        // Keep an explicit empty entry so a later compatibility migration
        // cannot restore a credential that the user deliberately revoked.
        KeyringSecretStore::for_profile_version(profile_id, profile_version).set_secret("")
    }
}

#[cfg(test)]
#[derive(Default)]
struct EphemeralSecretStore(Mutex<Option<String>>);

#[cfg(test)]
impl SecretStore for EphemeralSecretStore {
    fn get_secret(&self) -> Result<Option<String>, SettingsError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SettingsError::CredentialStore)?
            .clone())
    }

    fn set_secret(&self, secret: &str) -> Result<(), SettingsError> {
        *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = Some(secret.to_owned());
        Ok(())
    }

    fn clear_secret(&self) -> Result<(), SettingsError> {
        *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = None;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Default)]
struct EphemeralVersionedSecretStore(Mutex<std::collections::BTreeMap<(Uuid, i64), String>>);

#[cfg(test)]
impl VersionedSecretStore for EphemeralVersionedSecretStore {
    fn get_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<Option<String>, SettingsError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SettingsError::CredentialStore)?
            .get(&(profile_id, profile_version))
            .filter(|secret| !secret.is_empty())
            .cloned())
    }

    fn contains_secret_entry(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> Result<bool, SettingsError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SettingsError::CredentialStore)?
            .contains_key(&(profile_id, profile_version)))
    }

    fn set_secret(
        &self,
        profile_id: Uuid,
        profile_version: i64,
        secret: &str,
    ) -> Result<(), SettingsError> {
        self.0
            .lock()
            .map_err(|_| SettingsError::CredentialStore)?
            .insert((profile_id, profile_version), secret.to_owned());
        Ok(())
    }

    fn clear_secret(&self, profile_id: Uuid, profile_version: i64) -> Result<(), SettingsError> {
        self.set_secret(profile_id, profile_version, "")
    }
}

#[derive(Clone)]
pub(crate) struct SettingsService {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    vision_secrets: Arc<dyn SecretStore>,
    versioned_secrets: Arc<dyn VersionedSecretStore>,
    write_lock: Arc<Mutex<()>>,
    operation_lock: Arc<AsyncMutex<()>>,
}

impl fmt::Debug for SettingsService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SettingsService")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SettingsService {
    #[cfg(not(test))]
    pub(crate) fn for_app_data(app_data_dir: &Path) -> Self {
        Self::new_with_profile_secrets(
            app_data_dir.join("ai-provider.json"),
            Arc::new(KeyringSecretStore::for_profile(MIGRATED_LOCAL_PROFILE_ID)),
            Arc::new(KeyringSecretStore::for_profile(
                MIGRATED_LOCAL_VISION_PROFILE_ID,
            )),
            Arc::new(KeyringVersionedSecretStore),
        )
    }

    #[cfg(test)]
    pub(crate) fn for_app_data(app_data_dir: &Path) -> Self {
        Self::new_with_profile_secrets(
            app_data_dir.join("ai-provider.json"),
            Arc::new(EphemeralSecretStore::default()),
            Arc::new(EphemeralSecretStore::default()),
            Arc::new(EphemeralVersionedSecretStore::default()),
        )
    }

    #[cfg(test)]
    pub(crate) fn new(
        path: PathBuf,
        secrets: Arc<dyn SecretStore>,
        versioned_secrets: Arc<dyn VersionedSecretStore>,
    ) -> Self {
        Self::new_with_profile_secrets(path, secrets.clone(), secrets, versioned_secrets)
    }

    fn new_with_profile_secrets(
        path: PathBuf,
        secrets: Arc<dyn SecretStore>,
        vision_secrets: Arc<dyn SecretStore>,
        versioned_secrets: Arc<dyn VersionedSecretStore>,
    ) -> Self {
        Self {
            path,
            secrets,
            vision_secrets,
            versioned_secrets,
            write_lock: Arc::new(Mutex::new(())),
            operation_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    /// Projects the legacy Desktop JSON settings into versioned SQLite model
    /// profiles without reading or persisting the API key.
    ///
    /// Repeated calls are idempotent. When non-sensitive settings change, one
    /// immutable profile version is appended and the previous version remains
    /// available to historical conversations.
    pub(crate) async fn materialize_model_profiles<
        S: AiModelProfileStore + AiModelProfileSecretRefStore + ?Sized,
    >(
        &self,
        store: &S,
        audit: &AuditContext,
    ) -> Result<AiModelProfileBinding, SettingsError> {
        let _operation = self.operation_lock.lock().await;
        self.materialize_model_profiles_locked(store, audit, false, false)
            .await
    }

    async fn materialize_model_profiles_locked<
        S: AiModelProfileStore + AiModelProfileSecretRefStore + ?Sized,
    >(
        &self,
        store: &S,
        audit: &AuditContext,
        force_text_credential_revision: bool,
        force_vision_credential_revision: bool,
    ) -> Result<AiModelProfileBinding, SettingsError> {
        let settings = self.read_or_default()?;
        let runtime = settings.runtime()?;
        let vision_model = settings
            .supports_vision
            .then(|| {
                settings
                    .vision_model
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or(SettingsError::InvalidFile)
            })
            .transpose()?;
        if let Some(model_id) = &vision_model {
            let mut vision_provider = settings.provider.clone();
            vision_provider.model = model_id.clone();
            BuiltinProvider::from_config(vision_provider)
                .map_err(SettingsError::InvalidProvider)?;
        }
        let now = Utc::now();
        let text_spec = ModelProfileSpec {
            id: MIGRATED_LOCAL_PROFILE_ID,
            name: "Migrated default model",
            transport: provider_transport(settings.provider.kind),
            model_id: settings.provider.model.clone(),
            supports_vision: false,
            base_url: settings.provider.base_url.clone(),
            runtime,
        };
        let text_binding = materialize_profile(store, &text_spec, now, audit).await?;

        let configured_vision_binding = if let Some(model_id) = vision_model {
            let vision_spec = ModelProfileSpec {
                id: MIGRATED_LOCAL_VISION_PROFILE_ID,
                name: "Migrated vision model",
                transport: provider_transport(settings.provider.kind),
                model_id,
                supports_vision: true,
                base_url: settings.provider.base_url.clone(),
                runtime,
            };
            Some(materialize_profile(store, &vision_spec, now, audit).await?)
        } else {
            None
        };

        materialize_defaults(
            store,
            text_binding.profile_id,
            configured_vision_binding.map(|binding| binding.profile_id),
            now,
            audit,
        )
        .await?;
        let retained_vision_binding = match configured_vision_binding {
            Some(binding) => Some(binding),
            None => {
                existing_compatible_profile_binding(
                    store,
                    MIGRATED_LOCAL_VISION_PROFILE_ID,
                    "Migrated vision model",
                )
                .await?
            }
        };
        let copied_text = self
            .ensure_profile_version_secret(store, text_binding, self.secrets.as_ref())
            .await?;
        if let Some(binding) = retained_vision_binding {
            let copied_vision = self
                .ensure_profile_version_secret(store, binding, self.vision_secrets.as_ref())
                .await?;
            if let Err(error) = self
                .reconcile_profile_secret_refs(
                    store,
                    binding,
                    force_vision_credential_revision,
                    audit,
                )
                .await
            {
                self.compensate_copied_secret_entries(&copied_vision);
                self.compensate_copied_secret_entries(&copied_text);
                return Err(error);
            }
        }
        if let Err(error) = self
            .reconcile_profile_secret_refs(
                store,
                text_binding,
                force_text_credential_revision,
                audit,
            )
            .await
        {
            self.compensate_copied_secret_entries(&copied_text);
            return Err(error);
        }
        self.sync_materialized_versions(text_binding, retained_vision_binding)?;
        Ok(text_binding)
    }

    pub(crate) fn get(&self) -> Result<AiSettingsView, SettingsError> {
        let settings = self.read_or_default()?;
        let has_key = self
            .versioned_secrets
            .get_secret(
                MIGRATED_LOCAL_PROFILE_ID,
                settings.conversation_profile_version.max(1),
            )?
            .is_some();
        Ok(AiSettingsView::from_file(settings, has_key))
    }

    pub(crate) async fn save_and_materialize<
        S: AiModelProfileStore + AiModelProfileSecretRefStore + ?Sized,
    >(
        &self,
        store: &S,
        input: SaveAiSettingsInput,
        audit: &AuditContext,
    ) -> Result<AiSettingsView, SettingsError> {
        let _operation = self.operation_lock.lock().await;
        let before = self.read_or_default()?;
        let force_text_credential_revision = input.api_key.is_some();
        let force_vision_credential_revision =
            force_text_credential_revision && input.supports_vision;
        let saved = match self.save(input) {
            Ok(saved) => saved,
            Err(error) => {
                if matches!(
                    &error,
                    SettingsError::Storage | SettingsError::CredentialStore
                ) {
                    self.fail_closed_after_save_error(&before);
                }
                return Err(error);
            }
        };
        if let Err(error) = self
            .materialize_model_profiles_locked(
                store,
                audit,
                force_text_credential_revision,
                force_vision_credential_revision,
            )
            .await
        {
            let current = self.read_or_default().unwrap_or(before);
            self.fail_closed_after_save_error(&current);
            return Err(error);
        }
        Ok(saved)
    }

    pub(crate) fn save(&self, input: SaveAiSettingsInput) -> Result<AiSettingsView, SettingsError> {
        let runtime = input.runtime()?;
        let provider_preset_id = validated_preset_id(&input.provider_preset_id)?;
        let mut provider = match input.provider_kind {
            ProviderKind::OpenAiCompatible => ProviderConfig::openai_compatible(
                PROVIDER_ID,
                input.model.clone(),
                input.base_url.clone(),
            ),
            ProviderKind::LocalHttp => {
                ProviderConfig::local_http(PROVIDER_ID, input.model.clone(), input.base_url.clone())
            }
        };
        provider.timeout_ms = runtime.timeout_ms;
        BuiltinProvider::from_config(provider.clone()).map_err(SettingsError::InvalidProvider)?;
        if let Some(secret) = input.api_key.as_deref() {
            ProviderCredentials::bearer(secret).map_err(|_| SettingsError::InvalidCredential)?;
        }
        let vision_model = input.supports_vision.then(|| {
            input
                .vision_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or(SettingsError::InvalidFile)
        });
        let vision_model = vision_model.transpose()?;
        if input.supports_vision {
            let mut vision_provider = provider.clone();
            vision_provider.model = vision_model.clone().ok_or(SettingsError::InvalidFile)?;
            BuiltinProvider::from_config(vision_provider)
                .map_err(SettingsError::InvalidProvider)?;
        }

        let _guard = self.write_lock.lock().map_err(|_| SettingsError::Storage)?;
        let current = self.read_or_default()?;
        let identity_matches = current.provider.kind == provider.kind
            && normalized_url(&current.provider.base_url) == normalized_url(&provider.base_url);
        let text_profile_changed =
            !settings_profile_matches(&current, &provider, &runtime, &provider.model, false);
        let current_text_version = current.conversation_profile_version.max(1);
        let next_text_version = if text_profile_changed {
            current_text_version
                .checked_add(1)
                .ok_or(SettingsError::InvalidFile)?
        } else {
            current_text_version
        };
        let vision_profile_changed = input.supports_vision
            && (!current.supports_vision
                || !settings_profile_matches(
                    &current,
                    &provider,
                    &runtime,
                    vision_model.as_deref().ok_or(SettingsError::InvalidFile)?,
                    true,
                ));
        let current_vision_version = current.vision_profile_version.max(0);
        let next_vision_version = if input.supports_vision {
            if current_vision_version == 0 {
                1
            } else if vision_profile_changed {
                current_vision_version
                    .checked_add(1)
                    .ok_or(SettingsError::InvalidFile)?
            } else {
                current_vision_version
            }
        } else {
            current_vision_version
        };
        if let Some(secret) = input.api_key.as_deref() {
            self.versioned_secrets.set_secret(
                MIGRATED_LOCAL_PROFILE_ID,
                next_text_version,
                secret,
            )?;
            if input.supports_vision {
                self.versioned_secrets.set_secret(
                    MIGRATED_LOCAL_VISION_PROFILE_ID,
                    next_vision_version,
                    secret,
                )?;
            }
        } else if identity_matches {
            if next_text_version != current_text_version {
                self.copy_version_secret_if_missing(
                    MIGRATED_LOCAL_PROFILE_ID,
                    current_text_version,
                    MIGRATED_LOCAL_PROFILE_ID,
                    next_text_version,
                )?;
            }
            if input.supports_vision && next_vision_version != current_vision_version {
                if current.supports_vision && current_vision_version > 0 {
                    self.copy_version_secret_if_missing(
                        MIGRATED_LOCAL_VISION_PROFILE_ID,
                        current_vision_version,
                        MIGRATED_LOCAL_VISION_PROFILE_ID,
                        next_vision_version,
                    )?;
                } else {
                    self.copy_version_secret_if_missing(
                        MIGRATED_LOCAL_PROFILE_ID,
                        next_text_version,
                        MIGRATED_LOCAL_VISION_PROFILE_ID,
                        next_vision_version,
                    )?;
                }
            }
        } else {
            // An explicit empty entry prevents compatibility migration from
            // copying an old Provider credential into a new identity.
            self.versioned_secrets
                .set_secret(MIGRATED_LOCAL_PROFILE_ID, next_text_version, "")?;
            if input.supports_vision {
                self.versioned_secrets.set_secret(
                    MIGRATED_LOCAL_VISION_PROFILE_ID,
                    next_vision_version,
                    "",
                )?;
            }
        }
        let file = AiSettingsFile {
            schema_version: SETTINGS_SCHEMA_VERSION,
            enabled: input.enabled,
            provider,
            provider_preset_id,
            supports_vision: input.supports_vision,
            vision_model,
            context_window_tokens: runtime.context_window_tokens,
            max_input_tokens: runtime.max_input_tokens,
            max_output_tokens: runtime.max_output_tokens,
            history_token_budget: runtime.history_token_budget,
            history_turns: runtime.history_turns,
            temperature: runtime.temperature,
            timeout_ms: runtime.timeout_ms,
            conversation_profile_version: next_text_version,
            vision_profile_version: next_vision_version,
            revision: current.revision.saturating_add(1),
        };
        self.write_atomic(&file)?;
        let has_key = self
            .versioned_secrets
            .get_secret(MIGRATED_LOCAL_PROFILE_ID, next_text_version)?
            .is_some();
        Ok(AiSettingsView::from_file(file, has_key))
    }

    /// Resolves one immutable profile version into the exact Provider/runtime
    /// used for a turn. Historical conversations must use this path rather
    /// than rebuilding a Provider from the mutable JSON projection.
    pub(crate) async fn resolve_provider_for_profile<S: AiModelProfileStore + ?Sized>(
        &self,
        store: &S,
        binding: AiModelProfileBinding,
    ) -> Result<ResolvedAiProvider, SettingsError> {
        if !self.read_or_default()?.enabled {
            return Err(SettingsError::Disabled);
        }
        let profile = store.get_ai_model_profile(binding.profile_id).await?;
        if profile.lab_id != LOCAL_LAB_ID
            || profile.user_id != LOCAL_USER_ID
            || profile.archived_at.is_some()
            || profile.meta.deleted_at.is_some()
        {
            return Err(StoreError::Validation(
                "the selected local AI model profile is unavailable".to_owned(),
            )
            .into());
        }
        let version = store
            .get_ai_model_profile_version(binding.profile_id, binding.profile_version)
            .await?;
        if version.protocol != AiProviderProtocol::OpenaiChatCompletions {
            return Err(SettingsError::UnsupportedProtocol);
        }
        let runtime = AssistantRuntimeConfig {
            context_window_tokens: version.context_window_tokens,
            max_input_tokens: version.max_input_tokens,
            max_output_tokens: version.max_output_tokens,
            history_token_budget: version.history_token_budget,
            history_turns: version.history_turns,
            temperature: version.temperature,
            timeout_ms: version.timeout_ms,
        }
        .validate()
        .map_err(|_| SettingsError::InvalidFile)?;
        let mut config = match version.transport {
            AiProviderTransport::OpenAiCompatible => ProviderConfig::openai_compatible(
                format!("desktop-profile-{}", binding.profile_id.simple()),
                version.model_id.clone(),
                version.base_url.clone(),
            ),
            AiProviderTransport::LocalHttp => ProviderConfig::local_http(
                format!("desktop-profile-{}", binding.profile_id.simple()),
                version.model_id.clone(),
                version.base_url.clone(),
            ),
        };
        config.timeout_ms = version.timeout_ms;
        let provider =
            BuiltinProvider::from_config(config).map_err(SettingsError::InvalidProvider)?;
        if !matches!(
            binding.profile_id,
            MIGRATED_LOCAL_PROFILE_ID | MIGRATED_LOCAL_VISION_PROFILE_ID
        ) {
            return Err(StoreError::Validation(
                "the selected local AI model profile credential is unavailable".to_owned(),
            )
            .into());
        }
        let api_key = self
            .versioned_secrets
            .get_secret(binding.profile_id, binding.profile_version)?
            .map(AiSecret::new);
        if version.transport == AiProviderTransport::OpenAiCompatible && api_key.is_none() {
            return Err(SettingsError::MissingCredential);
        }
        Ok(ResolvedAiProvider {
            provider,
            api_key,
            runtime,
        })
    }

    pub(crate) fn clear_key(&self) -> Result<AiSettingsView, SettingsError> {
        let _guard = self.write_lock.lock().map_err(|_| SettingsError::Storage)?;
        let mut settings = self.read_or_default()?;
        self.secrets.clear_secret()?;
        self.vision_secrets.clear_secret()?;
        for profile_version in 1..=settings.conversation_profile_version.max(1) {
            self.versioned_secrets
                .clear_secret(MIGRATED_LOCAL_PROFILE_ID, profile_version)?;
        }
        for profile_version in 1..=settings.vision_profile_version.max(0) {
            self.versioned_secrets
                .clear_secret(MIGRATED_LOCAL_VISION_PROFILE_ID, profile_version)?;
        }
        settings.revision = settings.revision.saturating_add(1);
        self.write_atomic(&settings)?;
        Ok(AiSettingsView::from_file(settings, false))
    }

    pub(crate) async fn clear_key_with_metadata<
        S: AiModelProfileStore + AiModelProfileSecretRefStore + ?Sized,
    >(
        &self,
        store: &S,
        audit: &AuditContext,
    ) -> Result<AiSettingsView, SettingsError> {
        let _operation = self.operation_lock.lock().await;
        let text_binding = existing_compatible_profile_binding(
            store,
            MIGRATED_LOCAL_PROFILE_ID,
            "Migrated default model",
        )
        .await?;
        let vision_binding = existing_compatible_profile_binding(
            store,
            MIGRATED_LOCAL_VISION_PROFILE_ID,
            "Migrated vision model",
        )
        .await?;

        // Revoke first: if any later metadata write fails, no provider can
        // continue using a credential whose audit projection is stale.
        let view = self.clear_key()?;
        let text_binding = text_binding.ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: MIGRATED_LOCAL_PROFILE_ID,
        })?;
        self.reconcile_profile_secret_refs(store, text_binding, true, audit)
            .await?;
        if let Some(binding) = vision_binding {
            self.reconcile_profile_secret_refs(store, binding, true, audit)
                .await?;
        }
        Ok(view)
    }

    fn read_or_default(&self) -> Result<AiSettingsFile, SettingsError> {
        if !self.path.exists() {
            return Ok(AiSettingsFile::default());
        }
        let bytes = fs::read(&self.path).map_err(|_| SettingsError::Storage)?;
        let mut settings: AiSettingsFile =
            serde_json::from_slice(&bytes).map_err(|_| SettingsError::InvalidFile)?;
        if !matches!(settings.schema_version, 1 | SETTINGS_SCHEMA_VERSION) {
            return Err(SettingsError::InvalidFile);
        }
        if settings.schema_version == 1 {
            settings.provider_preset_id = infer_preset_id(&settings.provider.base_url).to_owned();
            settings.schema_version = SETTINGS_SCHEMA_VERSION;
        }
        if !settings.supports_vision {
            settings.vision_model = None;
        }
        if settings.conversation_profile_version < 1 || settings.vision_profile_version < 0 {
            return Err(SettingsError::InvalidFile);
        }
        settings.runtime()?;
        BuiltinProvider::from_config(settings.provider.clone())
            .map_err(SettingsError::InvalidProvider)?;
        Ok(settings)
    }

    fn write_atomic(&self, settings: &AiSettingsFile) -> Result<(), SettingsError> {
        let parent = self.path.parent().ok_or(SettingsError::Storage)?;
        fs::create_dir_all(parent).map_err(|_| SettingsError::Storage)?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|_| SettingsError::Storage)?;
        serde_json::to_writer_pretty(&mut temporary, settings)
            .map_err(|_| SettingsError::Storage)?;
        temporary
            .write_all(b"\n")
            .map_err(|_| SettingsError::Storage)?;
        temporary.flush().map_err(|_| SettingsError::Storage)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| SettingsError::Storage)?;
        temporary
            .persist(&self.path)
            .map_err(|_| SettingsError::Storage)?;
        Ok(())
    }

    async fn ensure_profile_version_secret<S: AiModelProfileStore + ?Sized>(
        &self,
        store: &S,
        binding: AiModelProfileBinding,
        legacy_secrets: &dyn SecretStore,
    ) -> Result<Vec<AiModelProfileBinding>, SettingsError> {
        let mut copied = Vec::new();
        if self
            .versioned_secrets
            .contains_secret_entry(binding.profile_id, binding.profile_version)?
        {
            return Ok(copied);
        }

        let mut missing_versions = Vec::new();
        let mut cursor = binding.profile_version;
        while cursor > 1
            && !self
                .versioned_secrets
                .contains_secret_entry(binding.profile_id, cursor)?
        {
            let current = store
                .get_ai_model_profile_version(binding.profile_id, cursor)
                .await?;
            let previous = store
                .get_ai_model_profile_version(binding.profile_id, cursor - 1)
                .await?;
            if !profile_credential_identity_matches(&current, &previous) {
                return Ok(copied);
            }
            missing_versions.push(cursor);
            cursor -= 1;
        }

        if cursor == 1
            && !self
                .versioned_secrets
                .contains_secret_entry(binding.profile_id, 1)?
        {
            let Some(secret) = legacy_secrets
                .get_secret()?
                .filter(|secret| !secret.is_empty())
                .map(Zeroizing::new)
            else {
                return Ok(copied);
            };
            self.versioned_secrets
                .set_secret(binding.profile_id, 1, secret.as_str())?;
            copied.push(AiModelProfileBinding {
                profile_id: binding.profile_id,
                profile_version: 1,
            });
        }

        for profile_version in missing_versions.into_iter().rev() {
            if !self.copy_version_secret_if_missing(
                binding.profile_id,
                profile_version - 1,
                binding.profile_id,
                profile_version,
            )? {
                break;
            }
            copied.push(AiModelProfileBinding {
                profile_id: binding.profile_id,
                profile_version,
            });
        }
        Ok(copied)
    }

    fn copy_version_secret_if_missing(
        &self,
        source_profile_id: Uuid,
        source_profile_version: i64,
        destination_profile_id: Uuid,
        destination_profile_version: i64,
    ) -> Result<bool, SettingsError> {
        if self
            .versioned_secrets
            .contains_secret_entry(destination_profile_id, destination_profile_version)?
        {
            return Ok(false);
        }
        let Some(secret) = self
            .versioned_secrets
            .get_secret(source_profile_id, source_profile_version)?
            .filter(|secret| !secret.is_empty())
            .map(Zeroizing::new)
        else {
            return Ok(false);
        };
        self.versioned_secrets.set_secret(
            destination_profile_id,
            destination_profile_version,
            secret.as_str(),
        )?;
        Ok(true)
    }

    async fn reconcile_profile_secret_refs<S: AiModelProfileSecretRefStore + ?Sized>(
        &self,
        store: &S,
        binding: AiModelProfileBinding,
        force_current_revision: bool,
        audit: &AuditContext,
    ) -> Result<(), SettingsError> {
        for profile_version in 1..=binding.profile_version {
            let account =
                KeyringSecretStore::for_profile_version(binding.profile_id, profile_version)
                    .account();
            let credential_state = if self
                .versioned_secrets
                .get_secret(binding.profile_id, profile_version)?
                .is_some()
            {
                AiModelCredentialState::Present
            } else {
                AiModelCredentialState::Revoked
            };
            let current = store
                .get_ai_model_profile_secret_ref(binding.profile_id, profile_version)
                .await?;
            if current.as_ref().is_some_and(|current| {
                current.keyring_account == account
                    && current.credential_state == credential_state
                    && !(force_current_revision && profile_version == binding.profile_version)
            }) {
                continue;
            }
            let now = Utc::now();
            let expected_revision = current.as_ref().map(|current| current.revision);
            let value = match current {
                Some(mut current) => {
                    current.credential_state = credential_state;
                    current.updated_at = now;
                    current.revision = current
                        .revision
                        .checked_add(1)
                        .ok_or(SettingsError::InvalidFile)?;
                    current
                }
                None => AiModelProfileSecretRef {
                    profile_id: binding.profile_id,
                    profile_version,
                    keyring_account: account,
                    credential_state,
                    created_at: now,
                    updated_at: now,
                    revision: 1,
                },
            };
            store
                .save_ai_model_profile_secret_ref(&value, expected_revision, audit)
                .await?;
        }
        Ok(())
    }

    fn compensate_copied_secret_entries(&self, copied: &[AiModelProfileBinding]) {
        for binding in copied {
            let _ = self
                .versioned_secrets
                .clear_secret(binding.profile_id, binding.profile_version);
        }
    }

    fn fail_closed_after_save_error(&self, settings: &AiSettingsFile) {
        let _ = self.secrets.clear_secret();
        let _ = self.vision_secrets.clear_secret();
        let text_limit = settings
            .conversation_profile_version
            .max(1)
            .saturating_add(1);
        let vision_limit = settings.vision_profile_version.max(0).saturating_add(1);
        for profile_version in 1..=text_limit {
            let _ = self
                .versioned_secrets
                .clear_secret(MIGRATED_LOCAL_PROFILE_ID, profile_version);
        }
        for profile_version in 1..=vision_limit {
            let _ = self
                .versioned_secrets
                .clear_secret(MIGRATED_LOCAL_VISION_PROFILE_ID, profile_version);
        }
    }

    fn sync_materialized_versions(
        &self,
        text_binding: AiModelProfileBinding,
        vision_binding: Option<AiModelProfileBinding>,
    ) -> Result<(), SettingsError> {
        let _guard = self.write_lock.lock().map_err(|_| SettingsError::Storage)?;
        let mut settings = self.read_or_default()?;
        let vision_profile_version = vision_binding
            .map(|binding| binding.profile_version)
            .unwrap_or(settings.vision_profile_version);
        if settings.conversation_profile_version == text_binding.profile_version
            && settings.vision_profile_version == vision_profile_version
        {
            return Ok(());
        }
        settings.conversation_profile_version = text_binding.profile_version;
        settings.vision_profile_version = vision_profile_version;
        self.write_atomic(&settings)
    }
}

#[derive(Clone)]
struct ModelProfileSpec {
    id: Uuid,
    name: &'static str,
    transport: AiProviderTransport,
    model_id: String,
    supports_vision: bool,
    base_url: String,
    runtime: AssistantRuntimeConfig,
}

impl ModelProfileSpec {
    fn version(&self, number: i64, created_at: DateTime<Utc>) -> AiModelProfileVersion {
        AiModelProfileVersion {
            profile_id: self.id,
            version: number,
            protocol: AiProviderProtocol::OpenaiChatCompletions,
            transport: self.transport,
            base_url: self.base_url.clone(),
            normalized_base_url: normalized_url(&self.base_url),
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
}

async fn materialize_profile<S: AiModelProfileStore + ?Sized>(
    store: &S,
    spec: &ModelProfileSpec,
    now: DateTime<Utc>,
    audit: &AuditContext,
) -> Result<AiModelProfileBinding, SettingsError> {
    let mut profile = match store.get_ai_model_profile(spec.id).await {
        Ok(profile) => profile,
        Err(StoreError::NotFound { .. }) => {
            let profile = AiModelProfile {
                id: spec.id,
                lab_id: LOCAL_LAB_ID,
                user_id: LOCAL_USER_ID,
                name: spec.name.to_owned(),
                current_version: 1,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            store
                .create_ai_model_profile(&profile, &spec.version(1, now), audit)
                .await?;
            return Ok(AiModelProfileBinding {
                profile_id: profile.id,
                profile_version: 1,
            });
        }
        Err(error) => return Err(error.into()),
    };
    if profile.lab_id != LOCAL_LAB_ID
        || profile.user_id != LOCAL_USER_ID
        || profile.name != spec.name
        || profile.archived_at.is_some()
        || profile.meta.deleted_at.is_some()
    {
        return Err(StoreError::Validation(
            "the deterministic local AI model profile identity is unavailable".to_owned(),
        )
        .into());
    }

    let current = store
        .get_ai_model_profile_version(profile.id, profile.current_version)
        .await?;
    if profile_version_matches(&current, spec) {
        return Ok(AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: profile.current_version,
        });
    }

    let expected_revision = profile.meta.revision;
    profile.current_version += 1;
    profile.meta.touch(now);
    let next = spec.version(profile.current_version, now);
    store
        .append_ai_model_profile_version(&profile, &next, expected_revision, audit)
        .await?;
    Ok(AiModelProfileBinding {
        profile_id: profile.id,
        profile_version: profile.current_version,
    })
}

async fn existing_compatible_profile_binding<S: AiModelProfileStore + ?Sized>(
    store: &S,
    profile_id: Uuid,
    expected_name: &str,
) -> Result<Option<AiModelProfileBinding>, SettingsError> {
    let profile = match store.get_ai_model_profile(profile_id).await {
        Ok(profile) => profile,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if profile.lab_id != LOCAL_LAB_ID
        || profile.user_id != LOCAL_USER_ID
        || profile.name != expected_name
        || profile.archived_at.is_some()
        || profile.meta.deleted_at.is_some()
    {
        return Err(StoreError::Validation(
            "the deterministic local AI model profile identity is unavailable".to_owned(),
        )
        .into());
    }
    Ok(Some(AiModelProfileBinding {
        profile_id,
        profile_version: profile.current_version,
    }))
}

fn profile_version_matches(version: &AiModelProfileVersion, spec: &ModelProfileSpec) -> bool {
    version.protocol == AiProviderProtocol::OpenaiChatCompletions
        && version.transport == spec.transport
        && version.base_url == spec.base_url
        && version.normalized_base_url == normalized_url(&spec.base_url)
        && version.model_id == spec.model_id
        && version.supports_vision == spec.supports_vision
        && version.context_window_tokens == spec.runtime.context_window_tokens
        && version.max_input_tokens == spec.runtime.max_input_tokens
        && version.max_output_tokens == spec.runtime.max_output_tokens
        && version.history_token_budget == spec.runtime.history_token_budget
        && version.history_turns == spec.runtime.history_turns
        && version.temperature == spec.runtime.temperature
        && version.timeout_ms == spec.runtime.timeout_ms
}

fn profile_credential_identity_matches(
    current: &AiModelProfileVersion,
    previous: &AiModelProfileVersion,
) -> bool {
    current.protocol == previous.protocol
        && current.transport == previous.transport
        && current.normalized_base_url == previous.normalized_base_url
}

fn settings_profile_matches(
    settings: &AiSettingsFile,
    provider: &ProviderConfig,
    runtime: &AssistantRuntimeConfig,
    model_id: &str,
    supports_vision: bool,
) -> bool {
    let model_matches = if supports_vision {
        settings.supports_vision && settings.vision_model.as_deref() == Some(model_id)
    } else {
        settings.provider.model == model_id
    };
    settings.provider.kind == provider.kind
        && settings.provider.base_url == provider.base_url
        && normalized_url(&settings.provider.base_url) == normalized_url(&provider.base_url)
        && model_matches
        && settings.context_window_tokens == runtime.context_window_tokens
        && settings.max_input_tokens == runtime.max_input_tokens
        && settings.max_output_tokens == runtime.max_output_tokens
        && settings.history_token_budget == runtime.history_token_budget
        && settings.history_turns == runtime.history_turns
        && settings.temperature == runtime.temperature
        && settings.timeout_ms == runtime.timeout_ms
}

async fn materialize_defaults<S: AiModelProfileStore + ?Sized>(
    store: &S,
    conversation_profile_id: Uuid,
    vision_profile_id: Option<Uuid>,
    now: DateTime<Utc>,
    audit: &AuditContext,
) -> Result<(), SettingsError> {
    let current = store.get_ai_user_model_defaults(LOCAL_USER_ID).await?;
    if current.as_ref().is_some_and(|defaults| {
        defaults.default_conversation_profile_id == Some(conversation_profile_id)
            && defaults.default_vision_profile_id == vision_profile_id
    }) {
        return Ok(());
    }
    let expected_revision = current.as_ref().map(|defaults| defaults.meta.revision);
    let defaults = match current {
        Some(mut defaults) => {
            defaults.default_conversation_profile_id = Some(conversation_profile_id);
            defaults.default_vision_profile_id = vision_profile_id;
            defaults.meta.touch(now);
            defaults
        }
        None => AiUserModelDefaults {
            user_id: LOCAL_USER_ID,
            default_conversation_profile_id: Some(conversation_profile_id),
            default_vision_profile_id: vision_profile_id,
            meta: RecordMeta::new(now),
        },
    };
    store
        .save_ai_user_model_defaults(&defaults, expected_revision, audit)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiSettingsFile {
    schema_version: u32,
    enabled: bool,
    provider: ProviderConfig,
    #[serde(default = "default_provider_preset_id")]
    provider_preset_id: String,
    #[serde(default)]
    supports_vision: bool,
    #[serde(default)]
    vision_model: Option<String>,
    #[serde(default = "default_context_window_tokens")]
    context_window_tokens: u32,
    #[serde(default = "default_max_input_tokens")]
    max_input_tokens: u32,
    #[serde(default = "default_max_output_tokens")]
    max_output_tokens: u32,
    #[serde(default = "default_history_token_budget")]
    history_token_budget: u32,
    #[serde(default = "default_history_turns")]
    history_turns: u32,
    #[serde(default = "default_temperature")]
    temperature: f32,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_conversation_profile_version")]
    conversation_profile_version: i64,
    #[serde(default)]
    vision_profile_version: i64,
    #[serde(default)]
    revision: u64,
}

impl AiSettingsFile {
    fn runtime(&self) -> Result<AssistantRuntimeConfig, SettingsError> {
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
        .map_err(|_| SettingsError::InvalidFile)
    }
}

impl Default for AiSettingsFile {
    fn default() -> Self {
        let runtime = AssistantRuntimeConfig::default();
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            enabled: true,
            provider: ProviderConfig::openai_compatible(
                PROVIDER_ID,
                "deepseek-chat",
                "https://api.deepseek.com",
            ),
            provider_preset_id: default_provider_preset_id(),
            supports_vision: false,
            vision_model: None,
            context_window_tokens: runtime.context_window_tokens,
            max_input_tokens: runtime.max_input_tokens,
            max_output_tokens: runtime.max_output_tokens,
            history_token_budget: runtime.history_token_budget,
            history_turns: runtime.history_turns,
            temperature: runtime.temperature,
            timeout_ms: runtime.timeout_ms,
            conversation_profile_version: default_conversation_profile_version(),
            vision_profile_version: 0,
            revision: 0,
        }
    }
}

pub(crate) struct AiSecret(Zeroizing<String>);

impl AiSecret {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for AiSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AiSecret([REDACTED])")
    }
}

pub(crate) struct ResolvedAiProvider {
    pub(crate) provider: BuiltinProvider,
    pub(crate) api_key: Option<AiSecret>,
    pub(crate) runtime: AssistantRuntimeConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiSettingsView {
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
    pub revision: u64,
}

impl AiSettingsView {
    fn from_file(file: AiSettingsFile, has_key: bool) -> Self {
        Self {
            enabled: file.enabled,
            provider_kind: file.provider.kind,
            provider_preset_id: file.provider_preset_id,
            model: file.provider.model,
            base_url: file.provider.base_url,
            has_key,
            supports_vision: file.supports_vision,
            vision_model: file.vision_model,
            context_window_tokens: file.context_window_tokens,
            max_input_tokens: file.max_input_tokens,
            max_output_tokens: file.max_output_tokens,
            history_token_budget: file.history_token_budget,
            history_turns: file.history_turns,
            temperature: file.temperature,
            timeout_ms: file.timeout_ms,
            revision: file.revision,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveAiSettingsInput {
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
const fn default_conversation_profile_version() -> i64 {
    1
}

impl SaveAiSettingsInput {
    fn runtime(&self) -> Result<AssistantRuntimeConfig, SettingsError> {
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
        .map_err(|_| SettingsError::InvalidFile)
    }
}

fn validated_preset_id(value: &str) -> Result<String, SettingsError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        Err(SettingsError::InvalidFile)
    } else {
        Ok(value.to_owned())
    }
}

fn normalized_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn provider_transport(kind: ProviderKind) -> AiProviderTransport {
    match kind {
        ProviderKind::OpenAiCompatible => AiProviderTransport::OpenAiCompatible,
        ProviderKind::LocalHttp => AiProviderTransport::LocalHttp,
    }
}

fn infer_preset_id(base_url: &str) -> &'static str {
    if base_url.starts_with("https://api.deepseek.com") {
        "deepseek"
    } else if base_url.starts_with("https://open.bigmodel.cn/") {
        "zhipu-glm"
    } else if base_url.starts_with("https://api.moonshot.cn/") {
        "moonshot-kimi"
    } else if base_url.starts_with("https://api.openai.com/") {
        "openai"
    } else {
        "custom-openai-compatible"
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use super::*;
    use muriarc_core::{
        Actor, ActorType, AuditAction, AuditFilter, EntityType, Lab, MuriArcStore, User,
        WriteSource,
    };
    use muriarc_store_sqlite::SqliteStore;
    use tempfile::tempdir;

    #[derive(Default)]
    struct FakeSecretStore(Mutex<Option<String>>);

    impl SecretStore for FakeSecretStore {
        fn get_secret(&self) -> Result<Option<String>, SettingsError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SettingsError::CredentialStore)?
                .clone())
        }

        fn contains_secret_entry(&self) -> Result<bool, SettingsError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SettingsError::CredentialStore)?
                .is_some())
        }

        fn set_secret(&self, secret: &str) -> Result<(), SettingsError> {
            *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = Some(secret.to_owned());
            Ok(())
        }

        fn clear_secret(&self) -> Result<(), SettingsError> {
            *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeVersionedSecretStore(Mutex<BTreeMap<(Uuid, i64), String>>);

    impl FakeVersionedSecretStore {
        fn raw_secret(&self, profile_id: Uuid, profile_version: i64) -> Option<String> {
            self.0
                .lock()
                .unwrap()
                .get(&(profile_id, profile_version))
                .cloned()
        }
    }

    impl VersionedSecretStore for FakeVersionedSecretStore {
        fn get_secret(
            &self,
            profile_id: Uuid,
            profile_version: i64,
        ) -> Result<Option<String>, SettingsError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SettingsError::CredentialStore)?
                .get(&(profile_id, profile_version))
                .filter(|secret| !secret.is_empty())
                .cloned())
        }

        fn contains_secret_entry(
            &self,
            profile_id: Uuid,
            profile_version: i64,
        ) -> Result<bool, SettingsError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SettingsError::CredentialStore)?
                .contains_key(&(profile_id, profile_version)))
        }

        fn set_secret(
            &self,
            profile_id: Uuid,
            profile_version: i64,
            secret: &str,
        ) -> Result<(), SettingsError> {
            self.0
                .lock()
                .map_err(|_| SettingsError::CredentialStore)?
                .insert((profile_id, profile_version), secret.to_owned());
            Ok(())
        }

        fn clear_secret(
            &self,
            profile_id: Uuid,
            profile_version: i64,
        ) -> Result<(), SettingsError> {
            self.set_secret(profile_id, profile_version, "")
        }
    }

    #[test]
    fn profile_keyring_accounts_are_stable_and_isolated_from_legacy() {
        let profile_a = MIGRATED_LOCAL_PROFILE_ID;
        let profile_b = Uuid::parse_str("018f4b65-30ac-7fe2-9ef0-2a773be63a25").unwrap();

        assert_eq!(
            KeyringSecretStore::default().account(),
            LEGACY_KEYRING_ACCOUNT
        );
        assert_eq!(
            KeyringSecretStore::for_profile(profile_a).account(),
            "local-user-model-profile-4d555249-4152-4300-0000-000000000002-api-key"
        );
        assert_eq!(profile_a, LOCAL_USER_ID);
        assert!(KeyringSecretStore::for_profile(profile_a).migrate_legacy);
        assert_eq!(
            KeyringSecretStore::for_profile(MIGRATED_LOCAL_VISION_PROFILE_ID).account(),
            "local-user-model-profile-4d555249-4152-f300-0000-000000000002-api-key"
        );
        let local_user_id = LOCAL_USER_ID.to_string();
        let migrated_vision_id = format!("{}f{}", &local_user_id[..14], &local_user_id[15..]);
        assert_eq!(
            MIGRATED_LOCAL_VISION_PROFILE_ID,
            Uuid::parse_str(&migrated_vision_id).unwrap(),
            "Desktop and SQLite migration must derive the same vision profile ID"
        );
        assert_eq!(
            KeyringSecretStore::for_profile_version(profile_a, 7).account(),
            "local-user-model-profile-4d555249-4152-4300-0000-000000000002-v7-api-key"
        );
        assert!(KeyringSecretStore::for_profile(MIGRATED_LOCAL_VISION_PROFILE_ID).migrate_legacy);
        assert!(!KeyringSecretStore::for_profile(profile_b).migrate_legacy);
        assert_ne!(
            KeyringSecretStore::for_profile(profile_a).account(),
            KeyringSecretStore::for_profile(profile_b).account()
        );
    }

    #[test]
    fn legacy_secret_copy_is_explicit_non_destructive_and_never_overwrites() {
        let legacy = FakeSecretStore(Mutex::new(Some("legacy-secret".to_owned())));
        let profile = FakeSecretStore::default();

        assert!(copy_secret_if_missing(&legacy, &profile).unwrap());
        assert!(!copy_secret_if_missing(&legacy, &profile).unwrap());
        assert_eq!(
            legacy.get_secret().unwrap().as_deref(),
            Some("legacy-secret")
        );
        assert_eq!(
            profile.get_secret().unwrap().as_deref(),
            Some("legacy-secret")
        );

        profile.set_secret("profile-secret").unwrap();
        assert!(!copy_secret_if_missing(&legacy, &profile).unwrap());
        assert_eq!(
            profile.get_secret().unwrap().as_deref(),
            Some("profile-secret")
        );

        profile.set_secret("").unwrap();
        assert!(!copy_secret_if_missing(&legacy, &profile).unwrap());
        assert_eq!(profile.get_secret().unwrap().as_deref(), Some(""));

        let unrelated_profile = FakeSecretStore::default();
        assert_eq!(unrelated_profile.get_secret().unwrap(), None);
    }

    fn service() -> (
        tempfile::TempDir,
        SettingsService,
        Arc<FakeSecretStore>,
        Arc<FakeVersionedSecretStore>,
    ) {
        let temp = tempdir().unwrap();
        let secrets = Arc::new(FakeSecretStore::default());
        let versioned_secrets = Arc::new(FakeVersionedSecretStore::default());
        let service = SettingsService::new(
            temp.path().join("ai-provider.json"),
            secrets.clone(),
            versioned_secrets.clone(),
        );
        (temp, service, secrets, versioned_secrets)
    }

    async fn local_profile_store(path: &Path) -> SqliteStore {
        let store = SqliteStore::connect_path(path).await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let bootstrap = AuditContext::system(WriteSource::Migration);
        let mut lab = Lab::new("Local profile test", now).unwrap();
        lab.id = LOCAL_LAB_ID;
        store.create_lab(&lab, &bootstrap).await.unwrap();
        let mut user = User::new(
            LOCAL_LAB_ID,
            "local.profile@muriarc.invalid",
            "Local profile operator",
            now,
        )
        .unwrap();
        user.id = LOCAL_USER_ID;
        store.create_user(&user, &bootstrap).await.unwrap();
        store
    }

    fn save_input(
        model: &str,
        base_url: &str,
        supports_vision: bool,
        vision_model: Option<&str>,
        api_key: Option<&str>,
    ) -> SaveAiSettingsInput {
        SaveAiSettingsInput {
            enabled: true,
            provider_kind: ProviderKind::OpenAiCompatible,
            provider_preset_id: if base_url.starts_with("https://api.deepseek.com") {
                "deepseek"
            } else {
                "custom-openai-compatible"
            }
            .to_owned(),
            model: model.to_owned(),
            base_url: base_url.to_owned(),
            supports_vision,
            vision_model: vision_model.map(str::to_owned),
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

    #[tokio::test]
    async fn materializes_legacy_json_as_idempotent_profile_versions_and_defaults() {
        let (temp, service, secrets, versioned_secrets) = service();
        secrets.set_secret("must-never-enter-the-database").unwrap();
        let store = local_profile_store(&temp.path().join("muriarc.sqlite3")).await;
        let mut audit = AuditContext::system(WriteSource::Migration);
        audit.reason = Some("materialize_desktop_ai_model_profiles".to_owned());

        let first = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(
            first,
            AiModelProfileBinding {
                profile_id: LOCAL_USER_ID,
                profile_version: 1,
            }
        );
        let repeated = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(repeated, first);
        let profile = store
            .get_ai_model_profile(MIGRATED_LOCAL_PROFILE_ID)
            .await
            .unwrap();
        assert_eq!(profile.current_version, 1);
        assert_eq!(profile.meta.revision, 1);
        let defaults = store
            .get_ai_user_model_defaults(LOCAL_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            defaults.default_conversation_profile_id,
            Some(MIGRATED_LOCAL_PROFILE_ID)
        );
        assert_eq!(defaults.default_vision_profile_id, None);
        assert_eq!(defaults.meta.revision, 1);
        assert_eq!(
            versioned_secrets
                .get_secret(MIGRATED_LOCAL_PROFILE_ID, 1)
                .unwrap()
                .as_deref(),
            Some("must-never-enter-the-database")
        );

        service
            .save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                provider_preset_id: "deepseek".to_owned(),
                model: "deepseek-chat-next".to_owned(),
                base_url: "https://api.deepseek.com".to_owned(),
                supports_vision: true,
                vision_model: Some("deepseek-vision".to_owned()),
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: None,
            })
            .unwrap();
        let changed = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(changed.profile_version, 2);
        assert_eq!(
            store
                .get_ai_model_profile_version(MIGRATED_LOCAL_PROFILE_ID, 1)
                .await
                .unwrap()
                .model_id,
            "deepseek-chat"
        );
        assert_eq!(
            store
                .get_ai_model_profile_version(MIGRATED_LOCAL_PROFILE_ID, 2)
                .await
                .unwrap()
                .model_id,
            "deepseek-chat-next"
        );
        let historical = service
            .resolve_provider_for_profile(
                &store,
                AiModelProfileBinding {
                    profile_id: MIGRATED_LOCAL_PROFILE_ID,
                    profile_version: 1,
                },
            )
            .await
            .unwrap();
        let current = service
            .resolve_provider_for_profile(&store, changed)
            .await
            .unwrap();
        assert_eq!(historical.provider.config().model, "deepseek-chat");
        assert_eq!(current.provider.config().model, "deepseek-chat-next");
        assert_eq!(
            historical.provider.config().base_url,
            "https://api.deepseek.com"
        );
        assert_eq!(
            current.provider.config().base_url,
            "https://api.deepseek.com"
        );
        assert_eq!(
            historical.api_key.as_ref().map(AiSecret::as_str),
            Some("must-never-enter-the-database")
        );
        assert_eq!(
            current.api_key.as_ref().map(AiSecret::as_str),
            Some("must-never-enter-the-database")
        );
        let vision = store
            .get_ai_model_profile_version(MIGRATED_LOCAL_VISION_PROFILE_ID, 1)
            .await
            .unwrap();
        assert_eq!(vision.model_id, "deepseek-vision");
        assert!(vision.supports_vision);
        let changed_defaults = store
            .get_ai_user_model_defaults(LOCAL_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            changed_defaults.default_vision_profile_id,
            Some(MIGRATED_LOCAL_VISION_PROFILE_ID)
        );
        assert_eq!(changed_defaults.meta.revision, 2);

        let repeated_changed = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(repeated_changed, changed);
        assert_eq!(
            store
                .get_ai_model_profile(MIGRATED_LOCAL_PROFILE_ID)
                .await
                .unwrap()
                .meta
                .revision,
            2
        );
        let profile_audit = store
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(MIGRATED_LOCAL_PROFILE_ID),
            })
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| {
                entry.entity_type == EntityType::AiModelProfile
                    && entry
                        .after
                        .as_ref()
                        .and_then(|after| after.get("name"))
                        .is_some()
            })
            .collect::<Vec<_>>();
        assert_eq!(profile_audit.len(), 2);
        assert!(
            profile_audit
                .iter()
                .all(|entry| entry.source == WriteSource::Migration)
        );
        assert!(
            !serde_json::to_string(&profile_audit)
                .unwrap()
                .contains("must-never-enter-the-database")
        );

        let unsupported_profile = AiModelProfile {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            name: "Unsupported protocol profile".to_owned(),
            current_version: 1,
            archived_at: None,
            meta: RecordMeta::new(Utc::now()),
        };
        let unsupported_version = AiModelProfileVersion {
            profile_id: unsupported_profile.id,
            version: 1,
            protocol: AiProviderProtocol::OpenaiResponses,
            transport: AiProviderTransport::OpenAiCompatible,
            base_url: "https://provider.example.test/v1".to_owned(),
            normalized_base_url: "https://provider.example.test/v1".to_owned(),
            model_id: "responses-model".to_owned(),
            supports_vision: false,
            context_window_tokens: default_context_window_tokens(),
            max_input_tokens: default_max_input_tokens(),
            max_output_tokens: default_max_output_tokens(),
            history_token_budget: default_history_token_budget(),
            history_turns: default_history_turns(),
            temperature: default_temperature(),
            timeout_ms: default_timeout_ms(),
            created_at: Utc::now(),
        };
        store
            .create_ai_model_profile(&unsupported_profile, &unsupported_version, &audit)
            .await
            .unwrap();
        assert!(matches!(
            service
                .resolve_provider_for_profile(
                    &store,
                    AiModelProfileBinding {
                        profile_id: unsupported_profile.id,
                        profile_version: 1,
                    },
                )
                .await,
            Err(SettingsError::UnsupportedProtocol)
        ));
    }

    #[tokio::test]
    async fn failed_desktop_secret_metadata_write_fails_closed_and_reconciles_state() {
        let (temp, service, legacy_secrets, versioned_secrets) = service();
        legacy_secrets
            .set_secret("credential-before-failed-save")
            .unwrap();
        let store = local_profile_store(&temp.path().join("failed-secret-audit.sqlite3")).await;
        let migration_audit = AuditContext::system(WriteSource::Migration);
        service
            .materialize_model_profiles(&store, &migration_audit)
            .await
            .unwrap();
        let original = store
            .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.credential_state, AiModelCredentialState::Present);
        assert_eq!(original.revision, 1);

        let invalid_desktop_audit = AuditContext {
            actor: Actor::human(Uuid::new_v4(), "Wrong Desktop User"),
            source: WriteSource::Desktop,
            request_id: Some("failed-secret-metadata-write".to_owned()),
            reason: Some("replace_model_credential".to_owned()),
        };
        assert!(matches!(
            service
                .save_and_materialize(
                    &store,
                    save_input(
                        "deepseek-chat",
                        "https://api.deepseek.com",
                        false,
                        None,
                        Some("credential-must-be-failed-closed"),
                    ),
                    &invalid_desktop_audit,
                )
                .await,
            Err(SettingsError::ModelProfileStore(StoreError::Validation(_)))
        ));
        assert_eq!(
            versioned_secrets
                .raw_secret(MIGRATED_LOCAL_PROFILE_ID, 1)
                .as_deref(),
            Some("")
        );
        assert!(!service.get().unwrap().has_key);
        assert_eq!(
            store
                .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 1)
                .await
                .unwrap()
                .unwrap()
                .credential_state,
            AiModelCredentialState::Present,
            "the failed transaction must not publish a partial metadata update"
        );

        service
            .materialize_model_profiles(&store, &migration_audit)
            .await
            .unwrap();
        let revoked = store
            .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revoked.credential_state, AiModelCredentialState::Revoked);
        assert_eq!(revoked.revision, 2);

        versioned_secrets
            .set_secret(
                MIGRATED_LOCAL_PROFILE_ID,
                1,
                "credential-restored-outside-metadata",
            )
            .unwrap();
        service
            .materialize_model_profiles(&store, &migration_audit)
            .await
            .unwrap();
        let restored = store
            .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.credential_state, AiModelCredentialState::Present);
        assert_eq!(restored.revision, 3);

        let audits = store
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(MIGRATED_LOCAL_PROFILE_ID),
            })
            .await
            .unwrap();
        let serialized = serde_json::to_string(&audits).unwrap();
        assert!(!serialized.contains("credential-before-failed-save"));
        assert!(!serialized.contains("credential-must-be-failed-closed"));
        assert!(!serialized.contains("credential-restored-outside-metadata"));
    }

    #[tokio::test]
    async fn desktop_key_rotation_and_clear_audit_exact_versions_and_repair_missing_refs() {
        let (temp, service, legacy_secrets, versioned_secrets) = service();
        legacy_secrets
            .set_secret("legacy-credential-value")
            .unwrap();
        let store = local_profile_store(&temp.path().join("secret-ref-lifecycle.sqlite3")).await;
        let migration_audit = AuditContext::system(WriteSource::Migration);
        service
            .materialize_model_profiles(&store, &migration_audit)
            .await
            .unwrap();

        let rotate_audit = AuditContext {
            actor: Actor::human(LOCAL_USER_ID, "Local profile operator"),
            source: WriteSource::Desktop,
            request_id: Some("desktop-key-rotation".to_owned()),
            reason: Some("replace_model_credential".to_owned()),
        };
        let saved = service
            .save_and_materialize(
                &store,
                save_input(
                    "deepseek-chat",
                    "https://api.deepseek.com",
                    false,
                    None,
                    Some("replacement-credential-value"),
                ),
                &rotate_audit,
            )
            .await
            .unwrap();
        assert!(saved.has_key);
        let rotated = store
            .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rotated.credential_state, AiModelCredentialState::Present);
        assert_eq!(rotated.revision, 2);

        service
            .save_and_materialize(
                &store,
                save_input(
                    "deepseek-chat-next",
                    "https://api.deepseek.com",
                    false,
                    None,
                    Some("replacement-credential-value"),
                ),
                &rotate_audit,
            )
            .await
            .unwrap();
        let current = store
            .get_ai_model_profile_secret_ref(MIGRATED_LOCAL_PROFILE_ID, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.credential_state, AiModelCredentialState::Present);
        assert_eq!(current.revision, 1);

        sqlx::query(
            "DELETE FROM ai_model_profile_secret_refs
             WHERE profile_id = ? AND profile_version = 1",
        )
        .bind(MIGRATED_LOCAL_PROFILE_ID.to_string())
        .execute(store.pool())
        .await
        .unwrap();
        let clear_audit = AuditContext {
            actor: Actor::human(LOCAL_USER_ID, "Local profile operator"),
            source: WriteSource::Desktop,
            request_id: Some("desktop-key-clear".to_owned()),
            reason: Some("revoke_model_credentials".to_owned()),
        };
        assert!(
            !service
                .clear_key_with_metadata(&store, &clear_audit)
                .await
                .unwrap()
                .has_key
        );

        let references = store
            .list_ai_model_profile_secret_refs(MIGRATED_LOCAL_PROFILE_ID)
            .await
            .unwrap();
        assert_eq!(references.len(), 2);
        assert!(references.iter().all(|reference| {
            reference.credential_state == AiModelCredentialState::Revoked
                && reference.keyring_account
                    == KeyringSecretStore::for_profile_version(
                        reference.profile_id,
                        reference.profile_version,
                    )
                    .account()
        }));
        assert_eq!(references[0].profile_version, 1);
        assert_eq!(references[0].revision, 1);
        assert_eq!(references[1].profile_version, 2);
        assert_eq!(references[1].revision, 2);
        for profile_version in 1..=2 {
            assert_eq!(
                versioned_secrets
                    .raw_secret(MIGRATED_LOCAL_PROFILE_ID, profile_version)
                    .as_deref(),
                Some("")
            );
        }

        let secret_ref_audits = store
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(MIGRATED_LOCAL_PROFILE_ID),
            })
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| {
                entry.entity_type == EntityType::AiModelProfile
                    && entry
                        .after
                        .as_ref()
                        .and_then(|after| after.get("keyring_account"))
                        .is_some()
            })
            .collect::<Vec<_>>();
        assert!(secret_ref_audits.iter().any(|entry| {
            entry.request_id.as_deref() == Some("desktop-key-rotation")
                && entry.action == AuditAction::Update
                && entry.source == WriteSource::Desktop
                && entry.actor.actor_type == ActorType::Human
        }));
        assert!(secret_ref_audits.iter().any(|entry| {
            entry.request_id.as_deref() == Some("desktop-key-clear")
                && entry.action == AuditAction::Revoke
                && entry.source == WriteSource::Desktop
                && entry.actor.user_id == Some(LOCAL_USER_ID)
        }));
        let serialized = serde_json::to_string(&secret_ref_audits).unwrap();
        assert!(!serialized.contains("legacy-credential-value"));
        assert!(!serialized.contains("replacement-credential-value"));
    }

    #[tokio::test]
    async fn profile_versions_resolve_their_own_credentials_after_key_rotation() {
        let (temp, service, legacy_secrets, _versioned_secrets) = service();
        legacy_secrets.set_secret("credential-a").unwrap();
        let store = local_profile_store(&temp.path().join("key-rotation.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);

        let version_one = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        service
            .save(save_input(
                "provider-b-model",
                "https://provider-b.example/v1",
                false,
                None,
                Some("credential-b"),
            ))
            .unwrap();
        let version_two = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(version_one.profile_version, 1);
        assert_eq!(version_two.profile_version, 2);

        let resolved_one = service
            .resolve_provider_for_profile(&store, version_one)
            .await
            .unwrap();
        let resolved_two = service
            .resolve_provider_for_profile(&store, version_two)
            .await
            .unwrap();
        assert_eq!(
            resolved_one.api_key.as_ref().map(AiSecret::as_str),
            Some("credential-a")
        );
        assert_eq!(
            resolved_two.api_key.as_ref().map(AiSecret::as_str),
            Some("credential-b")
        );
        assert_eq!(
            resolved_one.provider.config().base_url,
            "https://api.deepseek.com"
        );
        assert_eq!(
            resolved_two.provider.config().base_url,
            "https://provider-b.example/v1"
        );
    }

    #[tokio::test]
    async fn local_https_transport_round_trips_and_remains_distinct_from_cloud_transport() {
        let (temp, service, _legacy_secrets, _versioned_secrets) = service();
        let store = local_profile_store(&temp.path().join("local-https.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        let mut local_input = save_input(
            "shared-model",
            "https://gateway.example.test/v1",
            false,
            None,
            Some("local-credential"),
        );
        local_input.provider_kind = ProviderKind::LocalHttp;
        service.save(local_input).unwrap();
        let local_binding = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        let local_version = store
            .get_ai_model_profile_version(local_binding.profile_id, local_binding.profile_version)
            .await
            .unwrap();
        assert_eq!(local_version.transport, AiProviderTransport::LocalHttp);
        let resolved_local = service
            .resolve_provider_for_profile(&store, local_binding)
            .await
            .unwrap();
        assert_eq!(
            resolved_local.provider.config().kind,
            ProviderKind::LocalHttp
        );
        assert_eq!(
            resolved_local.api_key.as_ref().map(AiSecret::as_str),
            Some("local-credential")
        );

        service
            .save(save_input(
                "shared-model",
                "https://gateway.example.test/v1",
                false,
                None,
                Some("cloud-credential"),
            ))
            .unwrap();
        let cloud_binding = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_ai_model_profile_version(
                    cloud_binding.profile_id,
                    cloud_binding.profile_version,
                )
                .await
                .unwrap()
                .transport,
            AiProviderTransport::OpenAiCompatible
        );
        let resolved_cloud = service
            .resolve_provider_for_profile(&store, cloud_binding)
            .await
            .unwrap();
        assert_eq!(
            resolved_cloud.provider.config().kind,
            ProviderKind::OpenAiCompatible
        );
        assert_eq!(
            resolved_cloud.api_key.as_ref().map(AiSecret::as_str),
            Some("cloud-credential")
        );
        assert_eq!(
            service
                .resolve_provider_for_profile(&store, local_binding)
                .await
                .unwrap()
                .api_key
                .as_ref()
                .map(AiSecret::as_str),
            Some("local-credential")
        );
        assert_eq!(
            cloud_binding.profile_version,
            local_binding.profile_version + 1
        );
    }

    #[tokio::test]
    async fn preset_only_edit_preserves_the_current_version_and_credential() {
        let (temp, service, legacy_secrets, _versioned_secrets) = service();
        legacy_secrets.set_secret("credential-a").unwrap();
        let store = local_profile_store(&temp.path().join("preset-only.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);
        let original = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        let mut input = save_input(
            "deepseek-chat",
            "https://api.deepseek.com",
            false,
            None,
            None,
        );
        input.provider_preset_id = "custom-openai-compatible".to_owned();
        assert!(service.save(input).unwrap().has_key);
        let after_edit = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        assert_eq!(after_edit, original);
        assert_eq!(
            service
                .resolve_provider_for_profile(&store, after_edit)
                .await
                .unwrap()
                .api_key
                .as_ref()
                .map(AiSecret::as_str),
            Some("credential-a")
        );
    }

    #[tokio::test]
    async fn identity_change_without_key_leaves_only_the_new_version_uncredentialed() {
        let (temp, service, legacy_secrets, versioned_secrets) = service();
        legacy_secrets.set_secret("credential-a").unwrap();
        let store = local_profile_store(&temp.path().join("missing-key.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);

        let version_one = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        service
            .save(save_input(
                "provider-b-model",
                "https://provider-b.example/v1",
                false,
                None,
                None,
            ))
            .unwrap();
        let version_two = service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        assert_eq!(
            service
                .resolve_provider_for_profile(&store, version_one)
                .await
                .unwrap()
                .api_key
                .as_ref()
                .map(AiSecret::as_str),
            Some("credential-a")
        );
        assert!(matches!(
            service
                .resolve_provider_for_profile(&store, version_two)
                .await,
            Err(SettingsError::MissingCredential)
        ));
        assert_eq!(
            versioned_secrets
                .raw_secret(MIGRATED_LOCAL_PROFILE_ID, version_two.profile_version)
                .as_deref(),
            Some("")
        );
    }

    #[tokio::test]
    async fn reenabling_vision_copies_the_current_text_identity_not_disabled_vision_history() {
        let (temp, service, legacy_secrets, _versioned_secrets) = service();
        legacy_secrets.set_secret("credential-a").unwrap();
        let store = local_profile_store(&temp.path().join("vision-reenable.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        service
            .save(save_input(
                "deepseek-chat",
                "https://api.deepseek.com",
                true,
                Some("vision-a"),
                Some("credential-a"),
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        service
            .save(save_input(
                "provider-b-chat",
                "https://provider-b.example/v1",
                false,
                Some("disabled-residual"),
                Some("credential-b"),
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        service
            .save(save_input(
                "provider-b-chat",
                "https://provider-b.example/v1",
                true,
                Some("vision-b"),
                None,
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        let old_vision = service
            .resolve_provider_for_profile(
                &store,
                AiModelProfileBinding {
                    profile_id: MIGRATED_LOCAL_VISION_PROFILE_ID,
                    profile_version: 1,
                },
            )
            .await
            .unwrap();
        let current_vision = service
            .resolve_provider_for_profile(
                &store,
                AiModelProfileBinding {
                    profile_id: MIGRATED_LOCAL_VISION_PROFILE_ID,
                    profile_version: 2,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            old_vision.api_key.as_ref().map(AiSecret::as_str),
            Some("credential-a")
        );
        assert_eq!(
            current_vision.api_key.as_ref().map(AiSecret::as_str),
            Some("credential-b")
        );
        assert_eq!(
            old_vision.provider.config().base_url,
            "https://api.deepseek.com"
        );
        assert_eq!(
            current_vision.provider.config().base_url,
            "https://provider-b.example/v1"
        );
    }

    #[tokio::test]
    async fn disabling_vision_ignores_residual_model_and_clear_revokes_all_compatible_versions() {
        let (temp, service, legacy_secrets, versioned_secrets) = service();
        legacy_secrets.set_secret("legacy-credential").unwrap();
        let store = local_profile_store(&temp.path().join("vision-disable.sqlite3")).await;
        let audit = AuditContext::system(WriteSource::Migration);
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        service
            .save(save_input(
                "deepseek-chat",
                "https://api.deepseek.com",
                true,
                Some("vision-one"),
                Some("credential-one"),
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        service
            .save(save_input(
                "deepseek-chat-two",
                "https://api.deepseek.com",
                true,
                Some("vision-two"),
                Some("credential-two"),
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();

        service
            .save(save_input(
                "deepseek-chat-two",
                "https://api.deepseek.com",
                false,
                Some("must-be-ignored"),
                None,
            ))
            .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        let settings_path = temp.path().join("ai-provider.json");
        let mut legacy_settings: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
        legacy_settings
            .as_object_mut()
            .unwrap()
            .remove("vision_profile_version");
        fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&legacy_settings).unwrap(),
        )
        .unwrap();
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        let recovered_settings: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(recovered_settings["vision_profile_version"], 2);
        let view = service.get().unwrap();
        assert!(!view.supports_vision);
        assert_eq!(view.vision_model, None);
        let defaults = store
            .get_ai_user_model_defaults(LOCAL_USER_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(defaults.default_vision_profile_id, None);
        assert_eq!(
            store
                .get_ai_model_profile(MIGRATED_LOCAL_VISION_PROFILE_ID)
                .await
                .unwrap()
                .current_version,
            2
        );
        for profile_version in 1..=2 {
            assert!(
                versioned_secrets
                    .get_secret(MIGRATED_LOCAL_PROFILE_ID, profile_version)
                    .unwrap()
                    .is_some()
            );
            assert!(
                versioned_secrets
                    .get_secret(MIGRATED_LOCAL_VISION_PROFILE_ID, profile_version)
                    .unwrap()
                    .is_some()
            );
        }

        assert!(!service.clear_key().unwrap().has_key);
        service
            .materialize_model_profiles(&store, &audit)
            .await
            .unwrap();
        for profile_version in 1..=2 {
            assert_eq!(
                versioned_secrets
                    .raw_secret(MIGRATED_LOCAL_PROFILE_ID, profile_version)
                    .as_deref(),
                Some("")
            );
            assert_eq!(
                versioned_secrets
                    .raw_secret(MIGRATED_LOCAL_VISION_PROFILE_ID, profile_version)
                    .as_deref(),
                Some("")
            );
        }
    }

    #[test]
    fn saves_config_atomically_without_serializing_the_secret() {
        let (temp, service, _secrets, versioned_secrets) = service();
        let view = service
            .save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                provider_preset_id: "custom-openai-compatible".to_owned(),
                model: "gpt-4.1-mini".to_owned(),
                base_url: "https://api.openai.com/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: Some("test-secret-value".to_owned()),
            })
            .unwrap();
        assert!(view.has_key);
        assert_eq!(
            versioned_secrets
                .raw_secret(MIGRATED_LOCAL_PROFILE_ID, 2)
                .as_deref(),
            Some("test-secret-value")
        );
        let saved = fs::read_to_string(temp.path().join("ai-provider.json")).unwrap();
        assert!(!saved.contains("test-secret-value"));
        assert!(!saved.to_ascii_lowercase().contains("api_key"));

        service
            .save(SaveAiSettingsInput {
                enabled: false,
                provider_kind: ProviderKind::LocalHttp,
                provider_preset_id: "custom-openai-compatible".to_owned(),
                model: "local-model".to_owned(),
                base_url: "http://127.0.0.1:11434/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: None,
            })
            .unwrap();
        assert_eq!(
            service.get().unwrap().provider_kind,
            ProviderKind::LocalHttp
        );
    }

    #[test]
    fn enforces_cloud_https_and_allows_local_http() {
        let (_temp, service, _secrets, _versioned_secrets) = service();
        assert!(matches!(
            service.save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                provider_preset_id: "custom-openai-compatible".to_owned(),
                model: "model".to_owned(),
                base_url: "http://example.org/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: None,
            }),
            Err(SettingsError::InvalidProvider(_))
        ));
        assert!(
            service
                .save(SaveAiSettingsInput {
                    enabled: true,
                    provider_kind: ProviderKind::LocalHttp,
                    provider_preset_id: "custom-openai-compatible".to_owned(),
                    model: "model".to_owned(),
                    base_url: "http://127.0.0.1:11434/v1".to_owned(),
                    supports_vision: false,
                    vision_model: None,
                    context_window_tokens: default_context_window_tokens(),
                    max_input_tokens: default_max_input_tokens(),
                    max_output_tokens: default_max_output_tokens(),
                    history_token_budget: default_history_token_budget(),
                    history_turns: default_history_turns(),
                    temperature: default_temperature(),
                    timeout_ms: default_timeout_ms(),
                    api_key: None,
                })
                .is_ok()
        );
    }

    #[test]
    fn clear_key_is_idempotent_and_never_returns_the_secret() {
        let (_temp, service, _secrets, _versioned_secrets) = service();
        service
            .save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                provider_preset_id: "custom-openai-compatible".to_owned(),
                model: "model".to_owned(),
                base_url: "https://example.org/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                context_window_tokens: default_context_window_tokens(),
                max_input_tokens: default_max_input_tokens(),
                max_output_tokens: default_max_output_tokens(),
                history_token_budget: default_history_token_budget(),
                history_turns: default_history_turns(),
                temperature: default_temperature(),
                timeout_ms: default_timeout_ms(),
                api_key: Some("secret".to_owned()),
            })
            .unwrap();
        assert!(!service.clear_key().unwrap().has_key);
        assert!(!service.clear_key().unwrap().has_key);
    }
}
