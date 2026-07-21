use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    sync::Arc,
};

use async_trait::async_trait;
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method,
        header::{AUTHORIZATION, COOKIE},
        request::Parts,
    },
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use muriarc_core::{
    Actor, ActorAccess, AiScope, AuditContext, LabRole, Permission, ProjectRole, WriteSource,
};
use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

use crate::{ApiError, AppState};

pub const SESSION_COOKIE_NAME: &str = "muriarc_session";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";
const REQUEST_ID_HEADER: &str = "x-request-id";
const AUDIT_REASON_HEADER: &str = "x-audit-reason";
const MAX_TOKEN_BYTES: usize = 4096;
const MAX_REASON_BYTES: usize = 512;
const MAX_REQUEST_ID_BYTES: usize = 128;
const DEFAULT_SESSION_HOURS: i64 = 12;
const MIN_SESSION_MINUTES: i64 = 5;
const MAX_SESSION_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct AuthPrincipal {
    pub user_id: Uuid,
    pub display_name: String,
    pub lab_id: Uuid,
    email: Option<String>,
    lab_roles: BTreeSet<LabRole>,
    project_roles: BTreeMap<Uuid, BTreeSet<ProjectRole>>,
    ai_scopes: Option<BTreeSet<AiScope>>,
    source: WriteSource,
    must_change_password: bool,
    environment_root: bool,
}

impl AuthPrincipal {
    pub fn human(
        user_id: Uuid,
        display_name: impl Into<String>,
        lab_id: Uuid,
        lab_roles: impl IntoIterator<Item = LabRole>,
    ) -> Self {
        Self {
            user_id,
            display_name: display_name.into(),
            lab_id,
            email: None,
            lab_roles: lab_roles.into_iter().collect(),
            project_roles: BTreeMap::new(),
            ai_scopes: None,
            source: WriteSource::Web,
            must_change_password: false,
            environment_root: false,
        }
    }

    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    pub fn with_project_role(mut self, project_id: Uuid, role: ProjectRole) -> Self {
        self.project_roles
            .entry(project_id)
            .or_default()
            .insert(role);
        self
    }

    /// Narrows this identity to AI/external-token scopes.
    pub fn with_ai_scopes(mut self, scopes: impl IntoIterator<Item = AiScope>) -> Self {
        self.ai_scopes = Some(scopes.into_iter().collect());
        self.source = WriteSource::Ai;
        self
    }

    pub fn with_source(mut self, source: WriteSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_credential_state(
        mut self,
        must_change_password: bool,
        environment_root: bool,
    ) -> Self {
        self.must_change_password = must_change_password;
        self.environment_root = environment_root;
        self
    }

    pub fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    pub fn lab_roles(&self) -> impl Iterator<Item = LabRole> + '_ {
        self.lab_roles.iter().copied()
    }

