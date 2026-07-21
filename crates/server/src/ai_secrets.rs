use std::fmt;

use async_trait::async_trait;
use muriarc_ai::{BuiltinProvider, ProviderKind};
#[cfg(feature = "postgres")]
use muriarc_ai::{ProviderConfig, ProviderCredentials};
use muriarc_core::AuditContext;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(feature = "postgres")]
const SERVER_PROVIDER_ID: &str = "server-user-provider";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderSettingsView {
    pub enabled: bool,
    pub provider_kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub has_key: bool,
    pub supports_vision: bool,
    pub vision_model: Option<String>,
    pub revision: i64,
}

impl AiProviderSettingsView {
    #[cfg(feature = "postgres")]
    fn unconfigured() -> Self {
        Self {
            enabled: false,
            provider_kind: ProviderKind::OpenAiCompatible,
            model: "gpt-4.1-mini".to_owned(),
            base_url: "https://api.openai.com/v1".to_owned(),
            has_key: false,
            supports_vision: false,
            vision_model: None,
            revision: 0,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiProviderSettingsInput {
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

impl fmt::Debug for SaveAiProviderSettingsInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SaveAiProviderSettingsInput")
            .field("enabled", &self.enabled)
            .field("provider_kind", &self.provider_kind)
            .field("model", &"[REDACTED]")
            .field("base_url", &"[REDACTED]")
            .field("supports_vision", &self.supports_vision)
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderDiagnosticsView {
    pub runtime_configured: bool,
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
pub struct AiLabSettingsView {
    pub enabled: bool,
    pub custom_url_approval_required: bool,
    pub configured_user_count: i64,
    pub enabled_user_count: i64,
    pub vision_user_count: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiLabSettingsInput {
    pub enabled: bool,
    pub custom_url_approval_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderEndpointView {
    pub id: Uuid,
    pub provider_kind: ProviderKind,
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
    pub label: String,
    pub base_url: String,
    pub enabled: bool,
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
    #[error("AI provider is disabled")]
    Disabled,
    #[error("AI provider credential is missing")]
    MissingCredential,
    #[error("AI secret master key configuration is invalid")]
    InvalidMasterKey,
    #[error("AI secret could not be encrypted or decrypted")]
    Encryption,
    #[error("AI provider settings storage is unavailable")]
    Storage,
    #[error("AI provider settings are not configured")]
    NotConfigured,
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
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    #[test]
    fn provider_input_debug_is_redacted() {
        let input = SaveAiProviderSettingsInput {
            enabled: true,
            provider_kind: ProviderKind::OpenAiCompatible,
            model: "private-model".to_owned(),
            base_url: "https://private-provider.example.test/v1".to_owned(),
            supports_vision: true,
            vision_model: Some("private-vision-model".to_owned()),
            api_key: Some("private-api-key".to_owned()),
        };
        let debug = format!("{input:?}");
        assert!(!debug.contains("private-model"));
        assert!(!debug.contains("private-provider"));
        assert!(!debug.contains("private-api-key"));
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
    const OFFICIAL_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
    const OFFICIAL_OPENAI_LABEL: &str = "OpenAI API";

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

        fn builtin_openai_endpoint_id() -> Uuid {
            Uuid::from_u128(1)
        }

        fn builtin_openai_endpoint() -> AiProviderEndpointView {
            AiProviderEndpointView {
                id: Self::builtin_openai_endpoint_id(),
                provider_kind: ProviderKind::OpenAiCompatible,
                label: OFFICIAL_OPENAI_LABEL.to_owned(),
                base_url: OFFICIAL_OPENAI_BASE_URL.to_owned(),
                enabled: true,
                builtin: true,
                revision: 1,
            }
        }

        fn endpoint_view(
            row: &sqlx::postgres::PgRow,
        ) -> Result<AiProviderEndpointView, AiProviderStoreError> {
            let kind: String = row
                .try_get("provider_kind")
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(AiProviderEndpointView {
                id: row.try_get("id").map_err(|_| AiProviderStoreError::Storage)?,
                provider_kind: Self::provider_kind_from_db(&kind)?,
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
                "label": view.label,
                "enabled": view.enabled,
                "builtin": view.builtin,
                "base_url_present": true,
                "revision": view.revision,
            })
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

        async fn validate_endpoint_for_lab(
            &self,
            lab_id: Uuid,
            config: &ProviderConfig,
        ) -> Result<(), AiProviderStoreError> {
            let normalized = normalized_url(&config.base_url);
            if config.kind == ProviderKind::OpenAiCompatible
                && normalized == normalized_url(OFFICIAL_OPENAI_BASE_URL)
            {
                return Ok(());
            }
            let allowed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ai_provider_endpoints WHERE lab_id = $1 AND provider_kind = $2 AND normalized_base_url = $3 AND enabled = TRUE)",
            )
            .bind(lab_id)
            .bind(Self::provider_kind_name(config.kind))
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
            Ok((counts.0 as usize, counts.1 as usize + 1))
        }

        fn encrypt(
            &self,
            user_id: Uuid,
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
                    Aad::from(aad(user_id, self.master_key.version).as_bytes()),
                    &mut ciphertext,
                )
                .map_err(|_| AiProviderStoreError::Encryption)?;
            Ok((nonce, ciphertext))
        }

        fn decrypt(
            &self,
            user_id: Uuid,
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
                    Aad::from(aad(user_id, key_version).as_bytes()),
                    &mut plaintext,
                )
                .map_err(|_| AiProviderStoreError::Encryption)?;
            let secret =
                String::from_utf8(opened.to_vec()).map_err(|_| AiProviderStoreError::Encryption)?;
            plaintext.fill(0);
            Ok(SensitiveSecret::new(secret))
        }

        async fn row(
            &self,
            user_id: Uuid,
        ) -> Result<Option<sqlx::postgres::PgRow>, AiProviderStoreError> {
            sqlx::query("SELECT enabled, provider_config, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, revision FROM ai_provider_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(self.postgres.pool())
                .await
                .map_err(|_| AiProviderStoreError::Storage)
        }

        async fn locked_row(
            transaction: &mut Transaction<'_, Postgres>,
            user_id: Uuid,
        ) -> Result<Option<sqlx::postgres::PgRow>, AiProviderStoreError> {
            sqlx::query("SELECT enabled, provider_config, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, revision FROM ai_provider_settings WHERE user_id = $1 FOR UPDATE")
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
                "SELECT lab_id FROM users WHERE id = $1 AND status = 'active' AND deleted_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::Storage)
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
                revision: row
                    .try_get("revision")
                    .map_err(|_| AiProviderStoreError::Storage)?,
            })
        }

        async fn resolve_row(
            &self,
            user_id: Uuid,
            lab_id: Uuid,
            row: &sqlx::postgres::PgRow,
            vision: bool,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            let enabled: bool = row
                .try_get("enabled")
                .map_err(|_| AiProviderStoreError::Storage)?;
            if !enabled {
                return Err(AiProviderStoreError::Disabled);
            }
            let mut config = decode_config(row)?;
            if vision {
                let supports_vision: bool = row
                    .try_get("supports_vision")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                let vision_model: Option<String> = row
                    .try_get("vision_model")
                    .map_err(|_| AiProviderStoreError::Storage)?;
                if !supports_vision {
                    return Err(AiProviderStoreError::Disabled);
                }
                config.model = vision_model.ok_or(AiProviderStoreError::InvalidSettings)?;
            }
            self.validate_endpoint_for_lab(lab_id, &config).await?;
            let provider = BuiltinProvider::from_config(config.clone())
                .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            let key_version: Option<i32> = row
                .try_get("secret_key_version")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let nonce: Option<Vec<u8>> = row
                .try_get("secret_nonce")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let ciphertext: Option<Vec<u8>> = row
                .try_get("secret_ciphertext")
                .map_err(|_| AiProviderStoreError::Storage)?;
            let api_key = match (key_version, nonce, ciphertext) {
                (Some(version), Some(nonce), Some(ciphertext)) => {
                    Some(self.decrypt(user_id, version, &nonce, &ciphertext)?)
                }
                (None, None, None) => None,
                _ => return Err(AiProviderStoreError::Encryption),
            };
            if config.kind == ProviderKind::OpenAiCompatible && api_key.is_none() {
                return Err(AiProviderStoreError::MissingCredential);
            }
            Ok(ResolvedAiProvider { provider, api_key })
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
            let vision_model = input
                .vision_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if input.supports_vision && vision_model.is_none() {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            if let Some(model) = vision_model.as_ref() {
                let mut vision_config = config.clone();
                vision_config.model = model.clone();
                BuiltinProvider::from_config(vision_config)
                    .map_err(|_| AiProviderStoreError::InvalidSettings)?;
            }
            let config_json =
                serde_json::to_value(&config).map_err(|_| AiProviderStoreError::Storage)?;
            let credential_action = if input.api_key.is_some() {
                "replace"
            } else {
                "preserve"
            };
            let mut transaction = self
                .postgres
                .pool()
                .begin()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            let lab_id = Self::active_user_lab(&mut transaction, user_id).await?;
            self.validate_endpoint_for_lab(lab_id, &config).await?;
            let current = Self::locked_row(&mut transaction, user_id).await?;
            let before = settings_audit_state(current.as_ref())?;
            let (key_version, nonce, ciphertext) = match input.api_key.as_deref() {
                Some(secret) => {
                    let (nonce, ciphertext) = self.encrypt(user_id, secret)?;
                    (
                        Some(self.master_key.version),
                        Some(nonce.to_vec()),
                        Some(ciphertext),
                    )
                }
                None => match current.as_ref() {
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
            };
            if input.enabled
                && config.kind == ProviderKind::OpenAiCompatible
                && ciphertext.is_none()
            {
                return Err(AiProviderStoreError::MissingCredential);
            }
            let row = sqlx::query("INSERT INTO ai_provider_settings (user_id, enabled, provider_config, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now(),now(),1) ON CONFLICT (user_id) DO UPDATE SET enabled = EXCLUDED.enabled, provider_config = EXCLUDED.provider_config, secret_key_version = EXCLUDED.secret_key_version, secret_nonce = EXCLUDED.secret_nonce, secret_ciphertext = EXCLUDED.secret_ciphertext, supports_vision = EXCLUDED.supports_vision, vision_model = EXCLUDED.vision_model, updated_at = now(), revision = ai_provider_settings.revision + 1 RETURNING enabled, provider_config, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, revision")
                .bind(user_id).bind(input.enabled).bind(config_json).bind(key_version).bind(nonce).bind(ciphertext).bind(input.supports_vision).bind(vision_model)
                .fetch_one(&mut *transaction).await.map_err(|_| AiProviderStoreError::Storage)?;
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
                let row = sqlx::query("UPDATE ai_provider_settings SET enabled = FALSE, secret_key_version = NULL, secret_nonce = NULL, secret_ciphertext = NULL, updated_at = now(), revision = revision + 1 WHERE user_id = $1 RETURNING enabled, provider_config, secret_key_version, secret_nonce, secret_ciphertext, supports_vision, vision_model, revision")
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
            let lab_id = self.active_user_lab_from_pool(user_id).await?;
            let row = self
                .row(user_id)
                .await?
                .ok_or(AiProviderStoreError::NotConfigured)?;
            self.resolve_row(user_id, lab_id, &row, false).await
        }

        async fn resolve_vision(
            &self,
            user_id: Uuid,
        ) -> Result<ResolvedAiProvider, AiProviderStoreError> {
            let lab_id = self.active_user_lab_from_pool(user_id).await?;
            let row = self
                .row(user_id)
                .await?
                .ok_or(AiProviderStoreError::NotConfigured)?;
            self.resolve_row(user_id, lab_id, &row, true).await
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
            let (local_endpoint_count, cloud_endpoint_count) = self.endpoint_counts(lab_id).await?;
            Ok(AiProviderDiagnosticsView {
                runtime_configured: true,
                provider_configured: row.is_some(),
                provider_enabled: row
                    .as_ref()
                    .and_then(|value| value.try_get("enabled").ok())
                    .unwrap_or(false),
                credential_configured: row
                    .as_ref()
                    .and_then(|value| {
                        value
                            .try_get::<Option<Vec<u8>>, _>("secret_ciphertext")
                            .ok()
                    })
                    .flatten()
                    .is_some(),
                supports_vision: row
                    .as_ref()
                    .and_then(|value| value.try_get("supports_vision").ok())
                    .unwrap_or(false),
                text_model_configured: row.is_some(),
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
            let settings: Option<(bool, bool, i64)> = sqlx::query_as(
                "SELECT enabled, custom_url_approval_required, revision FROM ai_lab_settings WHERE lab_id = $1",
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
            let (_, _, revision) = settings.unwrap_or((true, false, 0));
            Ok(AiLabSettingsView {
                enabled: true,
                custom_url_approval_required: false,
                configured_user_count: counts.0,
                enabled_user_count: counts.1,
                vision_user_count: counts.2,
                revision,
            })
        }

        async fn save_lab_settings(
            &self,
            lab_id: Uuid,
            _input: SaveAiLabSettingsInput,
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
                "SELECT enabled, custom_url_approval_required, revision FROM ai_lab_settings WHERE lab_id = $1 FOR UPDATE",
            )
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .map(|row| {
                json!({
                    "enabled": row.try_get::<bool, _>("enabled").unwrap_or(true),
                    "custom_url_approval_required": row.try_get::<bool, _>("custom_url_approval_required").unwrap_or(true),
                    "revision": row.try_get::<i64, _>("revision").unwrap_or(0),
                })
            });
            let row = sqlx::query(
                "INSERT INTO ai_lab_settings (lab_id, enabled, custom_url_approval_required, updated_by, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,now(),now(),1) ON CONFLICT (lab_id) DO UPDATE SET enabled = EXCLUDED.enabled, custom_url_approval_required = EXCLUDED.custom_url_approval_required, updated_by = EXCLUDED.updated_by, updated_at = now(), revision = ai_lab_settings.revision + 1 RETURNING enabled, custom_url_approval_required, revision",
            )
            .bind(lab_id)
            .bind(true)
            .bind(false)
            .bind(actor_user_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let after = json!({
                "enabled": true,
                "custom_url_approval_required": false,
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

        async fn list_provider_endpoints(
            &self,
            lab_id: Uuid,
        ) -> Result<Vec<AiProviderEndpointView>, AiProviderStoreError> {
            let rows = sqlx::query(
                "SELECT id, provider_kind, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE lab_id = $1 ORDER BY enabled DESC, provider_kind, label, base_url",
            )
            .bind(lab_id)
            .fetch_all(self.postgres.pool())
            .await
            .map_err(|_| AiProviderStoreError::Storage)?;
            let mut endpoints = vec![Self::builtin_openai_endpoint()];
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
            if endpoint_id == Some(Self::builtin_openai_endpoint_id()) {
                return Err(AiProviderStoreError::InvalidSettings);
            }
            let label = Self::validated_endpoint_label(&input.label)?;
            let config = Self::endpoint_config(&input)?;
            let normalized = normalized_url(&config.base_url);
            if config.kind == ProviderKind::OpenAiCompatible
                && normalized == normalized_url(OFFICIAL_OPENAI_BASE_URL)
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
            let before_row = match endpoint_id {
                Some(id) => sqlx::query(
                    "SELECT id, provider_kind, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE id = $1 AND lab_id = $2 AND builtin = FALSE FOR UPDATE",
                )
                .bind(id)
                .bind(lab_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| AiProviderStoreError::Storage)?,
                None => sqlx::query(
                    "SELECT id, provider_kind, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE lab_id = $1 AND provider_kind = $2 AND normalized_base_url = $3 FOR UPDATE",
                )
                .bind(lab_id)
                .bind(kind)
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
                    "UPDATE ai_provider_endpoints SET provider_kind = $1, label = $2, base_url = $3, normalized_base_url = $4, enabled = $5, updated_by = $6, updated_at = now(), revision = revision + 1 WHERE id = $7 AND lab_id = $8 AND builtin = FALSE RETURNING id, provider_kind, label, base_url, enabled, builtin, revision",
                )
                .bind(kind)
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
                    "INSERT INTO ai_provider_endpoints (id, lab_id, provider_kind, label, base_url, normalized_base_url, enabled, builtin, created_by, updated_by, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,FALSE,$8,$8,now(),now(),1) ON CONFLICT (lab_id, provider_kind, normalized_base_url) DO UPDATE SET label = EXCLUDED.label, base_url = EXCLUDED.base_url, enabled = EXCLUDED.enabled, updated_by = EXCLUDED.updated_by, updated_at = now(), revision = ai_provider_endpoints.revision + 1 RETURNING id, provider_kind, label, base_url, enabled, builtin, revision",
                )
                .bind(id)
                .bind(lab_id)
                .bind(kind)
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
                view.id,
                if before.is_some() { "update" } else { "create" },
                before,
                Some(after),
                audit,
                "AI provider endpoint saved",
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
            if audit.actor.actor_type != ActorType::Human
                || endpoint_id == Self::builtin_openai_endpoint_id()
            {
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
                "SELECT id, provider_kind, label, base_url, enabled, builtin, revision FROM ai_provider_endpoints WHERE id = $1 AND lab_id = $2 AND builtin = FALSE FOR UPDATE",
            )
            .bind(endpoint_id)
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| AiProviderStoreError::Storage)?
            .ok_or(AiProviderStoreError::InvalidSettings)?;
            let before_view = Self::endpoint_view(&current)?;
            let row = sqlx::query(
                "UPDATE ai_provider_endpoints SET enabled = FALSE, updated_by = $1, updated_at = now(), revision = revision + 1 WHERE id = $2 AND lab_id = $3 AND builtin = FALSE RETURNING id, provider_kind, label, base_url, enabled, builtin, revision",
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
                view.id,
                "update",
                Some(Self::endpoint_audit_state(&before_view)),
                Some(Self::endpoint_audit_state(&view)),
                audit,
                "AI provider endpoint disabled",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| AiProviderStoreError::Storage)?;
            Ok(view)
        }
    }

    fn aad(user_id: Uuid, version: i32) -> String {
        format!("MuriArc/ai-provider-secret/v{version}/{user_id}")
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

    async fn write_endpoint_audit(
        transaction: &mut Transaction<'_, Postgres>,
        lab_id: Uuid,
        endpoint_id: Uuid,
        action: &'static str,
        before: Option<Value>,
        after: Option<Value>,
        audit: &AuditContext,
        reason: &'static str,
    ) -> Result<(), AiProviderStoreError> {
        let actor_user_id = audit.actor.user_id.ok_or(AiProviderStoreError::Storage)?;
        sqlx::query(
            "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1,$2,NULL,'ai_provider_endpoint',$3,$4,'human',$5,$6,$7,$8,$9,$10,$11,now())",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(endpoint_id)
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
        use muriarc_core::{Actor, AuditFilter, EntityType, Lab, MuriArcStore, User};

        fn master() -> AiMasterKey {
            AiMasterKey {
                bytes: Zeroizing::new(vec![7_u8; KEY_BYTES]),
                version: 1,
            }
        }

        #[tokio::test]
        async fn ciphertext_is_user_bound_and_secret_is_redacted() {
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
        }

        #[test]
        fn master_key_requires_exactly_32_base64_bytes() {
            let encoded = general_purpose::STANDARD.encode([9_u8; KEY_BYTES]);
            assert!(AiMasterKey::from_base64(&encoded, 1).is_ok());
            assert!(AiMasterKey::from_base64("not-a-key", 1).is_err());
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
                        model: sensitive_model.to_owned(),
                        base_url: sensitive_url.to_owned(),
                        supports_vision: false,
                        vision_model: None,
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
            assert!(!cleared.enabled);
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
                        model: "rejected-model".to_owned(),
                        base_url: "https://not-allowlisted.example.test/v1".to_owned(),
                        supports_vision: false,
                        vision_model: None,
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
