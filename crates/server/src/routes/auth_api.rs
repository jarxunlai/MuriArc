use std::collections::BTreeSet;

use axum::{
    Router,
    extract::State,
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, SET_COOKIE},
    },
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::{Duration, Utc};
use muriarc_core::{AiScope, LabRole, ProjectRole};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApiJson, ApiPath, item};
use crate::{
    ApiError, AppState, AuthPrincipal, AuthenticationMethod, ExternalTokenSummary,
    NewExternalToken, NewSession, RequestMetadata, SESSION_COOKIE_NAME, derive_csrf_token,
    generate_secret_token, token_hash,
};

const MAX_EMAIL_BYTES: usize = 320;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_TOKEN_NAME_BYTES: usize = 120;
const MAX_EXTERNAL_TOKEN_DAYS: u16 = 365;
const DEFAULT_EXTERNAL_TOKEN_DAYS: u16 = 90;

pub(super) fn public_router() -> Router<AppState> {
    Router::new().route("/auth/login", post(login))
}

pub(super) fn credential_router() -> Router<AppState> {
    Router::new()
        .route("/auth/session", get(me))
        // Compatibility alias retained for early API clients.
        .route("/auth/me", get(me))
        .route("/auth/csrf", get(recover_csrf))
        .route("/auth/logout", post(logout))
        .route("/auth/password/change", post(change_password))
}

pub(super) fn protected_router() -> Router<AppState> {
    Router::new()
        .route("/auth/profile", patch(update_profile))
        .route("/auth/tokens", get(list_tokens).post(create_token))
        // Dedicated POST avoids relying on intermediaries to preserve a body
        // on DELETE while retaining both password step-up and CSRF.
        .route("/auth/tokens/{id}/revoke", post(revoke_token))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user: AuthUserResponse,
    csrf_token: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Serialize)]
