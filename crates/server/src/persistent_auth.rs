use std::{collections::BTreeSet, sync::Arc};

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use muriarc_core::{AiScope, MembershipFilter, MuriArcStore, StoreError, UserStatus, WriteSource};
use muriarc_store_postgres::PostgresStore;
use rand::rngs::OsRng;
use ring::hmac;
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AuthError, AuthPrincipal, AuthenticatedSession, Authenticator, ExternalTokenSummary,
    NewExternalToken, NewSession, SessionBackend, StaticTokenAuthenticator,
    deployment_security::{CredentialPolicy, PASSWORD_MAX_BYTES},
    token_hash,
};

const EMAIL_MAX_BYTES: usize = 320;
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;
const LOGIN_FAILURE_WINDOW_HOURS: i64 = 24;
const LOGIN_BACKOFF_RETENTION_DAYS: i64 = 30;
const LOGIN_BACKOFF_CLEANUP_BATCH: i64 = 500;
const LOGIN_BACKOFF_LOCK_NAMESPACE: i64 = 0x4d_75_72_69_41_72_63_4c;

#[derive(Clone)]
pub struct PostgresAuthBackend {
    store: Arc<dyn MuriArcStore>,
    postgres: PostgresStore,
    lab_id: Uuid,
    environment_root_user_id: Uuid,
    dummy_password_hash: Arc<str>,
    credential_policy: CredentialPolicy,
    login_identity_hmac_key: Option<Arc<[u8; 32]>>,
}

struct PasswordRecord {
    user_id: Uuid,
    password_hash: String,
    credential_policy_revision: i32,
}

struct VerifiedLogin {
    user_id: Uuid,
    policy_upgrade_required: bool,
}

#[derive(Clone)]
pub struct LiveBootstrapAuthenticator {
    bootstrap: StaticTokenAuthenticator,
    backend: PostgresAuthBackend,
}

impl std::fmt::Debug for LiveBootstrapAuthenticator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveBootstrapAuthenticator")
            .field("bootstrap", &self.bootstrap)
            .field("backend", &self.backend)
            .finish()
    }
}

impl LiveBootstrapAuthenticator {
    pub fn new(bootstrap: StaticTokenAuthenticator, backend: PostgresAuthBackend) -> Self {
        Self { bootstrap, backend }
    }
}

#[async_trait]
impl Authenticator for LiveBootstrapAuthenticator {
    async fn authenticate(&self, bearer_token: &str) -> Result<AuthPrincipal, AuthError> {
        let configured = self.bootstrap.authenticate(bearer_token).await?;
        let scopes = configured.ai_scopes().map(Iterator::collect);
        let source = if scopes.is_some() {
            WriteSource::Mcp
        } else {
            WriteSource::Api
        };
        self.backend
            .principal_for(configured.user_id, scopes, source)
            .await
    }
}

