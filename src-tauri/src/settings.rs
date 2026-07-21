use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use muriarc_ai::{
    BuiltinProvider, ProviderConfig, ProviderConfigError, ProviderCredentials, ProviderKind,
};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;
use zeroize::Zeroizing;

const SETTINGS_SCHEMA_VERSION: u32 = 1;
const PROVIDER_ID: &str = "desktop-user-provider";
const KEYRING_SERVICE: &str = concat!(env!("MURIARC_BUNDLE_IDENTIFIER"), ".ai");
const KEYRING_ACCOUNT: &str = "local-user-provider-api-key";

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
        )
    }
}

pub(crate) trait SecretStore: Send + Sync {
    fn get_secret(&self) -> Result<Option<String>, SettingsError>;
    fn has_secret(&self) -> Result<bool, SettingsError> {
        Ok(self.get_secret()?.is_some_and(|secret| !secret.is_empty()))
    }
    fn set_secret(&self, secret: &str) -> Result<(), SettingsError>;
    fn clear_secret(&self) -> Result<(), SettingsError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KeyringSecretStore;

impl KeyringSecretStore {
    fn entry() -> Result<keyring::Entry, SettingsError> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|_| SettingsError::CredentialStore)
    }
}

impl SecretStore for KeyringSecretStore {
    fn get_secret(&self) -> Result<Option<String>, SettingsError> {
        match Self::entry()?.get_password() {
            Ok(secret) if secret.is_empty() => Ok(None),
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(SettingsError::CredentialStore),
        }
    }

    fn set_secret(&self, secret: &str) -> Result<(), SettingsError> {
        Self::entry()?
            .set_password(secret)
            .map_err(|_| SettingsError::CredentialStore)
    }

    fn clear_secret(&self) -> Result<(), SettingsError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(SettingsError::CredentialStore),
        }
    }
}

#[derive(Clone)]
pub(crate) struct SettingsService {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    write_lock: Arc<Mutex<()>>,
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
    pub(crate) fn for_app_data(app_data_dir: &Path) -> Self {
        Self::new(
            app_data_dir.join("ai-provider.json"),
            Arc::new(KeyringSecretStore),
        )
    }