struct CsrfResponse {
    csrf_token: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct AuthUserResponse {
    id: Uuid,
    lab_id: Uuid,
    email: Option<String>,
    display_name: String,
    lab_roles: Vec<LabRole>,
    project_roles: Vec<ProjectRoleGrant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_scopes: Option<Vec<AiScope>>,
    authentication: &'static str,
    must_change_password: bool,
    is_environment_root: bool,
}

#[derive(Debug, Serialize)]
struct ProjectRoleGrant {
    project_id: Uuid,
    role: ProjectRole,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProfileRequest {
    display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTokenRequest {
    name: String,
    scopes: BTreeSet<AiScope>,
    expires_in_days: Option<u16>,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeTokenRequest {
    current_password: String,
}

#[derive(Serialize)]
struct IssuedExternalToken {
    /// Returned once. Only its SHA-256 digest is persisted.
    token: String,
    details: ExternalTokenSummary,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let metadata = crate::auth::request_metadata(&headers);
    if payload.email.trim().is_empty()
        || payload.email.len() > MAX_EMAIL_BYTES
        || payload.password.len() > MAX_PASSWORD_BYTES
    {
        return Err(
            ApiError::unauthorized("authentication failed").with_request_id(metadata.request_id)
        );
    }

    let session_token =
        generate_secret_token("mas_").map_err(|error| auth_error(error, &metadata))?;
    let csrf_token =
        derive_csrf_token(&session_token).map_err(|error| auth_error(error, &metadata))?;
    let now = Utc::now();
    let session = NewSession {
        id: Uuid::new_v4(),
        token_hash: token_hash(&session_token),
        csrf_hash: token_hash(&csrf_token),
        created_at: now,
        expires_at: now + state.session_cookie.ttl,
    };
    let authenticated = state
        .sessions
        .login(&payload.email, &payload.password, &session)
        .await
        .map_err(|error| auth_error(error, &metadata))?;

    let body = LoginResponse {
        user: user_response(&authenticated.principal, "session"),
        csrf_token,
        expires_at: authenticated.expires_at,
    };
    let mut response = item(body, &metadata).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        cookie_header(
            &session_token,
            state.session_cookie.ttl,
            state.session_cookie.secure,
        )?,
    );
    no_store(&mut response);
    Ok(crate::auth::with_request_id(response, &metadata.request_id))
}

async fn me(
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Response {
    let authentication = match method {
        AuthenticationMethod::Bearer => "bearer",
        AuthenticationMethod::Session { .. } => "session",
    };
    let mut response = item(user_response(&principal, authentication), &metadata).into_response();
    no_store(&mut response);
    response
}

/// Restores the synchronizer token after a browser reload.
///
/// This is a safe GET and never accepts bearer authentication. The token is
/// deterministically derived from the HttpOnly session secret, while the
/// database retains only its digest. Consequently no raw CSRF token is stored,
/// and multiple tabs cannot invalidate each other through token rotation.
async fn recover_csrf(
    State(state): State<AppState>,
    headers: HeaderMap,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Response, ApiError> {
    let AuthenticationMethod::Session { session_id } = method else {
        return Err(ApiError::unauthorized("a browser session is required")
            .with_request_id(metadata.request_id));
    };
    let session_token = crate::auth::session_cookie(&headers)
        .map_err(|error| auth_error(error, &metadata))?
        .ok_or_else(|| {
            ApiError::unauthorized("a browser session is required")
                .with_request_id(metadata.request_id.clone())
        })?;
    let authenticated = state
        .sessions
        .authenticate_session(session_token)
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    if authenticated.session_id != session_id
        || authenticated.principal.user_id != principal.user_id
        || authenticated.principal.lab_id != principal.lab_id
    {
        return Err(
            ApiError::unauthorized("authentication failed").with_request_id(metadata.request_id)
        );
    }
    let csrf_token =
        derive_csrf_token(session_token).map_err(|error| auth_error(error, &metadata))?;
    if token_hash(&csrf_token) != authenticated.csrf_hash {
        return Err(
            ApiError::unauthorized("authentication failed").with_request_id(metadata.request_id)
        );
    }

    let mut response = item(
        CsrfResponse {
            csrf_token,
            expires_at: authenticated.expires_at,
        },
        &metadata,
    )
    .into_response();
    no_store(&mut response);
    Ok(response)
}

async fn logout(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Response, ApiError> {
    let AuthenticationMethod::Session { session_id } = method else {
        return Err(ApiError::validation("logout requires a browser session")
            .with_request_id(metadata.request_id));
    };
    state
        .sessions
        .revoke_session(session_id, principal.user_id)
        .await
        .map_err(|error| auth_error(error, &metadata))?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        clear_cookie_header(state.session_cookie.secure)?,
    );
    no_store(&mut response);
    Ok(response)
}

async fn change_password(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    let AuthenticationMethod::Session { session_id } = method else {
        return Err(ApiError::forbidden().with_request_id(metadata.request_id));
    };
    let updated = state
        .sessions
        .change_password(
            &principal,
            session_id,
            &payload.current_password,
            &payload.new_password,
            &metadata.request_id,
        )
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    let mut response = item(user_response(&updated, "session"), &metadata).into_response();
    no_store(&mut response);
    Ok(response)
}

async fn update_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<UpdateProfileRequest>,
) -> Result<Response, ApiError> {
    ensure_browser_session(method, &metadata)?;
    let updated = state
        .sessions
        .update_own_display_name(&principal, &payload.display_name, &metadata.request_id)
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    let mut response = item(user_response(&updated, "session"), &metadata).into_response();
    no_store(&mut response);
    Ok(response)
}

async fn create_token(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateTokenRequest>,
) -> Result<Response, ApiError> {
    ensure_browser_session(method, &metadata)?;
    ensure_external_api_enabled(&state, &metadata)?;
    state
        .sessions
        .verify_current_password(principal.user_id, &payload.current_password)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "step_up_failed",
                "current password verification failed",
            )
            .with_request_id(metadata.request_id.clone())
        })?;
    let name = payload.name.trim().to_owned();
    if name.is_empty()
        || name.len() > MAX_TOKEN_NAME_BYTES
        || name.chars().any(char::is_control)
        || payload.scopes.is_empty()
    {
        return Err(ApiError::validation("token name and scopes are required")
            .with_request_id(metadata.request_id));
    }
    let days = payload
        .expires_in_days
        .unwrap_or(DEFAULT_EXTERNAL_TOKEN_DAYS);
    if days == 0 || days > MAX_EXTERNAL_TOKEN_DAYS {
        return Err(
            ApiError::validation("token expiry must be between 1 and 365 days")
                .with_request_id(metadata.request_id),
        );
    }