impl std::fmt::Debug for PostgresAuthBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresAuthBackend")
            .field("lab_id", &self.lab_id)
            .field("environment_root_user_id", &self.environment_root_user_id)
            .field("dummy_password_hash", &"[REDACTED]")
            .field("credential_policy", &self.credential_policy)
            .field("login_identity_hmac_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl PostgresAuthBackend {
    pub fn new(
        postgres: PostgresStore,
        store: Arc<dyn MuriArcStore>,
        lab_id: Uuid,
        environment_root_user_id: Uuid,
    ) -> Result<Self, AuthError> {
        Self::new_with_policy(
            postgres,
            store,
            lab_id,
            environment_root_user_id,
            CredentialPolicy::private(),
            None,
        )
    }

    pub fn new_with_policy(
        postgres: PostgresStore,
        store: Arc<dyn MuriArcStore>,
        lab_id: Uuid,
        environment_root_user_id: Uuid,
        credential_policy: CredentialPolicy,
        login_identity_hmac_key: Option<[u8; 32]>,
    ) -> Result<Self, AuthError> {
        let dummy_password_hash =
            hash_password_with_policy("MuriArc invalid credential placeholder", credential_policy)?;
        Ok(Self {
            store,
            postgres,
            lab_id,
            environment_root_user_id,
            dummy_password_hash: dummy_password_hash.into(),
            credential_policy,
            login_identity_hmac_key: login_identity_hmac_key.map(Arc::new),
        })
    }

    async fn principal_for(
        &self,
        user_id: Uuid,
        scopes: Option<BTreeSet<AiScope>>,
        source: WriteSource,
    ) -> Result<AuthPrincipal, AuthError> {
        let user = self.store.get_user(user_id).await.map_err(map_store)?;
        if user.lab_id != self.lab_id
            || user.status != UserStatus::Active
            || user.meta.deleted_at.is_some()
        {
            return Err(AuthError::InvalidCredentials);
        }

        let must_change_password: bool = sqlx::query_scalar(
            "SELECT must_change_password FROM user_credentials WHERE user_id = $1",
        )
        .bind(user.id)
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?
        .ok_or(AuthError::InvalidCredentials)?;

        if scopes.is_some() && must_change_password {
            return Err(AuthError::PasswordChangeRequired);
        }

        let memberships = self
            .store
            .list_memberships(&MembershipFilter {
                lab_id: user.lab_id,
                user_id: Some(user.id),
                project_id: None,
            })
            .await
            .map_err(map_store)?;
        let lab_roles = memberships.iter().filter_map(|membership| {
            (membership.project_id.is_none())
                .then_some(membership.lab_role)
                .flatten()
        });
        let mut principal =
            AuthPrincipal::human(user.id, user.display_name, user.lab_id, lab_roles)
                .with_email(user.email)
                .with_source(source)
                .with_credential_state(
                    must_change_password,
                    user.id == self.environment_root_user_id,
                );
        for membership in memberships {
            if let (Some(project_id), Some(role)) = (membership.project_id, membership.project_role)
            {
                principal = principal.with_project_role(project_id, role);
            }
        }
        if let Some(scopes) = scopes {
            principal = principal.with_ai_scopes(scopes).with_source(source);
        }
        Ok(principal)
    }

    async fn password_record(&self, email: &str) -> Result<Option<PasswordRecord>, AuthError> {
        let email = normalize_email(email);
        if email.is_empty() || email.len() > EMAIL_MAX_BYTES || !email.contains('@') {
            return Ok(None);
        }
        let rows = sqlx::query(
            "SELECT c.user_id, c.password_hash, c.credential_policy_revision FROM user_credentials c JOIN users u ON u.id = c.user_id WHERE u.lab_id = $1 AND lower(u.email) = $2 AND u.status = 'active' AND u.deleted_at IS NULL ORDER BY u.id LIMIT 2",
        )
        .bind(self.lab_id)
        .bind(email)
        .fetch_all(self.postgres.pool())
        .await
        .map_err(database)?;
        if rows.len() != 1 {
            return Ok(None);
        }
        let row = &rows[0];
        Ok(Some(PasswordRecord {
            user_id: row.try_get("user_id").map_err(database)?,
            password_hash: row.try_get("password_hash").map_err(database)?,
            credential_policy_revision: row
                .try_get("credential_policy_revision")
                .map_err(database)?,
        }))
    }

    async fn verify_login_password(
        &self,
        record: Option<PasswordRecord>,
        supplied_password: &str,
    ) -> Result<VerifiedLogin, AuthError> {
        let (user_id, policy_revision, password_hash, record_exists) = match record {
            Some(record) => (
                Some(record.user_id),
                record.credential_policy_revision,
                record.password_hash,
                true,
            ),
            None => (None, 0, self.dummy_password_hash.to_string(), false),
        };
        let supplied_password = supplied_password.to_owned();
        let valid = tokio::task::spawn_blocking(move || {
            verify_password(&password_hash, supplied_password.as_bytes())
        })
        .await
        .map_err(|_| AuthError::Unavailable)??;
        if record_exists && valid {
            Ok(VerifiedLogin {
                user_id: user_id.ok_or(AuthError::InvalidCredentials)?,
                policy_upgrade_required: policy_revision < self.credential_policy.revision(),
            })
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }

    fn login_identity_digest(&self, email: &str) -> Option<[u8; 32]> {
        let key = self.login_identity_hmac_key.as_deref()?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, key);
        let mut context = Vec::with_capacity(64 + email.len());
        context.extend_from_slice(b"muriarc-login-backoff-v1\0");
        context.extend_from_slice(self.lab_id.as_bytes());
        context.extend_from_slice(normalize_email(email).as_bytes());
        hmac::sign(&key, &context).as_ref().try_into().ok()
    }

    async fn login_is_blocked(
        &self,
        identity_digest: Option<&[u8; 32]>,
        now: DateTime<Utc>,
    ) -> Result<bool, AuthError> {
        let Some(identity_digest) = identity_digest else {
            return Ok(false);
        };
        let blocked_until: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT blocked_until FROM auth_login_backoff WHERE identity_digest = $1",
        )
        .bind(identity_digest.as_slice())
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?
        .flatten();
        Ok(blocked_until.is_some_and(|blocked_until| blocked_until > now))
    }

    async fn record_login_failure(
        &self,
        identity_digest: Option<&[u8; 32]>,
        now: DateTime<Utc>,
    ) -> Result<(), AuthError> {
        let Some(identity_digest) = identity_digest else {
            return Ok(());
        };
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(login_backoff_lock_id(identity_digest))
            .execute(&mut *tx)
            .await
            .map_err(database)?;
        let existing: Option<(i32, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT failure_count, first_failed_at, last_failed_at FROM auth_login_backoff WHERE identity_digest = $1 FOR UPDATE",
        )
        .bind(identity_digest.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?;
        let (failure_count, first_failed_at) = match existing {
            Some((count, first, last))
                if now - last <= Duration::hours(LOGIN_FAILURE_WINDOW_HOURS) =>
            {
                (count.saturating_add(1), first)
            }
            _ => (1, now),
        };
        let delay = login_backoff_delay(failure_count);
        let blocked_until = (delay > Duration::zero()).then_some(now + delay);
        sqlx::query(
            "INSERT INTO auth_login_backoff (identity_digest, failure_count, blocked_until, first_failed_at, last_failed_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (identity_digest) DO UPDATE SET failure_count = EXCLUDED.failure_count, blocked_until = EXCLUDED.blocked_until, first_failed_at = EXCLUDED.first_failed_at, last_failed_at = EXCLUDED.last_failed_at",
        )
        .bind(identity_digest.as_slice())
        .bind(failure_count)
        .bind(blocked_until)
        .bind(first_failed_at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        sqlx::query(
            "DELETE FROM auth_login_backoff WHERE identity_digest IN (SELECT identity_digest FROM auth_login_backoff WHERE last_failed_at < $1 ORDER BY last_failed_at LIMIT $2)",
        )
        .bind(now - Duration::days(LOGIN_BACKOFF_RETENTION_DAYS))
        .bind(LOGIN_BACKOFF_CLEANUP_BATCH)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        tx.commit().await.map_err(database)?;
        Ok(())
    }
}

#[async_trait]
impl Authenticator for PostgresAuthBackend {
    async fn authenticate(&self, bearer_token: &str) -> Result<AuthPrincipal, AuthError> {
        let digest = token_hash(bearer_token);
        let row = sqlx::query(
            "UPDATE external_tokens SET last_used_at = now() WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now() RETURNING id, user_id, scopes",
        )
        .bind(digest.as_slice())
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?
        .ok_or(AuthError::InvalidCredentials)?;
        let user_id: Uuid = row.try_get("user_id").map_err(database)?;
        let encoded_scopes: Vec<String> = row.try_get("scopes").map_err(database)?;
        let scopes = decode_scopes(encoded_scopes)?;
        self.principal_for(user_id, Some(scopes), WriteSource::Api)
            .await
    }
}

#[async_trait]
impl SessionBackend for PostgresAuthBackend {
    async fn login(
        &self,
        email: &str,
        password: &str,
        session: &NewSession,
    ) -> Result<AuthenticatedSession, AuthError> {
        if password.len() > PASSWORD_MAX_BYTES {
            return Err(AuthError::InvalidCredentials);
        }
        let now = Utc::now();
        let identity_digest = self.login_identity_digest(email);
        let blocked = self.login_is_blocked(identity_digest.as_ref(), now).await?;
        let record = self.password_record(email).await?;
        let verified = self.verify_login_password(record, password).await;
        let verified = match verified {
            Ok(verified) if !blocked => verified,
            Ok(_) | Err(AuthError::InvalidCredentials) => {
                if !blocked {
                    self.record_login_failure(identity_digest.as_ref(), now)
                        .await?;
                }
                return Err(AuthError::InvalidCredentials);
            }
            Err(error) => return Err(error),
        };
        let mut principal = self
            .principal_for(verified.user_id, None, WriteSource::Web)
            .await?;

        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        if verified.policy_upgrade_required {
            sqlx::query(
                "UPDATE user_credentials SET must_change_password = TRUE, revision = revision + 1 WHERE user_id = $1 AND credential_policy_revision < $2",
            )
            .bind(verified.user_id)
            .bind(self.credential_policy.revision())
            .execute(&mut *tx)
            .await
            .map_err(database)?;
            principal = principal
                .with_credential_state(true, verified.user_id == self.environment_root_user_id);
            write_auth_audit(
                &mut tx,
                &principal,
                "user_credential",
                verified.user_id,
                "policy_upgrade_required",
                json!({
                    "credential_policy_revision_required": self.credential_policy.revision(),
                    "must_change_password": true
                }),
                session.created_at,
            )
            .await?;
        }
        if let Some(identity_digest) = identity_digest {
            sqlx::query("DELETE FROM auth_login_backoff WHERE identity_digest = $1")
                .bind(identity_digest.as_slice())
                .execute(&mut *tx)
                .await
                .map_err(database)?;
        }
        sqlx::query(
            "INSERT INTO auth_sessions (id, user_id, token_hash, csrf_hash, created_at, last_seen_at, expires_at, revoked_at) VALUES ($1, $2, $3, $4, $5, $5, $6, NULL)",
        )
        .bind(session.id)
        .bind(verified.user_id)
        .bind(session.token_hash.as_slice())
        .bind(session.csrf_hash.as_slice())
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        write_auth_audit(
            &mut tx,
            &principal,
            "auth_session",
            session.id,
            "create",
            json!({"expires_at": session.expires_at}),
            session.created_at,
        )
        .await?;
        tx.commit().await.map_err(database)?;

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
        let digest = token_hash(session_token);
        let row = sqlx::query(
            "UPDATE auth_sessions SET last_seen_at = now() WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now() RETURNING id, user_id, csrf_hash, expires_at",
        )
        .bind(digest.as_slice())
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?
        .ok_or(AuthError::InvalidCredentials)?;
        let session_id: Uuid = row.try_get("id").map_err(database)?;
        let user_id: Uuid = row.try_get("user_id").map_err(database)?;
        let csrf_hash = hash_from_row(&row, "csrf_hash")?;
        let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(database)?;
        let principal = self.principal_for(user_id, None, WriteSource::Web).await?;
        Ok(AuthenticatedSession {
            principal,
            session_id,
            csrf_hash,
            expires_at,
        })
    }

    async fn verify_current_password(
        &self,
        user_id: Uuid,
        password: &str,
    ) -> Result<(), AuthError> {
        if password.len() > PASSWORD_MAX_BYTES {
            return Err(AuthError::InvalidCredentials);
        }
        let password_hash: Option<String> = sqlx::query_scalar(
            "SELECT c.password_hash FROM user_credentials c JOIN users u ON u.id = c.user_id WHERE u.id = $1 AND u.lab_id = $2 AND u.status = 'active' AND u.deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(self.lab_id)
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?;
        let record_exists = password_hash.is_some();
        let verification_hash =
            password_hash.unwrap_or_else(|| self.dummy_password_hash.to_string());
        let supplied = Zeroizing::new(password.to_owned());
        let valid = tokio::task::spawn_blocking(move || {
            verify_password(&verification_hash, supplied.as_bytes())
        })
        .await
        .map_err(|_| AuthError::Unavailable)??;
        if record_exists && valid {
            Ok(())
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }

    async fn change_password(
        &self,
        principal: &AuthPrincipal,
        session_id: Uuid,
        current_password: &str,
        new_password: &str,
        request_id: &str,
    ) -> Result<AuthPrincipal, AuthError> {
        if principal.lab_id != self.lab_id {
            return Err(AuthError::InvalidCredentials);
        }
        if principal.is_environment_root() {
            return Err(AuthError::EnvironmentRootManaged);
        }

        let row = sqlx::query(
            "SELECT c.password_hash, c.revision FROM user_credentials c JOIN users u ON u.id = c.user_id WHERE c.user_id = $1 AND u.lab_id = $2 AND u.status = 'active' AND u.deleted_at IS NULL",
        )
        .bind(principal.user_id)
        .bind(self.lab_id)
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?
        .ok_or(AuthError::InvalidCredentials)?;
        let existing_hash: String = row.try_get("password_hash").map_err(database)?;
        let expected_revision: i64 = row.try_get("revision").map_err(database)?;
        let verification_hash = existing_hash.clone();
        let credential_policy = self.credential_policy;
        let current = Zeroizing::new(current_password.to_owned());
        let next = Zeroizing::new(new_password.to_owned());
        let next_hash = tokio::task::spawn_blocking(move || {
            if !verify_password(&verification_hash, current.as_bytes())? {
                return Err(AuthError::InvalidCredentials);
            }
            if verify_password(&verification_hash, next.as_bytes())? {
                return Err(AuthError::PasswordReuse);
            }
            hash_password_with_policy(next.as_str(), credential_policy)
        })
        .await
        .map_err(|_| AuthError::Unavailable)??;

        let now = Utc::now();
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        let changed = sqlx::query(
            "UPDATE user_credentials SET password_hash = $2, password_changed_at = $3, must_change_password = FALSE, credential_policy_revision = $4, revision = revision + 1 WHERE user_id = $1 AND revision = $5 AND password_hash = $6",
        )
        .bind(principal.user_id)
        .bind(next_hash)
        .bind(now)
        .bind(self.credential_policy.revision())
        .bind(expected_revision)
        .bind(existing_hash)
        .execute(&mut *tx)
        .await
        .map_err(database)?
        .rows_affected();
        if changed != 1 {
            return Err(AuthError::Unavailable);
        }
        let revoked_session_ids: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE auth_sessions SET revoked_at = $3 WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL RETURNING id",
        )
        .bind(principal.user_id)
        .bind(session_id)
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(database)?;
        for revoked_session_id in &revoked_session_ids {
            write_security_audit(
                &mut tx,
                principal,
                SecurityAudit {
                    entity_type: "auth_session",
                    entity_id: *revoked_session_id,
                    action: "revoke",
                    operation_code: "auth.session.revoked.password_change",
                    request_id,
                    after: json!({
                        "revoked_at": now,
                        "reason": "password_changed"
                    }),
                    occurred_at: now,
                },
            )
            .await?;
        }
        write_security_audit(
            &mut tx,
            principal,
            SecurityAudit {
                entity_type: "user_credential",
                entity_id: principal.user_id,
                action: "update",
                operation_code: "auth.password.changed",
                request_id,
                after: json!({
                    "password_changed": true,
                    "must_change_password": false,
                    "other_sessions_revoked": revoked_session_ids.len(),
                    "credential_revision": expected_revision + 1,
                    "credential_policy_revision": self.credential_policy.revision()
                }),
                occurred_at: now,
            },
        )
        .await?;
        tx.commit().await.map_err(database)?;
        self.principal_for(principal.user_id, None, WriteSource::Web)
            .await
    }

    async fn update_own_display_name(
        &self,
        principal: &AuthPrincipal,
        display_name: &str,
        request_id: &str,
    ) -> Result<AuthPrincipal, AuthError> {
        if principal.lab_id != self.lab_id {
            return Err(AuthError::InvalidCredentials);
        }
        if principal.is_environment_root() {
            return Err(AuthError::EnvironmentRootManaged);
        }
        let display_name = display_name.trim();
        if display_name.is_empty()
            || display_name.chars().count() > 200
            || display_name.chars().any(char::is_control)
        {
            return Err(AuthError::InvalidProfile);
        }

        let now = Utc::now();
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        let row = sqlx::query(
            "SELECT display_name, revision FROM users WHERE id = $1 AND lab_id = $2 AND status = 'active' AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(principal.user_id)
        .bind(self.lab_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?
        .ok_or(AuthError::InvalidCredentials)?;
        let previous_name: String = row.try_get("display_name").map_err(database)?;
        let revision: i64 = row.try_get("revision").map_err(database)?;
        if previous_name != display_name {
            sqlx::query(
                "UPDATE users SET display_name = $2, updated_at = $3, revision = $4 WHERE id = $1 AND revision = $5",
            )
            .bind(principal.user_id)
            .bind(display_name)
            .bind(now)
            .bind(revision + 1)
            .bind(revision)
            .execute(&mut *tx)
            .await
            .map_err(database)?;
            write_security_audit(
                &mut tx,
                principal,
                SecurityAudit {
                    entity_type: "user",
                    entity_id: principal.user_id,
                    action: "update",
                    operation_code: "auth.profile.updated",
                    request_id,
                    after: json!({
                        "display_name_before": previous_name,
                        "display_name_after": display_name,
                        "revision": revision + 1
                    }),
                    occurred_at: now,
                },
            )
            .await?;
        }
        tx.commit().await.map_err(database)?;
        self.principal_for(principal.user_id, None, WriteSource::Web)
            .await
    }

    async fn revoke_session(&self, session_id: Uuid, user_id: Uuid) -> Result<(), AuthError> {
        let principal = self.principal_for(user_id, None, WriteSource::Web).await?;
        let now = Utc::now();
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        let changed = sqlx::query(
            "UPDATE auth_sessions SET revoked_at = $3 WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?
        .rows_affected()
            == 1;
        if changed {
            write_auth_audit(
                &mut tx,
                &principal,
                "auth_session",
                session_id,
                "revoke",
                json!({"revoked_at": now}),
                now,
            )
            .await?;
        }
        tx.commit().await.map_err(database)?;
        Ok(())
    }

    async fn create_external_token(
        &self,
        user_id: Uuid,
        token: &NewExternalToken,
    ) -> Result<ExternalTokenSummary, AuthError> {
        let principal = self.principal_for(user_id, None, WriteSource::Web).await?;
        let encoded_scopes = encode_scopes(&token.scopes);
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        sqlx::query(
            "INSERT INTO external_tokens (id, user_id, name, token_hash, scopes, created_at, expires_at, last_used_at, revoked_at) VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, NULL)",
        )
        .bind(token.id)
        .bind(user_id)
        .bind(&token.name)
        .bind(token.token_hash.as_slice())
        .bind(&encoded_scopes)
        .bind(token.created_at)
        .bind(token.expires_at)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        write_auth_audit(
            &mut tx,
            &principal,
            "external_token",
            token.id,
            "create",
            json!({
                "name": token.name,
                "scopes": encoded_scopes,
                "expires_at": token.expires_at,
            }),
            token.created_at,
        )
        .await?;
        tx.commit().await.map_err(database)?;
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
        self.principal_for(user_id, None, WriteSource::Web).await?;
        let rows = sqlx::query(
            "SELECT id, name, scopes, created_at, expires_at, last_used_at, revoked_at FROM external_tokens WHERE user_id = $1 ORDER BY created_at DESC, id",
        )
        .bind(user_id)
        .fetch_all(self.postgres.pool())
        .await
        .map_err(database)?;
        rows.iter().map(external_token_from_row).collect()
    }

    async fn revoke_external_token(&self, user_id: Uuid, token_id: Uuid) -> Result<(), AuthError> {
        let principal = self.principal_for(user_id, None, WriteSource::Web).await?;
        let now = Utc::now();
        let mut tx = self.postgres.pool().begin().await.map_err(database)?;
        let changed = sqlx::query(
            "UPDATE external_tokens SET revoked_at = $3 WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(token_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?
        .rows_affected()
            == 1;
        if changed {
            write_auth_audit(
                &mut tx,
                &principal,
                "external_token",
                token_id,
                "revoke",
                json!({"revoked_at": now}),
                now,
            )
            .await?;
        }
        tx.commit().await.map_err(database)?;
        Ok(())
    }
}

struct SecurityAudit<'a> {
    entity_type: &'static str,
    entity_id: Uuid,
    action: &'static str,
    operation_code: &'static str,
    request_id: &'a str,
    after: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

async fn write_security_audit(
    tx: &mut Transaction<'_, Postgres>,
    principal: &AuthPrincipal,
    audit: SecurityAudit<'_>,
) -> Result<(), AuthError> {
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at, operation_code, operation_version, operation_params_json, entity_name_snapshot) VALUES ($1, $2, NULL, $3, $4, $5, 'human', $6, $7, 'web', $8, 'account credential lifecycle', NULL, $9, $10, $11, 1, $12, $13)",
    )
    .bind(Uuid::new_v4())
    .bind(principal.lab_id)
    .bind(audit.entity_type)
    .bind(audit.entity_id)
    .bind(audit.action)
    .bind(principal.user_id)
    .bind(&principal.display_name)
    .bind(audit.request_id)
    .bind(&audit.after)
    .bind(audit.occurred_at)
    .bind(audit.operation_code)
    .bind(json!({"credentials": "redacted"}))
    .bind(&principal.display_name)
    .execute(&mut **tx)
    .await
    .map_err(database)?;
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, AuthError> {
    hash_password_with_policy(password, CredentialPolicy::private())
}

pub(crate) fn hash_password_with_policy(
    password: &str,
    credential_policy: CredentialPolicy,
) -> Result<String, AuthError> {
    validate_new_password(password, credential_policy)?;
    let salt = SaltString::generate(&mut OsRng);
    password_hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Unavailable)
}

fn password_hasher() -> Argon2<'static> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .expect("fixed Argon2id parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub(crate) fn verify_password(encoded_hash: &str, supplied: &[u8]) -> Result<bool, AuthError> {
    let hash = PasswordHash::new(encoded_hash).map_err(|_| AuthError::Unavailable)?;
    if hash.algorithm.as_str() != "argon2id" {
        return Err(AuthError::Unavailable);
    }
    Ok(password_hasher().verify_password(supplied, &hash).is_ok())
}

fn validate_new_password(
    password: &str,
    credential_policy: CredentialPolicy,
) -> Result<(), AuthError> {
    if !credential_policy.accepts(password) {
        Err(AuthError::PasswordPolicy {
            revision: credential_policy.revision(),
            min_chars: credential_policy.min_chars(),
        })
    } else {
        Ok(())
    }
}

fn login_backoff_delay(failure_count: i32) -> Duration {
    if failure_count < 5 {
        return Duration::zero();
    }
    let exponent = u32::try_from(failure_count.saturating_sub(5)).unwrap_or(u32::MAX);
    let multiplier = 1_i64.checked_shl(exponent.min(5)).unwrap_or(32);
    Duration::seconds((30 * multiplier).min(15 * 60))
}

fn login_backoff_lock_id(identity_digest: &[u8; 32]) -> i64 {
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&identity_digest[..8]);
    i64::from_be_bytes(prefix) ^ LOGIN_BACKOFF_LOCK_NAMESPACE
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn hash_from_row(row: &sqlx::postgres::PgRow, column: &str) -> Result<[u8; 32], AuthError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(database)?;
    bytes.try_into().map_err(|_| AuthError::Unavailable)
}

fn encode_scopes(scopes: &BTreeSet<AiScope>) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| scope_name(*scope).to_owned())
        .collect()
}