    pub(crate) fn new(path: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            path,
            secrets,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn get(&self) -> Result<AiSettingsView, SettingsError> {
        let settings = self.read_or_default()?;
        Ok(AiSettingsView::from_file(
            settings,
            self.secrets.has_secret()?,
        ))
    }

    pub(crate) fn save(&self, input: SaveAiSettingsInput) -> Result<AiSettingsView, SettingsError> {
        let provider = match input.provider_kind {
            ProviderKind::OpenAiCompatible => {
                ProviderConfig::openai_compatible(PROVIDER_ID, input.model, input.base_url)
            }
            ProviderKind::LocalHttp => {
                ProviderConfig::local_http(PROVIDER_ID, input.model, input.base_url)
            }
        };
        BuiltinProvider::from_config(provider.clone()).map_err(SettingsError::InvalidProvider)?;

        if let Some(secret) = input.api_key.as_deref() {
            ProviderCredentials::bearer(secret).map_err(|_| SettingsError::InvalidCredential)?;
        }

        let vision_model = input
            .vision_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if input.supports_vision {
            let mut vision_provider = provider.clone();
            vision_provider.model = vision_model.clone().ok_or(SettingsError::InvalidFile)?;
            BuiltinProvider::from_config(vision_provider)
                .map_err(SettingsError::InvalidProvider)?;
        }
        let file = AiSettingsFile {
            schema_version: SETTINGS_SCHEMA_VERSION,
            enabled: input.enabled,
            provider,
            supports_vision: input.supports_vision,
            vision_model,
        };
        let _guard = self.write_lock.lock().map_err(|_| SettingsError::Storage)?;
        if let Some(secret) = input.api_key.as_deref() {
            self.secrets.set_secret(secret)?;
        }
        self.write_atomic(&file)?;
        Ok(AiSettingsView::from_file(file, self.secrets.has_secret()?))
    }

    pub(crate) fn resolve_provider(&self) -> Result<ResolvedAiProvider, SettingsError> {
        let settings = self.read_or_default()?;
        if !settings.enabled {
            return Err(SettingsError::Disabled);
        }
        let provider = BuiltinProvider::from_config(settings.provider.clone())
            .map_err(SettingsError::InvalidProvider)?;
        let api_key = self.secrets.get_secret()?.map(AiSecret::new);
        if settings.provider.kind == ProviderKind::OpenAiCompatible && api_key.is_none() {
            return Err(SettingsError::MissingCredential);
        }
        Ok(ResolvedAiProvider { provider, api_key })
    }

    // Kept as the validated local multimodal boundary; the current desktop commands
    // only issue text requests, while image extraction is served by the shared server.
    #[allow(dead_code)]
    pub(crate) fn resolve_vision_provider(&self) -> Result<ResolvedAiProvider, SettingsError> {
        let settings = self.read_or_default()?;
        if !settings.enabled || !settings.supports_vision {
            return Err(SettingsError::Disabled);
        }
        let mut config = settings.provider.clone();
        config.model = settings.vision_model.ok_or(SettingsError::InvalidFile)?;
        let provider =
            BuiltinProvider::from_config(config).map_err(SettingsError::InvalidProvider)?;
        let api_key = self.secrets.get_secret()?.map(AiSecret::new);
        if settings.provider.kind == ProviderKind::OpenAiCompatible && api_key.is_none() {
            return Err(SettingsError::MissingCredential);
        }
        Ok(ResolvedAiProvider { provider, api_key })
    }

    pub(crate) fn clear_key(&self) -> Result<AiSettingsView, SettingsError> {
        let _guard = self.write_lock.lock().map_err(|_| SettingsError::Storage)?;
        self.secrets.clear_secret()?;
        let settings = self.read_or_default()?;
        Ok(AiSettingsView::from_file(settings, false))
    }

    fn read_or_default(&self) -> Result<AiSettingsFile, SettingsError> {
        if !self.path.exists() {
            return Ok(AiSettingsFile::default());
        }
        let bytes = fs::read(&self.path).map_err(|_| SettingsError::Storage)?;
        let settings: AiSettingsFile =
            serde_json::from_slice(&bytes).map_err(|_| SettingsError::InvalidFile)?;
        if settings.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::InvalidFile);
        }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiSettingsFile {
    schema_version: u32,
    enabled: bool,
    provider: ProviderConfig,
    #[serde(default)]
    supports_vision: bool,
    #[serde(default)]
    vision_model: Option<String>,
}

impl Default for AiSettingsFile {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            enabled: false,
            provider: ProviderConfig::openai_compatible(
                PROVIDER_ID,
                "gpt-4.1-mini",
                "https://api.openai.com/v1",
            ),
            supports_vision: false,
            vision_model: None,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiSettingsView {
    pub enabled: bool,
    pub provider_kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub has_key: bool,
    pub supports_vision: bool,
    pub vision_model: Option<String>,
}

impl AiSettingsView {
    fn from_file(file: AiSettingsFile, has_key: bool) -> Self {
        Self {
            enabled: file.enabled,
            provider_kind: file.provider.kind,
            model: file.provider.model,
            base_url: file.provider.base_url,
            has_key,
            supports_vision: file.supports_vision,
            vision_model: file.vision_model,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveAiSettingsInput {
    pub enabled: bool,
    pub provider_kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
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

        fn set_secret(&self, secret: &str) -> Result<(), SettingsError> {
            *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = Some(secret.to_owned());
            Ok(())
        }

        fn clear_secret(&self) -> Result<(), SettingsError> {
            *self.0.lock().map_err(|_| SettingsError::CredentialStore)? = None;
            Ok(())
        }
    }

    fn service() -> (tempfile::TempDir, SettingsService, Arc<FakeSecretStore>) {
        let temp = tempdir().unwrap();
        let secrets = Arc::new(FakeSecretStore::default());
        let service = SettingsService::new(temp.path().join("ai-provider.json"), secrets.clone());
        (temp, service, secrets)
    }

    #[test]
    fn saves_config_atomically_without_serializing_the_secret() {
        let (temp, service, secrets) = service();
        let view = service
            .save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                model: "gpt-4.1-mini".to_owned(),
                base_url: "https://api.openai.com/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                api_key: Some("test-secret-value".to_owned()),
            })
            .unwrap();
        assert!(view.has_key);
        assert_eq!(
            secrets.0.lock().unwrap().as_deref(),
            Some("test-secret-value")
        );
        let saved = fs::read_to_string(temp.path().join("ai-provider.json")).unwrap();
        assert!(!saved.contains("test-secret-value"));
        assert!(!saved.to_ascii_lowercase().contains("api_key"));

        service
            .save(SaveAiSettingsInput {
                enabled: false,
                provider_kind: ProviderKind::LocalHttp,
                model: "local-model".to_owned(),
                base_url: "http://127.0.0.1:11434/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
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
        let (_temp, service, _secrets) = service();
        assert!(matches!(
            service.save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                model: "model".to_owned(),
                base_url: "http://example.org/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                api_key: None,
            }),
            Err(SettingsError::InvalidProvider(_))
        ));
        assert!(
            service
                .save(SaveAiSettingsInput {
                    enabled: true,
                    provider_kind: ProviderKind::LocalHttp,
                    model: "model".to_owned(),
                    base_url: "http://127.0.0.1:11434/v1".to_owned(),
                    supports_vision: false,
                    vision_model: None,
                    api_key: None,
                })
                .is_ok()
        );
    }

    #[test]
    fn clear_key_is_idempotent_and_never_returns_the_secret() {
        let (_temp, service, _secrets) = service();
        service
            .save(SaveAiSettingsInput {
                enabled: true,
                provider_kind: ProviderKind::OpenAiCompatible,
                model: "model".to_owned(),
                base_url: "https://example.org/v1".to_owned(),
                supports_vision: false,
                vision_model: None,
                api_key: Some("secret".to_owned()),
            })
            .unwrap();
        assert!(!service.clear_key().unwrap().has_key);
        assert!(!service.clear_key().unwrap().has_key);
    }
}
