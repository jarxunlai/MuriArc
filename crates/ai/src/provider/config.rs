use std::fmt;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MIN_TIMEOUT_MS: u64 = 50;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MIN_MAX_RESPONSE_BYTES: usize = 1024;
pub const MAX_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROVIDER_ID_BYTES: usize = 64;
const MAX_MODEL_BYTES: usize = 256;
const MAX_BASE_URL_BYTES: usize = 2048;
const MAX_API_KEY_BYTES: usize = 8192;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
    LocalHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: usize,
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

const fn default_max_response_bytes() -> usize {
    DEFAULT_MAX_RESPONSE_BYTES
}

impl ProviderConfig {
    pub fn openai_compatible(
        provider_id: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            kind: ProviderKind::OpenAiCompatible,
            model: model.into(),
            base_url: base_url.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    pub fn local_http(
        provider_id: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            kind: ProviderKind::LocalHttp,
            model: model.into(),
            base_url: base_url.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    pub(super) fn validate(
        &self,
        expected_kind: ProviderKind,
    ) -> Result<ValidatedConfig, ProviderConfigError> {
        if self.kind != expected_kind {
            return Err(ProviderConfigError::KindMismatch);
        }
        if self.provider_id.is_empty()
            || self.provider_id.len() > MAX_PROVIDER_ID_BYTES
            || !self
                .provider_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ProviderConfigError::InvalidProviderId);
        }
        if self.model.trim().is_empty() || self.model.len() > MAX_MODEL_BYTES {
            return Err(ProviderConfigError::InvalidModel);
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(ProviderConfigError::InvalidTimeout);
        }
        if !(MIN_MAX_RESPONSE_BYTES..=MAX_MAX_RESPONSE_BYTES).contains(&self.max_response_bytes) {
            return Err(ProviderConfigError::InvalidResponseLimit);
        }
        if self.base_url.len() > MAX_BASE_URL_BYTES {
            return Err(ProviderConfigError::InvalidBaseUrl);
        }

        let mut base_url =
            Url::parse(&self.base_url).map_err(|_| ProviderConfigError::InvalidBaseUrl)?;
        if !base_url.has_host()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(ProviderConfigError::InvalidBaseUrl);
        }
        match expected_kind {
            ProviderKind::OpenAiCompatible if base_url.scheme() != "https" => {
                return Err(ProviderConfigError::HttpsRequired);
            }
            ProviderKind::LocalHttp if !matches!(base_url.scheme(), "http" | "https") => {
                return Err(ProviderConfigError::InvalidBaseUrl);
            }
            _ => {}
        }

        let path = base_url.path().trim_end_matches('/');
        if !path.ends_with("/chat/completions") {
            let endpoint_path = if path.is_empty() {
                "/chat/completions".to_owned()
            } else {
                format!("{path}/chat/completions")
            };
            base_url.set_path(&endpoint_path);
        }

        Ok(ValidatedConfig { endpoint: base_url })
    }
}

#[derive(Debug)]
pub(super) struct ValidatedConfig {
    pub(super) endpoint: Url,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderConfigError {
    #[error("provider kind does not match the selected provider implementation")]
    KindMismatch,
    #[error("provider id is invalid")]
    InvalidProviderId,
    #[error("model name is invalid")]
    InvalidModel,
    #[error("base URL is invalid")]
    InvalidBaseUrl,
    #[error("OpenAI-compatible cloud providers require HTTPS")]
    HttpsRequired,
    #[error("request timeout is outside the allowed range")]
    InvalidTimeout,
    #[error("response size limit is outside the allowed range")]
    InvalidResponseLimit,
    #[error("HTTP client could not be initialized")]
    ClientInitialization,
}

#[derive(Clone, Copy)]
pub struct ProviderCredentials<'a> {
    pub(super) api_key: Option<&'a str>,
}

impl<'a> ProviderCredentials<'a> {
    pub const fn none() -> Self {
        Self { api_key: None }
    }

    pub fn bearer(api_key: &'a str) -> Result<Self, CredentialError> {
        if api_key.is_empty()
            || api_key.len() > MAX_API_KEY_BYTES
            || api_key.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(CredentialError::InvalidApiKey);
        }
        Ok(Self {
            api_key: Some(api_key),
        })
    }
}

impl Default for ProviderCredentials<'_> {
    fn default() -> Self {
        Self::none()
    }
}

impl fmt::Debug for ProviderCredentials<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCredentials")
            .field("api_key", &self.api_key.map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CredentialError {
    #[error("API key is empty, too large, or contains control characters")]
    InvalidApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    Timeout,
    Connection,
    Request,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    #[error(transparent)]
    InvalidConfig(#[from] ProviderConfigError),
    #[error("completion request is invalid: {0}")]
    InvalidRequest(&'static str),
    #[error("serialized request exceeds {limit} bytes")]
    RequestTooLarge { limit: usize },
    #[error("provider response exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("provider transport failed: {kind:?}")]
    Transport { kind: TransportFailure },
    #[error("provider returned HTTP status {status}")]
    HttpStatus {
        status: u16,
        request_id: Option<String>,
    },
    #[error("provider returned a malformed response")]
    MalformedResponse,
    #[error("provider returned neither content nor a tool call")]
    EmptyResponse,
    #[error("provider exhausted the output token budget before returning final content")]
    OutputBudgetExhausted,
    #[error("mock provider has no queued response")]
    MockExhausted,
    #[error("mock provider state is unavailable")]
    MockUnavailable,
}