fn decode_scopes(encoded: Vec<String>) -> Result<BTreeSet<AiScope>, AuthError> {
    encoded
        .into_iter()
        .map(|scope| parse_scope(&scope))
        .collect()
}

fn scope_name(scope: AiScope) -> &'static str {
    match scope {
        AiScope::Read => "read",
        AiScope::WriteDraft => "write-draft",
        AiScope::Import => "import",
        AiScope::Export => "export",
        AiScope::TemplateDraft => "template-draft",
    }
}

fn parse_scope(scope: &str) -> Result<AiScope, AuthError> {
    match scope {
        "read" => Ok(AiScope::Read),
        "write-draft" => Ok(AiScope::WriteDraft),
        "import" => Ok(AiScope::Import),
        "export" => Ok(AiScope::Export),
        "template-draft" => Ok(AiScope::TemplateDraft),
        _ => Err(AuthError::Unavailable),
    }
}

fn external_token_from_row(row: &sqlx::postgres::PgRow) -> Result<ExternalTokenSummary, AuthError> {
    Ok(ExternalTokenSummary {
        id: row.try_get("id").map_err(database)?,
        name: row.try_get("name").map_err(database)?,
        scopes: decode_scopes(row.try_get("scopes").map_err(database)?)?,
        created_at: row.try_get("created_at").map_err(database)?,
        expires_at: row.try_get("expires_at").map_err(database)?,
        last_used_at: row.try_get("last_used_at").map_err(database)?,
        revoked_at: row.try_get("revoked_at").map_err(database)?,
    })
}

