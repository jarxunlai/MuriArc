use std::{fmt, sync::Arc};

use axum::http::{HeaderMap, header::HOST};
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const PRIVATE_CREDENTIAL_POLICY_REVISION: i32 = 1;
pub const PUBLIC_CREDENTIAL_POLICY_REVISION: i32 = 2;
pub const PRIVATE_PASSWORD_MIN_CHARS: usize = 8;
pub const PUBLIC_PASSWORD_MIN_CHARS: usize = 15;
pub const PASSWORD_MAX_BYTES: usize = 1024;
pub const DEFAULT_ATTACHMENT_MAX_BYTES: u64 = 100 * 1024 * 1024;
pub const CLOUDFLARE_ATTACHMENT_MAX_BYTES: u64 = 95 * 1024 * 1024;

const CF_ACCESS_CLIENT_ID: &str = "cf-access-client-id";
const CF_ACCESS_CLIENT_SECRET: &str = "cf-access-client-secret";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentProfile {
    Private,
    CloudflarePublic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialPolicy {
    revision: i32,
    min_chars: usize,
}

impl CredentialPolicy {
    pub const fn private() -> Self {
        Self {
            revision: PRIVATE_CREDENTIAL_POLICY_REVISION,
            min_chars: PRIVATE_PASSWORD_MIN_CHARS,
        }
    }

    pub const fn cloudflare_public() -> Self {
        Self {
            revision: PUBLIC_CREDENTIAL_POLICY_REVISION,
            min_chars: PUBLIC_PASSWORD_MIN_CHARS,
        }
    }

    pub const fn revision(self) -> i32 {
        self.revision
    }

    pub const fn min_chars(self) -> usize {
        self.min_chars
    }

    pub fn accepts(self, password: &str) -> bool {
        password.chars().count() >= self.min_chars
            && password.len() <= PASSWORD_MAX_BYTES
            && !password.chars().any(char::is_control)
    }
}

#[derive(Clone)]
enum ExternalApiMode {
    Disabled,
    /// Kept only for explicit in-process adapters and tests. Production main
    /// always installs either `Disabled` or `CloudflareServiceToken`.
    DevelopmentUngated,
    CloudflareServiceToken {
        hostname: Arc<str>,
        client_id_digest: [u8; 32],
        client_secret_digest: [u8; 32],
    },
}

#[derive(Clone)]
pub struct ExternalApiPolicy {
    mode: ExternalApiMode,
}

impl fmt::Debug for ExternalApiPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.mode {
            ExternalApiMode::Disabled => formatter.write_str("ExternalApiPolicy::Disabled"),
            ExternalApiMode::DevelopmentUngated => {
                formatter.write_str("ExternalApiPolicy::DevelopmentUngated")
            }
            ExternalApiMode::CloudflareServiceToken { hostname, .. } => formatter
                .debug_struct("ExternalApiPolicy::CloudflareServiceToken")
                .field("hostname", hostname)
                .field("client_id", &"[REDACTED]")
                .field("client_secret", &"[REDACTED]")
                .finish(),
        }
    }
}

impl ExternalApiPolicy {
    pub const fn disabled() -> Self {
        Self {
            mode: ExternalApiMode::Disabled,
        }
    }

    pub(crate) const fn development_ungated() -> Self {
        Self {
            mode: ExternalApiMode::DevelopmentUngated,
        }
    }