    pub fn project_roles(&self) -> impl Iterator<Item = (Uuid, ProjectRole)> + '_ {
        self.project_roles
            .iter()
            .flat_map(|(project_id, roles)| roles.iter().map(|role| (*project_id, *role)))
    }

    pub fn ai_scopes(&self) -> Option<impl Iterator<Item = AiScope> + '_> {
        self.ai_scopes.as_ref().map(|scopes| scopes.iter().copied())
    }

    /// MCP and other integration entry points accept only identities narrowed
    /// by explicit external-token scopes. A browser session never gains this
    /// marker implicitly.
    pub fn is_external_ai(&self) -> bool {
        self.ai_scopes.is_some()
    }

    pub fn is_lab_admin(&self) -> bool {
        self.lab_roles.contains(&LabRole::LabAdmin)
    }

    pub const fn must_change_password(&self) -> bool {
        self.must_change_password
    }

    pub const fn is_environment_root(&self) -> bool {
        self.environment_root
    }

    pub fn is_lab_operator(&self) -> bool {
        self.lab_roles.contains(&LabRole::LabAdmin)
            || self.lab_roles.contains(&LabRole::AnimalManager)
    }

    pub fn project_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.project_roles.keys().copied()
    }

    pub fn can(&self, permission: Permission, project_id: Option<Uuid>) -> bool {
        let project_roles = project_id
            .and_then(|id| self.project_roles.get(&id))
            .cloned()
            .unwrap_or_default();

        ActorAccess {
            lab_roles: self.lab_roles.clone(),
            project_roles,
            ai_scopes: self.ai_scopes.clone(),
        }
        .allows(permission)
    }

    pub fn ensure(&self, permission: Permission, project_id: Option<Uuid>) -> Result<(), ApiError> {
        if self.can(permission, project_id) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }

    pub fn audit_context(&self, metadata: &RequestMetadata) -> AuditContext {
        AuditContext {
            actor: Actor::human(self.user_id, self.display_name.clone()),
            source: self.source,
            request_id: Some(metadata.request_id.clone()),
            reason: metadata.reason.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub request_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMethod {
    Bearer,
    Session { session_id: Uuid },
}

#[derive(Clone)]
pub struct AuthenticatedSession {
    pub principal: AuthPrincipal,
    pub session_id: Uuid,
    pub csrf_hash: [u8; 32],
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for AuthenticatedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedSession")
            .field("principal", &self.principal)
            .field("session_id", &self.session_id)
            .field("csrf_hash", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct NewSession {
    pub id: Uuid,
    pub token_hash: [u8; 32],
    pub csrf_hash: [u8; 32],
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for NewSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewSession")
            .field("id", &self.id)
            .field("token_hash", &"[REDACTED]")
            .field("csrf_hash", &"[REDACTED]")
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct NewExternalToken {
    pub id: Uuid,
    pub name: String,
    pub token_hash: [u8; 32],
    pub scopes: BTreeSet<AiScope>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for NewExternalToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewExternalToken")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("token_hash", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalTokenSummary {
    pub id: Uuid,
    pub name: String,
    pub scopes: BTreeSet<AiScope>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCookieConfig {
    pub secure: bool,
    pub ttl: Duration,
}

impl Default for SessionCookieConfig {
    fn default() -> Self {
        Self {
            secure: true,
            ttl: Duration::hours(DEFAULT_SESSION_HOURS),
        }
    }
}

impl SessionCookieConfig {
    pub fn new(secure: bool, ttl: Duration) -> Result<Self, AuthError> {
        if ttl < Duration::minutes(MIN_SESSION_MINUTES) || ttl > Duration::days(MAX_SESSION_DAYS) {
            return Err(AuthError::InvalidConfiguration);
        }
        Ok(Self { secure, ttl })
    }
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, bearer_token: &str) -> Result<AuthPrincipal, AuthError>;
}

#[async_trait]
pub trait SessionBackend: Send + Sync {
    async fn login(
        &self,
        email: &str,
        password: &str,
        session: &NewSession,
    ) -> Result<AuthenticatedSession, AuthError>;

    async fn authenticate_session(
        &self,
        session_token: &str,
    ) -> Result<AuthenticatedSession, AuthError>;

    /// Re-authenticates the currently active human user for one high-risk
    /// operation. Implementations must verify the user's live credential and
    /// must never persist or log the supplied password.
    async fn verify_current_password(
        &self,
        _user_id: Uuid,
        _password: &str,
    ) -> Result<(), AuthError> {
        Err(AuthError::Unavailable)
    }

    async fn change_password(
        &self,
        _principal: &AuthPrincipal,
        _session_id: Uuid,
        _current_password: &str,
        _new_password: &str,
        _request_id: &str,
    ) -> Result<AuthPrincipal, AuthError> {
        Err(AuthError::Unavailable)
    }

    async fn update_own_display_name(
        &self,
        _principal: &AuthPrincipal,
        _display_name: &str,
        _request_id: &str,
    ) -> Result<AuthPrincipal, AuthError> {
        Err(AuthError::Unavailable)
    }

    async fn revoke_session(&self, session_id: Uuid, user_id: Uuid) -> Result<(), AuthError>;

    async fn create_external_token(
        &self,
        user_id: Uuid,
        token: &NewExternalToken,
    ) -> Result<ExternalTokenSummary, AuthError>;

    async fn list_external_tokens(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ExternalTokenSummary>, AuthError>;

    async fn revoke_external_token(&self, user_id: Uuid, token_id: Uuid) -> Result<(), AuthError>;
}

#[derive(Debug, Default)]
pub struct DisabledSessionBackend;

#[async_trait]
impl SessionBackend for DisabledSessionBackend {
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
        _session_token: &str,
    ) -> Result<AuthenticatedSession, AuthError> {
        Err(AuthError::InvalidCredentials)
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

/// Explicit token map for tests and controlled bootstrap previews.
/// No default token is ever installed.
#[derive(Clone, Default)]
pub struct StaticTokenAuthenticator {
    principals: Arc<HashMap<String, AuthPrincipal>>,
}

impl fmt::Debug for StaticTokenAuthenticator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticTokenAuthenticator")
            .field("token_count", &self.principals.len())
            .finish_non_exhaustive()
    }
}

impl StaticTokenAuthenticator {
    pub fn new(
        entries: impl IntoIterator<Item = (String, AuthPrincipal)>,
    ) -> Result<Self, AuthError> {
        let mut principals = HashMap::new();
        for (token, principal) in entries {
            validate_token(&token)?;
            if principals.insert(token, principal).is_some() {
                return Err(AuthError::DuplicateToken);
            }
        }
        Ok(Self {
            principals: Arc::new(principals),
        })
    }
}

#[async_trait]
impl Authenticator for StaticTokenAuthenticator {
    async fn authenticate(&self, bearer_token: &str) -> Result<AuthPrincipal, AuthError> {
        self.principals
            .get(bearer_token)
            .cloned()
            .ok_or(AuthError::InvalidCredentials)
    }
}

#[derive(Clone, Default)]
pub struct ChainedAuthenticator {
    providers: Arc<Vec<Arc<dyn Authenticator>>>,
}

impl ChainedAuthenticator {
    pub fn new(providers: impl IntoIterator<Item = Arc<dyn Authenticator>>) -> Self {
        Self {
            providers: Arc::new(providers.into_iter().collect()),
        }
    }
}

impl fmt::Debug for ChainedAuthenticator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChainedAuthenticator")
            .field("provider_count", &self.providers.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Authenticator for ChainedAuthenticator {
    async fn authenticate(&self, bearer_token: &str) -> Result<AuthPrincipal, AuthError> {
        for provider in self.providers.iter() {
            match provider.authenticate(bearer_token).await {
                Ok(principal) => return Ok(principal),
                Err(AuthError::InvalidCredentials) => {}
                Err(error) => return Err(error),
            }
        }
        Err(AuthError::InvalidCredentials)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthError {
    #[error("authentication is required")]
    MissingCredentials,
    #[error("authorization header is malformed")]
    MalformedBearer,
    #[error("authentication token is malformed")]
    InvalidToken,
    #[error("authentication token was configured more than once")]
    DuplicateToken,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password change is required")]
    PasswordChangeRequired,
    #[error("the environment root credential is managed by deployment configuration")]
    EnvironmentRootManaged,
    #[error("the new password must differ from the current password")]
    PasswordReuse,
    #[error("the password does not satisfy the password policy")]
    PasswordPolicy,
    #[error("the profile does not satisfy validation rules")]
    InvalidProfile,
    #[error("CSRF validation failed")]
    CsrfFailed,
    #[error("authentication service is unavailable")]
    Unavailable,
    #[error("authentication configuration is invalid")]
    InvalidConfiguration,
}

impl AuthError {
    pub(crate) fn into_api_error(self) -> ApiError {
        match self {
            Self::MissingCredentials
            | Self::MalformedBearer
            | Self::InvalidToken
            | Self::DuplicateToken
            | Self::InvalidCredentials => ApiError::unauthorized("authentication failed"),
            Self::PasswordChangeRequired => ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "password_change_required",
                "password change is required before using this resource",
            ),
            Self::EnvironmentRootManaged => ApiError::new(
                axum::http::StatusCode::CONFLICT,
                "environment_root_managed",
                "the environment root credential is managed by deployment configuration",
            ),
            Self::PasswordReuse => ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "password_reuse",
                "the new password must differ from the current password",
            ),
            Self::PasswordPolicy => ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_password",
                "passwords require at least 8 non-control characters and at most 1024 bytes",
            ),
            Self::InvalidProfile => ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_profile",
                "display name requires 1-200 non-control characters",
            ),
            Self::CsrfFailed => ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "csrf_failed",
                "CSRF validation failed",
            ),
            Self::Unavailable | Self::InvalidConfiguration => ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "authentication_unavailable",
                "authentication service is unavailable",
            ),
        }
    }
}

impl<S> FromRequestParts<S> for AuthPrincipal
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| ApiError::unauthorized("authentication middleware is not installed"))
    }
}