async fn write_auth_audit(
    tx: &mut Transaction<'_, Postgres>,
    principal: &AuthPrincipal,
    entity_type: &'static str,
    entity_id: Uuid,
    action: &'static str,
    after: serde_json::Value,
    occurred_at: DateTime<Utc>,
) -> Result<(), AuthError> {
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, $3, $4, $5, 'human', $6, $7, 'web', NULL, 'authentication lifecycle', NULL, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(principal.lab_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(action)
    .bind(principal.user_id)
    .bind(&principal.display_name)
    .bind(after)
    .bind(occurred_at)
    .execute(&mut **tx)
    .await
    .map_err(database)?;
    Ok(())
}

fn map_store(error: StoreError) -> AuthError {
    match error {
        StoreError::NotFound { .. } => AuthError::InvalidCredentials,
        other => {
            tracing::error!(error = %other, "identity store operation failed");
            AuthError::Unavailable
        }
    }
}

fn database(error: sqlx::Error) -> AuthError {
    tracing::error!(error = %error, "authentication database operation failed");
    AuthError::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_core::{
        AuditContext, LabRole, Membership, Permission, Project, ProjectRole, WriteSource,
    };

    #[test]
    fn argon2id_hashes_are_salted_and_verifiable() {
        let first = hash_password("a sufficiently long password").unwrap();
        let second = hash_password("a sufficiently long password").unwrap();
        assert!(first.starts_with("$argon2id$"));
        assert_ne!(first, second);
        assert!(verify_password(&first, b"a sufficiently long password").unwrap());
        assert!(!verify_password(&first, b"the wrong password").unwrap());
    }

    #[test]
    fn password_policy_rejects_short_and_control_characters() {
        assert!(hash_password("short").is_err());
        assert!(hash_password("valid length but\ncontrol").is_err());
        assert!(
            hash_password_with_policy("only-fourteen!", CredentialPolicy::cloudflare_public())
                .is_err()
        );
        assert!(
            hash_password_with_policy("fifteen-chars!!", CredentialPolicy::cloudflare_public())
                .is_ok()
        );
    }

    #[test]
    fn persistent_login_backoff_is_bounded_and_not_applied_before_threshold() {
        assert_eq!(login_backoff_delay(1), Duration::zero());
        assert_eq!(login_backoff_delay(4), Duration::zero());
        assert_eq!(login_backoff_delay(5), Duration::seconds(30));
        assert_eq!(login_backoff_delay(6), Duration::seconds(60));
        assert_eq!(login_backoff_delay(10), Duration::seconds(15 * 60));
        assert_eq!(login_backoff_delay(i32::MAX), Duration::seconds(15 * 60));
    }

    #[tokio::test]
    async fn postgres_sessions_and_external_tokens_are_hashed_scoped_and_revocable() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
        );
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        store.migrate().await.unwrap();
        let password = "integration session password";
        let password_hash = hash_password(password).unwrap();
        let config = crate::BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Authentication integration lab",
            Uuid::new_v4(),
            format!("auth-{}@example.org", Uuid::new_v4()),
            "Authentication integration user",
        )
        .unwrap()
        .with_password_hash(password_hash.clone())
        .unwrap();
        crate::seed_postgres_bootstrap(store.as_ref(), &config)
            .await
            .unwrap();

        let now = Utc::now();
        let audit = AuditContext::system(WriteSource::Migration);
        let project = Project::new(config.lab_id, "Authentication project", now).unwrap();
        store.create_project(&project, &audit).await.unwrap();
        store
            .create_membership(
                &Membership::project(
                    config.lab_id,
                    project.id,
                    config.user_id,
                    ProjectRole::Viewer,
                    now,
                ),
                &audit,
            )
            .await
            .unwrap();

        let backend = PostgresAuthBackend::new(
            store.as_ref().clone(),
            store.clone(),
            config.lab_id,
            config.user_id,
        )
        .unwrap();
        let raw_session = "mas_integration_session_secret_000000000000000000000";
        let raw_csrf = "mac_integration_csrf_secret_00000000000000000000000";
        let session = NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash(raw_session),
            csrf_hash: token_hash(raw_csrf),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        let logged_in = backend
            .login(&config.user_email, password, &session)
            .await
            .unwrap();
        assert!(
            logged_in
                .principal
                .lab_roles()
                .any(|role| role == LabRole::LabAdmin)
        );
        assert!(logged_in.principal.project_ids().any(|id| id == project.id));
        assert_eq!(
            backend
                .authenticate_session(raw_session)
                .await
                .unwrap()
                .session_id,
            session.id
        );
        backend
            .verify_current_password(config.user_id, password)
            .await
            .unwrap();
        assert_eq!(
            backend
                .verify_current_password(config.user_id, "wrong current password")
                .await
                .unwrap_err(),
            AuthError::InvalidCredentials
        );
        assert_eq!(
            backend
                .verify_current_password(Uuid::new_v4(), password)
                .await
                .unwrap_err(),
            AuthError::InvalidCredentials
        );

        let stored_password: String =
            sqlx::query_scalar("SELECT password_hash FROM user_credentials WHERE user_id = $1")
                .bind(config.user_id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        let stored_session: Vec<u8> =
            sqlx::query_scalar("SELECT token_hash FROM auth_sessions WHERE id = $1")
                .bind(session.id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(stored_password, password_hash);
        assert_ne!(stored_password, password);
        assert_eq!(stored_session, token_hash(raw_session));
        assert_ne!(stored_session, raw_session.as_bytes());

        let raw_external = "mat_integration_external_secret_0000000000000000000";
        let external = NewExternalToken {
            id: Uuid::new_v4(),
            name: "Integration MCP".to_owned(),
            token_hash: token_hash(raw_external),
            scopes: BTreeSet::from([AiScope::Read]),
            created_at: now,
            expires_at: now + chrono::Duration::days(1),
        };
        backend
            .create_external_token(config.user_id, &external)
            .await
            .unwrap();
        let external_principal = backend.authenticate(raw_external).await.unwrap();
        assert!(external_principal.is_external_ai());
        assert!(external_principal.can(Permission::ReadAnimal, Some(project.id)));
        assert!(!external_principal.can(Permission::ManageLab, None));
        backend
            .revoke_external_token(config.user_id, external.id)
            .await
            .unwrap();
        assert_eq!(
            backend.authenticate(raw_external).await.unwrap_err(),
            AuthError::InvalidCredentials
        );

        sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
            .bind(config.user_id)
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            backend.authenticate_session(raw_session).await.unwrap_err(),
            AuthError::InvalidCredentials
        );
        assert_eq!(
            backend
                .verify_current_password(config.user_id, password)
                .await
                .unwrap_err(),
            AuthError::InvalidCredentials
        );
        sqlx::query("UPDATE users SET status = 'active', deleted_at = now() WHERE id = $1")
            .bind(config.user_id)
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            backend.authenticate_session(raw_session).await.unwrap_err(),
            AuthError::InvalidCredentials
        );
    }

    #[tokio::test]
    async fn public_policy_marks_legacy_credentials_and_persists_hmac_backoff() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(database_url.contains("muriarc_test"));
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        store.migrate().await.unwrap();
        let legacy_password = "legacy public password";
        let config = crate::BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Public credential policy lab",
            Uuid::new_v4(),
            format!("public-policy-{}@example.org", Uuid::new_v4()),
            "Public policy user",
        )
        .unwrap()
        .with_password_hash(hash_password(legacy_password).unwrap())
        .unwrap();
        crate::seed_postgres_bootstrap(store.as_ref(), &config)
            .await
            .unwrap();

        let rate_key = [73_u8; 32];
        let backend = PostgresAuthBackend::new_with_policy(
            store.as_ref().clone(),
            store.clone(),
            config.lab_id,
            Uuid::new_v4(),
            CredentialPolicy::cloudflare_public(),
            Some(rate_key),
        )
        .unwrap();
        let now = Utc::now();
        let session = NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash("mas_public_policy_session_0000000000000000000000"),
            csrf_hash: token_hash("mac_public_policy_session_0000000000000000000000"),
            created_at: now,
            expires_at: now + Duration::hours(1),
        };
        let logged_in = backend
            .login(&config.user_email, legacy_password, &session)
            .await
            .unwrap();
        assert!(logged_in.principal.must_change_password());
        let forced: (bool, i32) = sqlx::query_as(
            "SELECT must_change_password, credential_policy_revision FROM user_credentials WHERE user_id = $1",
        )
        .bind(config.user_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(forced, (true, 1));

        let replacement = "replacement public password";
        let changed = backend
            .change_password(
                &logged_in.principal,
                session.id,
                legacy_password,
                replacement,
                "public-policy-test",
            )
            .await
            .unwrap();
        assert!(!changed.must_change_password());
        let upgraded: (bool, i32) = sqlx::query_as(
            "SELECT must_change_password, credential_policy_revision FROM user_credentials WHERE user_id = $1",
        )
        .bind(config.user_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(upgraded, (false, 2));

        let probed_email = format!("unknown-{}@example.org", Uuid::new_v4());
        for index in 0..5 {
            let rejected = backend
                .login(
                    &probed_email,
                    "not the password",
                    &NewSession {
                        id: Uuid::new_v4(),
                        token_hash: token_hash(&format!(
                            "mas_rejected_{index}_000000000000000000000000"
                        )),
                        csrf_hash: token_hash(&format!(
                            "mac_rejected_{index}_000000000000000000000000"
                        )),
                        created_at: now,
                        expires_at: now + Duration::hours(1),
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(rejected, AuthError::InvalidCredentials);
        }
        let identity_digest = backend.login_identity_digest(&probed_email).unwrap();
        assert_eq!(
            identity_digest,
            backend
                .login_identity_digest(&format!("  {}  ", probed_email.to_ascii_uppercase()))
                .unwrap()
        );
        let backoff: (i32, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT failure_count, blocked_until FROM auth_login_backoff WHERE identity_digest = $1",
        )
        .bind(identity_digest.as_slice())
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(backoff.0, 5);
        assert!(backoff.1.is_some_and(|until| until > now));
        assert!(!format!("{backend:?}").contains(&probed_email));
    }

    #[tokio::test]
    async fn postgres_password_change_clears_forced_state_and_revokes_other_sessions() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(database_url.contains("muriarc_test"));
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        store.migrate().await.unwrap();

        let original_password = format!("temporary-password-{}", Uuid::new_v4());
        let config = crate::BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Password lifecycle integration lab",
            Uuid::new_v4(),
            format!("password-user-{}@example.org", Uuid::new_v4()),
            "Password lifecycle user",
        )
        .unwrap()
        .with_password_hash(hash_password(&original_password).unwrap())
        .unwrap();
        crate::seed_postgres_bootstrap(store.as_ref(), &config)
            .await
            .unwrap();
        let backend = PostgresAuthBackend::new(
            store.as_ref().clone(),
            store.clone(),
            config.lab_id,
            Uuid::new_v4(),
        )
        .unwrap();
        let now = Utc::now();
        let make_session = || NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash(&format!("mas_{}", Uuid::new_v4().simple())),
            csrf_hash: token_hash(&format!("mac_{}", Uuid::new_v4().simple())),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        let first_session = make_session();
        let first_raw = format!("mas_{}", Uuid::new_v4().simple());
        let first_session = NewSession {
            token_hash: token_hash(&first_raw),
            ..first_session
        };
        let first = backend
            .login(&config.user_email, &original_password, &first_session)
            .await
            .unwrap();
        let second_raw = format!("mas_{}", Uuid::new_v4().simple());
        let second_session = NewSession {
            token_hash: token_hash(&second_raw),
            ..make_session()
        };
        backend
            .login(&config.user_email, &original_password, &second_session)
            .await
            .unwrap();

        let external_raw = format!("mat_{}", Uuid::new_v4().simple());
        let external = NewExternalToken {
            id: Uuid::new_v4(),
            name: "Existing integration token".to_owned(),
            token_hash: token_hash(&external_raw),
            scopes: BTreeSet::from([AiScope::Read]),
            created_at: now,
            expires_at: now + chrono::Duration::days(1),
        };
        backend
            .create_external_token(config.user_id, &external)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE user_credentials SET must_change_password = TRUE, revision = revision + 1 WHERE user_id = $1",
        )
        .bind(config.user_id)
        .execute(store.pool())
        .await
        .unwrap();

        assert_eq!(
            backend.authenticate(&external_raw).await.unwrap_err(),
            AuthError::PasswordChangeRequired
        );
        let forced = backend.authenticate_session(&first_raw).await.unwrap();
        assert!(forced.principal.must_change_password());

        let replacement_password = format!("permanent-password-{}", Uuid::new_v4());
        let request_id = format!("password-change-{}", Uuid::new_v4());
        let changed = backend
            .change_password(
                &forced.principal,
                first.session_id,
                &original_password,
                &replacement_password,
                &request_id,
            )
            .await
            .unwrap();
        assert!(!changed.must_change_password());
        assert_eq!(
            backend.authenticate_session(&second_raw).await.unwrap_err(),
            AuthError::InvalidCredentials
        );
        assert!(
            !backend
                .authenticate_session(&first_raw)
                .await
                .unwrap()
                .principal
                .must_change_password()
        );
        assert!(backend.authenticate(&external_raw).await.is_ok());

        let failed_login = make_session();
        assert_eq!(
            backend
                .login(&config.user_email, &original_password, &failed_login)
                .await
                .unwrap_err(),
            AuthError::InvalidCredentials
        );
        backend
            .login(&config.user_email, &replacement_password, &make_session())
            .await
            .unwrap();

        let credential: (String, bool, i64) = sqlx::query_as(
            "SELECT password_hash, must_change_password, revision FROM user_credentials WHERE user_id = $1",
        )
        .bind(config.user_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert!(verify_password(&credential.0, replacement_password.as_bytes()).unwrap());
        assert!(!credential.1);
        assert_eq!(credential.2, 3);

        let audit_payloads: Vec<String> = sqlx::query_scalar(
            "SELECT concat_ws(' ', coalesce(before_json::text, ''), coalesce(after_json::text, ''), operation_params_json::text) FROM audit_entries WHERE request_id = $1",
        )
        .bind(&request_id)
        .fetch_all(store.pool())
        .await
        .unwrap();
        let audit_payloads = audit_payloads.join("\n");
        assert!(!audit_payloads.contains(&original_password));
        assert!(!audit_payloads.contains(&replacement_password));
        assert!(!audit_payloads.contains("$argon2id$"));
        assert!(audit_payloads.contains("credential_revision"));
    }
}