    let raw_token = generate_secret_token("mat_").map_err(|error| auth_error(error, &metadata))?;
    let now = Utc::now();
    let token = NewExternalToken {
        id: Uuid::new_v4(),
        name,
        token_hash: token_hash(&raw_token),
        scopes: payload.scopes,
        created_at: now,
        expires_at: now + Duration::days(i64::from(days)),
    };
    let details = state
        .sessions
        .create_external_token(principal.user_id, &token)
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    let mut response = item(
        IssuedExternalToken {
            token: raw_token,
            details,
        },
        &metadata,
    )
    .into_response();
    no_store(&mut response);
    Ok(response)
}

async fn list_tokens(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Response, ApiError> {
    ensure_browser_session(method, &metadata)?;
    let tokens = state
        .sessions
        .list_external_tokens(principal.user_id)
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    let mut response = super::collection(tokens, &metadata).into_response();
    no_store(&mut response);
    Ok(response)
}

async fn revoke_token(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    method: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(token_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<RevokeTokenRequest>,
) -> Result<StatusCode, ApiError> {
    ensure_browser_session(method, &metadata)?;
    state
        .sessions
        .verify_current_password(principal.user_id, &payload.current_password)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "step_up_failed",
                "current password verification failed",
            )
            .with_request_id(metadata.request_id.clone())
        })?;
    state
        .sessions
        .revoke_external_token(principal.user_id, token_id)
        .await
        .map_err(|error| auth_error(error, &metadata))?;
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_external_api_enabled(
    state: &AppState,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if state.deployment_security.external_api().enabled() {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::CONFLICT,
            "external_api_disabled",
            "external API access must be enabled by the deployment operator before issuing tokens",
        )
        .with_request_id(metadata.request_id.clone()))
    }
}

fn ensure_browser_session(
    method: AuthenticationMethod,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if matches!(method, AuthenticationMethod::Session { .. }) {
        Ok(())
    } else {
        Err(ApiError::forbidden().with_request_id(metadata.request_id.clone()))
    }
}

fn user_response(principal: &AuthPrincipal, authentication: &'static str) -> AuthUserResponse {
    AuthUserResponse {
        id: principal.user_id,
        lab_id: principal.lab_id,
        email: principal.email().map(str::to_owned),
        display_name: principal.display_name.clone(),
        lab_roles: principal.lab_roles().collect(),
        project_roles: principal
            .project_roles()
            .map(|(project_id, role)| ProjectRoleGrant { project_id, role })
            .collect(),
        ai_scopes: principal.ai_scopes().map(Iterator::collect),
        authentication,
        must_change_password: principal.must_change_password(),
        is_environment_root: principal.is_environment_root(),
    }
}

fn cookie_header(token: &str, ttl: Duration, secure: bool) -> Result<HeaderValue, ApiError> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        ttl.num_seconds()
    ))
    .map_err(|_| ApiError::internal())
}

fn clear_cookie_header(secure: bool) -> Result<HeaderValue, ApiError> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT{secure}"
    ))
    .map_err(|_| ApiError::internal())
}

fn no_store(response: &mut Response) {
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, private, max-age=0"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("pragma"),
        HeaderValue::from_static("no-cache"),
    );
}