impl<S> FromRequestParts<S> for AuthenticationMethod
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .copied()
            .ok_or_else(ApiError::internal)
    }
}

impl<S> FromRequestParts<S> for RequestMetadata
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(ApiError::internal)
    }
}

pub(crate) async fn authenticate_request(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let metadata = request_metadata(request.headers());
    let request_id = metadata.request_id.clone();

    let (principal, method) = match bearer_token(request.headers())
        .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?
    {
        Some(token) => {
            let principal = state
                .authenticator
                .authenticate(token)
                .await
                .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?;
            (principal, AuthenticationMethod::Bearer)
        }
        None => {
            let token = session_cookie(request.headers())
                .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?
                .ok_or(AuthError::MissingCredentials)
                .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?;
            let session = state
                .sessions
                .authenticate_session(token)
                .await
                .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?;
            if is_mutation(request.method()) {
                verify_csrf(request.headers(), &session.csrf_hash)
                    .map_err(|error| error.into_api_error().with_request_id(request_id.clone()))?;
            }
            let method = AuthenticationMethod::Session {
                session_id: session.session_id,
            };
            (session.principal, method)
        }
    };

    request.extensions_mut().insert(principal);
    request.extensions_mut().insert(method);
    request.extensions_mut().insert(metadata);

    let response = next.run(request).await;
    Ok(with_request_id(response, &request_id))
}