    pub fn cloudflare_service_token(
        hostname: impl Into<String>,
        client_id: &[u8],
        client_secret: &[u8],
    ) -> Result<Self, String> {
        let hostname = normalize_hostname(&hostname.into())?;
        if client_id.is_empty() || client_id.len() > 1024 || client_secret.len() < 16 {
            return Err("Cloudflare Service Token credentials are malformed".to_owned());
        }
        if client_secret.len() > 4096 {
            return Err("Cloudflare Service Token credentials are malformed".to_owned());
        }
        Ok(Self {
            mode: ExternalApiMode::CloudflareServiceToken {
                hostname: hostname.into(),
                client_id_digest: sha256(client_id),
                client_secret_digest: sha256(client_secret),
            },
        })
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.mode, ExternalApiMode::Disabled)
    }

    pub(crate) fn authorize(&self, headers: &HeaderMap) -> Result<(), ExternalApiAccessError> {
        match &self.mode {
            ExternalApiMode::Disabled => Err(ExternalApiAccessError::Disabled),
            ExternalApiMode::DevelopmentUngated => Ok(()),
            ExternalApiMode::CloudflareServiceToken {
                hostname,
                client_id_digest,
                client_secret_digest,
            } => {
                let supplied_host = headers
                    .get(HOST)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(ExternalApiAccessError::Denied)?;
                if normalize_incoming_host(supplied_host).as_deref() != Some(hostname.as_ref()) {
                    return Err(ExternalApiAccessError::Denied);
                }
                let client_id = headers
                    .get(CF_ACCESS_CLIENT_ID)
                    .map(|value| value.as_bytes())
                    .ok_or(ExternalApiAccessError::Denied)?;
                let client_secret = headers
                    .get(CF_ACCESS_CLIENT_SECRET)
                    .map(|value| value.as_bytes())
                    .ok_or(ExternalApiAccessError::Denied)?;
                if !digest_matches(client_id, client_id_digest)
                    || !digest_matches(client_secret, client_secret_digest)
                {
                    return Err(ExternalApiAccessError::Denied);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalApiAccessError {
    Disabled,
    Denied,
}

#[derive(Clone)]
pub struct DeploymentSecurityPolicy {
    profile: DeploymentProfile,
    credential_policy: CredentialPolicy,
    external_api: ExternalApiPolicy,
    attachment_max_bytes: u64,
    login_identity_hmac_key: Option<Arc<[u8; 32]>>,
}

impl fmt::Debug for DeploymentSecurityPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeploymentSecurityPolicy")
            .field("profile", &self.profile)
            .field("credential_policy", &self.credential_policy)
            .field("external_api", &self.external_api)
            .field("attachment_max_bytes", &self.attachment_max_bytes)
            .field("login_identity_hmac_key", &"[REDACTED]")
            .finish()
    }
}

impl DeploymentSecurityPolicy {
    pub(crate) fn development_default() -> Self {
        Self {
            profile: DeploymentProfile::Private,
            credential_policy: CredentialPolicy::private(),
            external_api: ExternalApiPolicy::development_ungated(),
            attachment_max_bytes: DEFAULT_ATTACHMENT_MAX_BYTES,
            login_identity_hmac_key: None,
        }
    }

    pub fn private(external_api: ExternalApiPolicy) -> Self {
        Self {
            profile: DeploymentProfile::Private,
            credential_policy: CredentialPolicy::private(),
            external_api,
            attachment_max_bytes: DEFAULT_ATTACHMENT_MAX_BYTES,
            login_identity_hmac_key: None,
        }
    }

    pub fn cloudflare_public(
        login_identity_hmac_key: [u8; 32],
        external_api: ExternalApiPolicy,
    ) -> Self {
        Self {
            profile: DeploymentProfile::CloudflarePublic,
            credential_policy: CredentialPolicy::cloudflare_public(),
            external_api,
            attachment_max_bytes: CLOUDFLARE_ATTACHMENT_MAX_BYTES,
            login_identity_hmac_key: Some(Arc::new(login_identity_hmac_key)),
        }
    }

    pub const fn profile(&self) -> DeploymentProfile {
        self.profile
    }

    pub const fn credential_policy(&self) -> CredentialPolicy {
        self.credential_policy
    }

    pub const fn attachment_max_bytes(&self) -> u64 {
        self.attachment_max_bytes
    }

    pub const fn external_api(&self) -> &ExternalApiPolicy {
        &self.external_api
    }

    pub fn login_identity_hmac_key(&self) -> Option<&[u8; 32]> {
        self.login_identity_hmac_key.as_deref()
    }

    pub fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            profile: self.profile,
            credential_policy_revision: self.credential_policy.revision,
            password_min_chars: self.credential_policy.min_chars,
            password_max_bytes: PASSWORD_MAX_BYTES,
            attachment_max_bytes: self.attachment_max_bytes,
            import_max_bytes: 32 * 1024 * 1024,
            ai_source_max_bytes: 32 * 1024 * 1024,
            external_api_enabled: self.external_api.enabled(),
            mcp_enabled: self.external_api.enabled(),
            chunked_attachment_upload: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RuntimeCapabilities {
    pub profile: DeploymentProfile,
    pub credential_policy_revision: i32,
    pub password_min_chars: usize,
    pub password_max_bytes: usize,
    pub attachment_max_bytes: u64,
    pub import_max_bytes: u64,
    pub ai_source_max_bytes: u64,
    pub external_api_enabled: bool,
    pub mcp_enabled: bool,
    pub chunked_attachment_upload: bool,
}

fn normalize_hostname(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 253
        || value.contains(['/', ':'])
        || value.chars().any(|character| {
            !character.is_ascii_alphanumeric() && character != '-' && character != '.'
        })
        || value.split('.').any(|label| {
            label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
        })
    {
        return Err(
            "external API hostname must be a DNS hostname without scheme or port".to_owned(),
        );
    }
    Ok(value)
}

fn normalize_incoming_host(value: &str) -> Option<String> {
    normalize_hostname(value).ok()
}

fn sha256(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn digest_matches(value: &[u8], expected: &[u8; 32]) -> bool {
    let supplied = sha256(value);
    bool::from(supplied.ct_eq(expected))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::*;

    #[test]
    fn public_policy_is_length_only_and_uses_fifteen_characters() {
        let policy = CredentialPolicy::cloudflare_public();
        assert!(!policy.accepts("only-fourteen!"));
        assert!(policy.accepts("fifteen-chars!!"));
        assert!(policy.accepts("纯中文密码也可以满足十五个字符长度要求"));
        assert!(!policy.accepts("fifteen-chars!\n"));
    }

    #[test]
    fn external_api_requires_exact_host_and_both_service_token_headers() {
        let policy = ExternalApiPolicy::cloudflare_service_token(
            "api.example.test",
            b"service-client-id",
            b"service-client-secret-value",
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("api.example.test"));
        headers.insert(
            CF_ACCESS_CLIENT_ID,
            HeaderValue::from_static("service-client-id"),
        );
        headers.insert(
            CF_ACCESS_CLIENT_SECRET,
            HeaderValue::from_static("service-client-secret-value"),
        );
        assert_eq!(policy.authorize(&headers), Ok(()));
        headers.insert(HOST, HeaderValue::from_static("web.example.test"));
        assert_eq!(
            policy.authorize(&headers),
            Err(ExternalApiAccessError::Denied)
        );
        assert!(!format!("{policy:?}").contains("service-client-secret"));
    }
}