fn auth_error(error: crate::AuthError, metadata: &RequestMetadata) -> ApiError {
    error
        .into_api_error()
        .with_request_id(metadata.request_id.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use muriarc_store_sqlite::SqliteStore;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        AuthError, AuthenticatedSession, DeploymentSecurityPolicy, ExternalApiPolicy,
        InMemoryJobRepository, SessionBackend, SessionCookieConfig, StaticTokenAuthenticator,
        api_router,
    };

    #[derive(Default)]
    struct TestSessionBackend {
        session: Mutex<Option<(NewSession, AuthPrincipal)>>,
        external_tokens: Mutex<Vec<(Uuid, NewExternalToken)>>,
        revoked: AtomicBool,
        must_change_password: AtomicBool,
    }

    #[async_trait]
    impl SessionBackend for TestSessionBackend {
        async fn login(
            &self,
            email: &str,
            password: &str,
            session: &NewSession,
        ) -> Result<AuthenticatedSession, AuthError> {
            if email != "researcher@example.org" || password != "correct horse battery staple" {
                return Err(AuthError::InvalidCredentials);
            }
            let principal = AuthPrincipal::human(
                Uuid::new_v4(),
                "Researcher",
                Uuid::new_v4(),
                [LabRole::LabAdmin],
            )
            .with_email(email)
            .with_credential_state(self.must_change_password.load(Ordering::SeqCst), false);
            *self.session.lock().unwrap() = Some((session.clone(), principal.clone()));
            Ok(AuthenticatedSession {
                principal,
                session_id: session.id,
                csrf_hash: session.csrf_hash,
                expires_at: session.expires_at,
            })
        }

        async fn authenticate_session(
            &self,
            session_token: &str,
        ) -> Result<AuthenticatedSession, AuthError> {
            if self.revoked.load(Ordering::SeqCst) {
                return Err(AuthError::InvalidCredentials);
            }
            let guard = self.session.lock().unwrap();
            let (session, principal) = guard.as_ref().ok_or(AuthError::InvalidCredentials)?;
            if token_hash(session_token) != session.token_hash {
                return Err(AuthError::InvalidCredentials);
            }
            Ok(AuthenticatedSession {
                principal: principal
                    .clone()
                    .with_credential_state(self.must_change_password.load(Ordering::SeqCst), false),
                session_id: session.id,
                csrf_hash: session.csrf_hash,
                expires_at: session.expires_at,
            })
        }

        async fn change_password(
            &self,
            principal: &AuthPrincipal,
            session_id: Uuid,
            current_password: &str,
            new_password: &str,
            _request_id: &str,
        ) -> Result<AuthPrincipal, AuthError> {
            if current_password != "correct horse battery staple"
                || new_password.chars().count() < 8
                || new_password.chars().any(char::is_control)
                || new_password == current_password
            {
                return Err(AuthError::InvalidCredentials);
            }
            let mut guard = self.session.lock().unwrap();
            let (session, stored) = guard.as_mut().ok_or(AuthError::InvalidCredentials)?;
            if session.id != session_id || stored.user_id != principal.user_id {
                return Err(AuthError::InvalidCredentials);
            }
            self.must_change_password.store(false, Ordering::SeqCst);
            *stored = stored.clone().with_credential_state(false, false);
            Ok(stored.clone())
        }

        async fn verify_current_password(
            &self,
            _user_id: Uuid,
            password: &str,
        ) -> Result<(), AuthError> {
            if password == "correct horse battery staple" {
                Ok(())
            } else {
                Err(AuthError::InvalidCredentials)
            }
        }

        async fn revoke_session(&self, session_id: Uuid, user_id: Uuid) -> Result<(), AuthError> {
            let guard = self.session.lock().unwrap();
            let (session, principal) = guard.as_ref().ok_or(AuthError::InvalidCredentials)?;
            if session.id != session_id || principal.user_id != user_id {
                return Err(AuthError::InvalidCredentials);
            }
            self.revoked.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn create_external_token(
            &self,
            user_id: Uuid,
            token: &NewExternalToken,
        ) -> Result<ExternalTokenSummary, AuthError> {
            self.external_tokens
                .lock()
                .unwrap()
                .push((user_id, token.clone()));
            Ok(ExternalTokenSummary {
                id: token.id,
                name: token.name.clone(),
                scopes: token.scopes.clone(),
                created_at: token.created_at,
                expires_at: token.expires_at,
                last_used_at: None,
                revoked_at: None,
            })
        }

        async fn list_external_tokens(
            &self,
            user_id: Uuid,
        ) -> Result<Vec<ExternalTokenSummary>, AuthError> {
            Ok(self
                .external_tokens
                .lock()
                .unwrap()
                .iter()
                .filter(|(owner, _)| *owner == user_id)
                .map(|(_, token)| ExternalTokenSummary {
                    id: token.id,
                    name: token.name.clone(),
                    scopes: token.scopes.clone(),
                    created_at: token.created_at,
                    expires_at: token.expires_at,
                    last_used_at: None,
                    revoked_at: None,
                })
                .collect())
        }

        async fn revoke_external_token(
            &self,
            user_id: Uuid,
            token_id: Uuid,
        ) -> Result<(), AuthError> {
            let mut tokens = self.external_tokens.lock().unwrap();
            let original_len = tokens.len();
            tokens.retain(|(owner, token)| !(*owner == user_id && token.id == token_id));
            if tokens.len() == original_len {
                Err(AuthError::InvalidCredentials)
            } else {
                Ok(())
            }
        }
    }

    async fn test_app() -> Router {
        test_app_with_backend(Arc::new(TestSessionBackend::default())).await
    }

    async fn test_app_with_backend(backend: Arc<TestSessionBackend>) -> Router {
        test_app_with_security(backend, DeploymentSecurityPolicy::development_default()).await
    }

    async fn test_app_with_security(
        backend: Arc<TestSessionBackend>,
        security: DeploymentSecurityPolicy,
    ) -> Router {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        muriarc_core::MuriArcStore::migrate(store.as_ref())
            .await
            .unwrap();
        let state = AppState::new(
            store,
            Arc::new(StaticTokenAuthenticator::default()),
            Arc::new(InMemoryJobRepository::default()),
        )
        .with_sessions(
            backend,
            SessionCookieConfig::new(false, Duration::hours(12)).unwrap(),
        )
        .with_deployment_security(security);
        api_router(state)
    }

    async fn json_body(response: Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn production_cookie_is_http_only_strict_and_secure() {
        let cookie = cookie_header("mas_test", Duration::hours(12), true)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Secure"));
        assert!(!cookie.contains("Domain="));
    }

    #[test]
    fn cleared_cookie_preserves_security_attributes() {
        let cookie = clear_cookie_header(true).unwrap();
        let cookie = cookie.to_str().unwrap();
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn disabled_external_api_unmounts_mcp_and_rejects_bearer_before_authentication() {
        let app = test_app_with_security(
            Arc::new(TestSessionBackend::default()),
            DeploymentSecurityPolicy::private(ExternalApiPolicy::disabled()),
        )
        .await;

        let capabilities = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime/capabilities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(capabilities.status(), StatusCode::OK);
        assert_eq!(
            capabilities.headers().get(CACHE_CONTROL).unwrap(),
            "private, no-store"
        );
        let capabilities = json_body(capabilities).await;
        assert_eq!(capabilities["data"]["external_api_enabled"], false);
        assert_eq!(capabilities["data"]["mcp_enabled"], false);

        let mcp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp.status(), StatusCode::NOT_FOUND);

        let bearer = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/projects")
                    .header("authorization", "Bearer mat_disabled_external_api")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bearer.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(bearer).await["error"]["code"],
            "external_api_disabled"
        );
    }

    #[tokio::test]
    async fn login_session_and_logout_enforce_the_cookie_csrf_contract() {
        let app = test_app().await;
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "researcher@example.org",
                            "password": "correct horse battery staple"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(cookie.starts_with("muriarc_session=mas_"));
        let login = json_body(login).await;
        let csrf = login["data"]["csrf_token"].as_str().unwrap().to_owned();

        let session = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session.status(), StatusCode::OK);
        assert_eq!(
            json_body(session).await["data"]["authentication"],
            "session"
        );

        // A full page reload loses in-memory state but retains the HttpOnly
        // cookie. The safe recovery endpoint restores the same synchronizer
        // token without a password prompt or cross-tab rotation.
        let recovered = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/csrf")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(recovered.status(), StatusCode::OK);
        assert!(
            recovered
                .headers()
                .get(CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap()
                .split(',')
                .any(|directive| directive.trim() == "no-store")
        );
        let recovered = json_body(recovered).await;
        assert_eq!(recovered["data"]["csrf_token"], csrf);
        let csrf = recovered["data"]["csrf_token"].as_str().unwrap().to_owned();

        let issued = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/tokens")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(crate::auth::CSRF_HEADER_NAME, &csrf)
                    .body(Body::from(
                        json!({
                            "name": "Read-only integration",
                            "scopes": ["read"],
                            "expires_in_days": 30,
                            "current_password": "correct horse battery staple"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::OK);
        let issued = json_body(issued).await;
        let raw_external = issued["data"]["token"].as_str().unwrap().to_owned();
        let external_id = issued["data"]["details"]["id"].as_str().unwrap().to_owned();
        assert!(raw_external.starts_with("mat_"));

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/tokens")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = json_body(listed).await;
        assert_eq!(listed["count"], json!(1));
        assert!(!listed.to_string().contains(&raw_external));

        let wrong_step_up = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/auth/tokens/{external_id}/revoke"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(crate::auth::CSRF_HEADER_NAME, &csrf)
                    .body(Body::from(
                        json!({"current_password": "wrong password"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_step_up.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(wrong_step_up).await["error"]["code"],
            "step_up_failed"
        );

        let revoked_external = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/auth/tokens/{external_id}/revoke"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(crate::auth::CSRF_HEADER_NAME, &csrf)
                    .body(Body::from(
                        json!({"current_password": "correct horse battery staple"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked_external.status(), StatusCode::NO_CONTENT);

        let listed_after_revoke = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/tokens")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed_after_revoke.status(), StatusCode::OK);
        assert_eq!(json_body(listed_after_revoke).await["count"], json!(0));

        let missing_csrf = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(missing_csrf).await["error"]["code"],
            "csrf_failed"
        );

        let logout = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header("cookie", &cookie)
                    .header(crate::auth::CSRF_HEADER_NAME, csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        assert!(
            logout
                .headers()
                .get(SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );

        let revoked = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn csrf_recovery_requires_a_live_cookie_session() {
        let app = test_app().await;
        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/csrf")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn forced_password_change_allows_only_credential_lifecycle_routes() {
        let backend = Arc::new(TestSessionBackend::default());
        backend.must_change_password.store(true, Ordering::SeqCst);
        let app = test_app_with_backend(backend).await;
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "researcher@example.org",
                            "password": "correct horse battery staple"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let login = json_body(login).await;
        assert_eq!(login["data"]["user"]["must_change_password"], true);
        let csrf = login["data"]["csrf_token"].as_str().unwrap();

        for path in ["/api/v1/projects", "/api/v1/auth/tokens"] {
            let blocked = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                json_body(blocked).await["error"]["code"],
                "password_change_required"
            );
        }

        let session = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session.status(), StatusCode::OK);

        let missing_csrf = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/password/change")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "currentPassword": "correct horse battery staple",
                            "newPassword": "replacement password"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(missing_csrf).await["error"]["code"],
            "csrf_failed"
        );

        let changed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/password/change")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(crate::auth::CSRF_HEADER_NAME, csrf)
                    .body(Body::from(
                        json!({
                            "currentPassword": "correct horse battery staple",
                            "newPassword": "replacement password"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changed.status(), StatusCode::OK);
        let changed = json_body(changed).await;
        assert_eq!(changed["data"]["must_change_password"], false);
        let encoded = changed.to_string();
        assert!(!encoded.contains("correct horse battery staple"));
        assert!(!encoded.contains("replacement password"));

        let ready = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/projects")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::OK);
    }
}