pub(crate) async fn require_password_ready(
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let principal = request
        .extensions()
        .get::<AuthPrincipal>()
        .ok_or_else(ApiError::internal)?;
    if principal.must_change_password() {
        let request_id = request
            .extensions()
            .get::<RequestMetadata>()
            .map(|metadata| metadata.request_id.clone());
        let mut error = AuthError::PasswordChangeRequired.into_api_error();
        if let Some(request_id) = request_id {
            error = error.with_request_id(request_id);
        }
        return Err(error);
    }
    Ok(next.run(request).await)
}

pub(crate) fn request_metadata(headers: &HeaderMap) -> RequestMetadata {
    RequestMetadata {
        request_id: request_id(headers),
        reason: audit_reason(headers),
    }
}

pub(crate) fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
    }
    response
}

pub fn generate_secret_token(prefix: &str) -> Result<String, AuthError> {
    if prefix.is_empty()
        || prefix.len() > 16
        || !prefix
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(AuthError::InvalidConfiguration);
    }
    let mut random = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut random)
        .map_err(|_| AuthError::Unavailable)?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(random)))
}

pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Derives the browser CSRF token from the high-entropy session token.
///
/// The server persists only the SHA-256 digest of both values. Derivation lets
/// an authenticated browser recover its CSRF token after a page reload without
/// persisting the raw token, rotating it underneath another open tab, or
/// weakening the HttpOnly session cookie. The domain separator prevents the
/// derived value from being confused with a session credential.
pub fn derive_csrf_token(session_token: &str) -> Result<String, AuthError> {
    validate_token(session_token)?;
    let mut hasher = Sha256::new();
    hasher.update(b"muriarc-csrf-v1\0");
    hasher.update(session_token.as_bytes());
    Ok(format!("mac_{}", URL_SAFE_NO_PAD.encode(hasher.finalize())))
}

fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, AuthError> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(AuthError::MalformedBearer);
    }
    let value = value.to_str().map_err(|_| AuthError::MalformedBearer)?;
    let (scheme, token) = value.split_once(' ').ok_or(AuthError::MalformedBearer)?;

    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(AuthError::MalformedBearer);
    }
    validate_token(token)?;
    Ok(Some(token))
}

pub(crate) fn session_cookie(headers: &HeaderMap) -> Result<Option<&str>, AuthError> {
    let mut found = None;
    for header in headers.get_all(COOKIE) {
        let header = header.to_str().map_err(|_| AuthError::InvalidToken)?;
        for pair in header.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE_NAME && found.replace(value).is_some() {
                return Err(AuthError::InvalidToken);
            }
        }
    }
    if let Some(token) = found {
        validate_token(token)?;
    }
    Ok(found)
}

fn verify_csrf(headers: &HeaderMap, expected_hash: &[u8; 32]) -> Result<(), AuthError> {
    let mut values = headers
        .get_all(HeaderName::from_static(CSRF_HEADER_NAME))
        .iter();
    let token = values
        .next()
        .ok_or(AuthError::CsrfFailed)?
        .to_str()
        .map_err(|_| AuthError::CsrfFailed)?;
    if values.next().is_some() {
        return Err(AuthError::CsrfFailed);
    }
    validate_token(token).map_err(|_| AuthError::CsrfFailed)?;
    let supplied = token_hash(token);
    if supplied.ct_eq(expected_hash).into() {
        Ok(())
    } else {
        Err(AuthError::CsrfFailed)
    }
}

fn validate_token(token: &str) -> Result<(), AuthError> {
    if token.is_empty()
        || token.len() > MAX_TOKEN_BYTES
        || token.chars().any(char::is_whitespace)
        || token.chars().any(char::is_control)
    {
        Err(AuthError::InvalidToken)
    } else {
        Ok(())
    }
}

fn is_mutation(method: &Method) -> bool {
    !matches!(
        method,
        &Method::GET | &Method::HEAD | &Method::OPTIONS | &Method::TRACE
    )
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_REQUEST_ID_BYTES
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "-_.:".contains(character)
                })
        })
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

fn audit_reason(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUDIT_REASON_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= MAX_REASON_BYTES)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn viewer(project_id: Uuid) -> AuthPrincipal {
        AuthPrincipal::human(Uuid::new_v4(), "Viewer", Uuid::new_v4(), [])
            .with_project_role(project_id, ProjectRole::Viewer)
    }

    #[test]
    fn project_role_is_scoped_to_its_project() {
        let allowed_project = Uuid::new_v4();
        let other_project = Uuid::new_v4();
        let principal = viewer(allowed_project);

        assert!(principal.can(Permission::ReadAnimal, Some(allowed_project)));
        assert!(!principal.can(Permission::ReadAnimal, Some(other_project)));
        assert!(!principal.can(Permission::ReadAnimal, None));
    }

    #[test]
    fn ai_scope_narrows_human_permission() {
        let project_id = Uuid::new_v4();
        let principal = AuthPrincipal::human(Uuid::new_v4(), "Editor", Uuid::new_v4(), [])
            .with_project_role(project_id, ProjectRole::Editor)
            .with_ai_scopes([AiScope::Read]);

        assert!(principal.can(Permission::ReadMeasurement, Some(project_id)));
        assert!(!principal.can(Permission::WriteMeasurementDraft, Some(project_id)));
    }

    #[test]
    fn bearer_parser_rejects_non_bearer_and_whitespace() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer_token(&headers), Err(AuthError::MalformedBearer));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer a b"));
        assert_eq!(bearer_token(&headers), Err(AuthError::InvalidToken));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer first"));
        headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer second"));
        assert_eq!(bearer_token(&headers), Err(AuthError::MalformedBearer));
    }

    #[test]
    fn cookie_parser_rejects_duplicate_session_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("muriarc_session=one; muriarc_session=two"),
        );
        assert_eq!(session_cookie(&headers), Err(AuthError::InvalidToken));
    }

    #[test]
    fn csrf_comparison_uses_the_stored_digest() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(CSRF_HEADER_NAME),
            HeaderValue::from_static("csrf_test_value"),
        );
        assert!(verify_csrf(&headers, &token_hash("csrf_test_value")).is_ok());
        assert_eq!(
            verify_csrf(&headers, &token_hash("another_value")),
            Err(AuthError::CsrfFailed)
        );
    }

    #[tokio::test]
    async fn static_authenticator_has_no_implicit_default_token() {
        let auth = StaticTokenAuthenticator::default();
        assert_eq!(
            auth.authenticate("development").await.unwrap_err(),
            AuthError::InvalidCredentials
        );
    }

    #[test]
    fn generated_tokens_have_high_entropy_and_are_not_reused() {
        let first = generate_secret_token("mas_").unwrap();
        let second = generate_secret_token("mas_").unwrap();
        assert!(first.len() >= 47);
        assert_ne!(first, second);
        assert_ne!(token_hash(&first), token_hash(&second));
    }

    #[test]
    fn csrf_derivation_is_stable_and_session_scoped() {
        let first_session = generate_secret_token("mas_").unwrap();
        let second_session = generate_secret_token("mas_").unwrap();
        let first = derive_csrf_token(&first_session).unwrap();
        assert!(first.starts_with("mac_"));
        assert_eq!(derive_csrf_token(&first_session).unwrap(), first);
        assert_ne!(derive_csrf_token(&second_session).unwrap(), first);
        assert_eq!(derive_csrf_token("bad token"), Err(AuthError::InvalidToken));
    }

    #[test]
    fn session_debug_redacts_security_digests() {
        let session = AuthenticatedSession {
            principal: AuthPrincipal::human(Uuid::new_v4(), "User", Uuid::new_v4(), []),
            session_id: Uuid::new_v4(),
            csrf_hash: [7; 32],
            expires_at: Utc::now(),
        };
        let rendered = format!("{session:?}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains("7, 7, 7"));
    }

    #[test]
    fn invalid_client_request_id_is_replaced() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(REQUEST_ID_HEADER),
            HeaderValue::from_static("../../unsafe"),
        );
        assert_ne!(request_id(&headers), "../../unsafe");
    }
}
