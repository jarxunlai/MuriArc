use std::fmt;

use chrono::{DateTime, Utc};
use muriarc_store_postgres::PostgresStore;
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{hash_password, persistent_auth::verify_password};

const ROOT_LOCK_ID: i64 = 5_568_604_466_432_177_475;
const ROOT_REQUEST_ID: &str = "environment-root-sync";
const PASSWORD_MIN_CHARS: usize = 8;
const PASSWORD_MAX_BYTES: usize = 1024;

pub struct EnvironmentRootConfig {
    pub lab_id: Uuid,
    pub lab_name: String,
    pub user_id: Uuid,
    pub user_email: String,
    pub user_display_name: String,
    password: Zeroizing<String>,
}

impl fmt::Debug for EnvironmentRootConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentRootConfig")
            .field("lab_id", &self.lab_id)
            .field("lab_name", &self.lab_name)
            .field("user_id", &self.user_id)
            .field("user_email", &self.user_email)
            .field("user_display_name", &self.user_display_name)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl EnvironmentRootConfig {
    pub fn new(
        lab_id: Uuid,
        lab_name: impl Into<String>,
        user_id: Uuid,
        user_email: impl Into<String>,
        user_display_name: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, EnvironmentRootError> {
        let lab_name = clean_text("lab name", lab_name.into(), 200)?;
        let user_email = clean_text("root email", user_email.into(), 320)?.to_ascii_lowercase();
        let user_display_name = clean_text("root display name", user_display_name.into(), 200)?;
        let password = password.into();
        if !user_email.contains('@') {
            return Err(EnvironmentRootError::InvalidConfig(
                "root email must contain @".to_owned(),
            ));
        }
        if lab_id == user_id {
            return Err(EnvironmentRootError::InvalidConfig(
                "root user UUID must differ from the lab UUID".to_owned(),
            ));
        }
        if password.chars().count() < PASSWORD_MIN_CHARS
            || password.len() > PASSWORD_MAX_BYTES
            || password.chars().any(char::is_control)
        {
            return Err(EnvironmentRootError::InvalidConfig(
                "root password must contain 8-1024 non-control characters".to_owned(),
            ));
        }
        Ok(Self {
            lab_id,
            lab_name,
            user_id,
            user_email,
            user_display_name,
            password: Zeroizing::new(password),
        })
    }

    fn password(&self) -> &str {
        self.password.as_str()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnvironmentRootOutcome {
    pub lab_created: bool,
    pub lab_updated: bool,
    pub user_created: bool,
    pub user_updated: bool,
    pub membership_created: bool,
    pub membership_updated: bool,
    pub credential_created: bool,
    pub credential_updated: bool,
    pub sessions_revoked: u64,
}

impl EnvironmentRootOutcome {
    pub const fn changed(self) -> bool {
        self.lab_created
            || self.lab_updated
            || self.user_created
            || self.user_updated
            || self.membership_created
            || self.membership_updated
            || self.credential_created
            || self.credential_updated
            || self.sessions_revoked > 0
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentRootError {
    #[error("invalid environment root configuration: {0}")]
    InvalidConfig(String),
    #[error("environment root identity conflicts with existing data: {0}")]
    IdentityConflict(String),
    #[error("environment root database transaction failed: {0}")]
    Database(String),
}

pub async fn sync_postgres_environment_root(
    store: &PostgresStore,
    config: &EnvironmentRootConfig,
) -> Result<EnvironmentRootOutcome, EnvironmentRootError> {
    let now = Utc::now();
    let mut tx = store.pool().begin().await.map_err(database)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ROOT_LOCK_ID)
        .fetch_one(&mut *tx)
        .await
        .map_err(database)?;

    let mut outcome = EnvironmentRootOutcome::default();
    sync_lab(&mut tx, config, now, &mut outcome).await?;
    sync_user(&mut tx, config, now, &mut outcome).await?;
    sync_membership(&mut tx, config, now, &mut outcome).await?;
    sync_credential(&mut tx, config, now, &mut outcome).await?;

    tx.commit().await.map_err(database)?;
    Ok(outcome)
}

async fn sync_lab(
    tx: &mut Transaction<'_, Postgres>,
    config: &EnvironmentRootConfig,
    now: DateTime<Utc>,
    outcome: &mut EnvironmentRootOutcome,
) -> Result<(), EnvironmentRootError> {
    let row = sqlx::query("SELECT name, deleted_at, revision FROM labs WHERE id = $1 FOR UPDATE")
        .bind(config.lab_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database)?;
    match row {
        None => {
            sqlx::query(
                "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $3, NULL, 1)",
            )
            .bind(config.lab_id)
            .bind(&config.lab_name)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database)?;
            write_root_audit(
                tx,
                config,
                "lab",
                config.lab_id,
                "create",
                "auth.environment_root.lab.created",
                None,
                Some(json!({"name": config.lab_name, "revision": 1})),
                Some(&config.lab_name),
                Some(1),
                now,
            )
            .await?;
            outcome.lab_created = true;
        }
        Some(row) => {
            let deleted_at: Option<DateTime<Utc>> = row.try_get("deleted_at").map_err(database)?;
            if deleted_at.is_some() {
                return Err(EnvironmentRootError::IdentityConflict(
                    "configured lab is soft-deleted".to_owned(),
                ));
            }
            let name: String = row.try_get("name").map_err(database)?;
            let revision: i64 = row.try_get("revision").map_err(database)?;
            if name != config.lab_name {
                let next_revision = revision + 1;
                sqlx::query(
                    "UPDATE labs SET name = $2, updated_at = $3, revision = $4 WHERE id = $1 AND revision = $5",
                )
                .bind(config.lab_id)
                .bind(&config.lab_name)
                .bind(now)
                .bind(next_revision)
                .bind(revision)
                .execute(&mut **tx)
                .await
                .map_err(database)?;
                write_root_audit(
                    tx,
                    config,
                    "lab",
                    config.lab_id,
                    "update",
                    "auth.environment_root.lab.updated",
                    Some(json!({"name": name, "revision": revision})),
                    Some(json!({"name": config.lab_name, "revision": next_revision})),
                    Some(&config.lab_name),
                    Some(next_revision),
                    now,
                )
                .await?;
                outcome.lab_updated = true;
            }
        }
    }
    Ok(())
}

async fn sync_user(
    tx: &mut Transaction<'_, Postgres>,
    config: &EnvironmentRootConfig,
    now: DateTime<Utc>,
    outcome: &mut EnvironmentRootOutcome,
) -> Result<(), EnvironmentRootError> {
    let email_conflict: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE lab_id = $1 AND lower(email) = $2 AND id <> $3 LIMIT 1",
    )
    .bind(config.lab_id)
    .bind(&config.user_email)
    .bind(config.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database)?;
    if email_conflict.is_some() {
        return Err(EnvironmentRootError::IdentityConflict(
            "configured root email belongs to another user".to_owned(),
        ));
    }

    let row = sqlx::query(
        "SELECT lab_id, email, display_name, status, deleted_at, revision FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(config.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database)?;
    match row {
        None => {
            sqlx::query(
                "INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, 'active', $5, $5, NULL, 1)",
            )
            .bind(config.user_id)
            .bind(config.lab_id)
            .bind(&config.user_email)
            .bind(&config.user_display_name)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database)?;
            write_root_audit(
                tx,
                config,
                "user",
                config.user_id,
                "create",
                "auth.environment_root.user.created",
                None,
                Some(json!({
                    "email": config.user_email,
                    "display_name": config.user_display_name,
                    "status": "active",
                    "revision": 1
                })),
                Some(&config.user_display_name),
                Some(1),
                now,
            )
            .await?;
            outcome.user_created = true;
        }
        Some(row) => {
            let lab_id: Uuid = row.try_get("lab_id").map_err(database)?;
            let deleted_at: Option<DateTime<Utc>> = row.try_get("deleted_at").map_err(database)?;
            if lab_id != config.lab_id {
                return Err(EnvironmentRootError::IdentityConflict(
                    "configured root user belongs to another lab".to_owned(),
                ));
            }
            if deleted_at.is_some() {
                return Err(EnvironmentRootError::IdentityConflict(
                    "configured root user is soft-deleted".to_owned(),
                ));
            }
            let email: String = row.try_get("email").map_err(database)?;
            let display_name: String = row.try_get("display_name").map_err(database)?;
            let status: String = row.try_get("status").map_err(database)?;
            let revision: i64 = row.try_get("revision").map_err(database)?;
            if email != config.user_email
                || display_name != config.user_display_name
                || status != "active"
            {
                let next_revision = revision + 1;
                sqlx::query(
                    "UPDATE users SET email = $2, display_name = $3, status = 'active', updated_at = $4, revision = $5 WHERE id = $1 AND revision = $6",
                )
                .bind(config.user_id)
                .bind(&config.user_email)
                .bind(&config.user_display_name)
                .bind(now)
                .bind(next_revision)
                .bind(revision)
                .execute(&mut **tx)
                .await
                .map_err(database)?;
                write_root_audit(
                    tx,
                    config,
                    "user",
                    config.user_id,
                    "update",
                    "auth.environment_root.user.updated",
                    Some(json!({
                        "email": email,
                        "display_name": display_name,
                        "status": status,
                        "revision": revision
                    })),
                    Some(json!({
                        "email": config.user_email,
                        "display_name": config.user_display_name,
                        "status": "active",
                        "revision": next_revision
                    })),
                    Some(&config.user_display_name),
                    Some(next_revision),
                    now,
                )
                .await?;
                outcome.user_updated = true;
            }
        }
    }
    Ok(())
}

async fn sync_membership(
    tx: &mut Transaction<'_, Postgres>,
    config: &EnvironmentRootConfig,
    now: DateTime<Utc>,
    outcome: &mut EnvironmentRootOutcome,
) -> Result<(), EnvironmentRootError> {
    let row = sqlx::query(
        "SELECT id, lab_role, revision FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(config.lab_id)
    .bind(config.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database)?;
    match row {
        None => {
            let deleted_membership_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND deleted_at IS NOT NULL)",
            )
            .bind(config.lab_id)
            .bind(config.user_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database)?;
            if deleted_membership_exists {
                return Err(EnvironmentRootError::IdentityConflict(
                    "configured root lab membership is soft-deleted".to_owned(),
                ));
            }
            let membership_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO memberships (id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, NULL, $3, 'lab_admin', NULL, $4, $4, NULL, 1)",
            )
            .bind(membership_id)
            .bind(config.lab_id)
            .bind(config.user_id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database)?;
            write_root_audit(
                tx,
                config,
                "membership",
                membership_id,
                "create",
                "auth.environment_root.membership.created",
                None,
                Some(json!({"lab_role": "lab_admin", "revision": 1})),
                Some(&config.user_display_name),
                Some(1),
                now,
            )
            .await?;
            outcome.membership_created = true;
        }
        Some(row) => {
            let role: Option<String> = row.try_get("lab_role").map_err(database)?;
            let revision: i64 = row.try_get("revision").map_err(database)?;
            if role.as_deref() != Some("lab_admin") {
                let membership_id: Uuid = row.try_get("id").map_err(database)?;
                let next_revision = revision + 1;
                sqlx::query(
                    "UPDATE memberships SET lab_role = 'lab_admin', project_role = NULL, updated_at = $2, revision = $3 WHERE id = $1 AND revision = $4",
                )
                .bind(membership_id)
                .bind(now)
                .bind(next_revision)
                .bind(revision)
                .execute(&mut **tx)
                .await
                .map_err(database)?;
                write_root_audit(
                    tx,
                    config,
                    "membership",
                    membership_id,
                    "update",
                    "auth.environment_root.membership.updated",
                    Some(json!({"lab_role": role, "revision": revision})),
                    Some(json!({"lab_role": "lab_admin", "revision": next_revision})),
                    Some(&config.user_display_name),
                    Some(next_revision),
                    now,
                )
                .await?;
                outcome.membership_updated = true;
            }
        }
    }
    Ok(())
}

async fn sync_credential(
    tx: &mut Transaction<'_, Postgres>,
    config: &EnvironmentRootConfig,
    now: DateTime<Utc>,
    outcome: &mut EnvironmentRootOutcome,
) -> Result<(), EnvironmentRootError> {
    let row = sqlx::query(
        "SELECT password_hash, must_change_password, revision FROM user_credentials WHERE user_id = $1 FOR UPDATE",
    )
    .bind(config.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database)?;

    let mut revoke_sessions = outcome.user_updated;
    match row {
        None => {
            let password_hash = hash_password(config.password()).map_err(|_| {
                EnvironmentRootError::InvalidConfig(
                    "root password does not satisfy the configured password policy".to_owned(),
                )
            })?;
            sqlx::query(
                "INSERT INTO user_credentials (user_id, password_hash, created_at, password_changed_at, must_change_password, revision) VALUES ($1, $2, $3, $3, FALSE, 1)",
            )
            .bind(config.user_id)
            .bind(password_hash)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database)?;
            write_root_audit(
                tx,
                config,
                "user_credential",
                config.user_id,
                "create",
                "auth.environment_root.credential.created",
                None,
                Some(
                    json!({"algorithm": "argon2id", "must_change_password": false, "revision": 1}),
                ),
                Some(&config.user_display_name),
                Some(1),
                now,
            )
            .await?;
            outcome.credential_created = true;
            revoke_sessions = true;
        }
        Some(row) => {
            let password_hash: String = row.try_get("password_hash").map_err(database)?;
            let must_change: bool = row.try_get("must_change_password").map_err(database)?;
            let revision: i64 = row.try_get("revision").map_err(database)?;
            let password_matches = verify_password(&password_hash, config.password().as_bytes())
                .map_err(|_| {
                    EnvironmentRootError::IdentityConflict(
                        "configured root credential contains an unsupported hash".to_owned(),
                    )
                })?;
            if !password_matches || must_change {
                let next_revision = revision + 1;
                let next_hash = if password_matches {
                    password_hash
                } else {
                    hash_password(config.password()).map_err(|_| {
                        EnvironmentRootError::InvalidConfig(
                            "root password does not satisfy the configured password policy"
                                .to_owned(),
                        )
                    })?
                };
                sqlx::query(
                    "UPDATE user_credentials SET password_hash = $2, password_changed_at = $3, must_change_password = FALSE, revision = $4 WHERE user_id = $1 AND revision = $5",
                )
                .bind(config.user_id)
                .bind(next_hash)
                .bind(now)
                .bind(next_revision)
                .bind(revision)
                .execute(&mut **tx)
                .await
                .map_err(database)?;
                write_root_audit(
                    tx,
                    config,
                    "user_credential",
                    config.user_id,
                    "update",
                    "auth.environment_root.credential.updated",
                    Some(json!({"must_change_password": must_change, "revision": revision})),
                    Some(json!({
                        "algorithm": "argon2id",
                        "must_change_password": false,
                        "revision": next_revision
                    })),
                    Some(&config.user_display_name),
                    Some(next_revision),
                    now,
                )
                .await?;
                outcome.credential_updated = true;
                revoke_sessions = true;
            }
        }
    }

    if revoke_sessions {
        let session_ids: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE auth_sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL RETURNING id",
        )
        .bind(config.user_id)
        .bind(now)
        .fetch_all(&mut **tx)
        .await
        .map_err(database)?;
        outcome.sessions_revoked = session_ids.len() as u64;
        for session_id in session_ids {
            write_root_audit(
                tx,
                config,
                "auth_session",
                session_id,
                "revoke",
                "auth.environment_root.session.revoked",
                None,
                Some(json!({
                    "revoked_at": now,
                    "reason": "environment_root_configuration_changed"
                })),
                Some(&config.user_display_name),
                None,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_root_audit(
    tx: &mut Transaction<'_, Postgres>,
    config: &EnvironmentRootConfig,
    entity_type: &'static str,
    entity_id: Uuid,
    action: &'static str,
    operation_code: &'static str,
    before: Option<Value>,
    after: Option<Value>,
    entity_name: Option<&str>,
    entity_revision: Option<i64>,
    occurred_at: DateTime<Utc>,
) -> Result<(), EnvironmentRootError> {
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at, operation_code, operation_version, operation_params_json, entity_name_snapshot, entity_revision) VALUES ($1, $2, NULL, $3, $4, $5, 'system', NULL, 'MuriArc environment root', 'api', $6, 'environment root configuration reconciliation', $7, $8, $9, $10, 1, $11, $12, $13)",
    )
    .bind(Uuid::new_v4())
    .bind(config.lab_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(action)
    .bind(ROOT_REQUEST_ID)
    .bind(before)
    .bind(after)
    .bind(occurred_at)
    .bind(operation_code)
    .bind(json!({"credential_material": "redacted", "source": "environment"}))
    .bind(entity_name)
    .bind(entity_revision)
    .execute(&mut **tx)
    .await
    .map_err(database)?;
    Ok(())
}

fn clean_text(
    field: &'static str,
    value: String,
    maximum: usize,
) -> Result<String, EnvironmentRootError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control) {
        Err(EnvironmentRootError::InvalidConfig(format!(
            "{field} must contain 1-{maximum} non-control characters"
        )))
    } else {
        Ok(value)
    }
}

fn database(error: sqlx::Error) -> EnvironmentRootError {
    EnvironmentRootError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalizes_identity_and_redacts_password() {
        let config = EnvironmentRootConfig::new(
            Uuid::new_v4(),
            "  Lab  ",
            Uuid::new_v4(),
            "  ROOT@Example.org ",
            " Root Owner ",
            "root-password",
        )
        .unwrap();
        assert_eq!(config.lab_name, "Lab");
        assert_eq!(config.user_email, "root@example.org");
        assert_eq!(config.user_display_name, "Root Owner");
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("root-password"));
    }

    #[test]
    fn config_rejects_short_or_control_character_passwords() {
        assert!(
            EnvironmentRootConfig::new(
                Uuid::new_v4(),
                "Lab",
                Uuid::new_v4(),
                "root@example.org",
                "Root",
                "short",
            )
            .is_err()
        );
        assert!(
            EnvironmentRootConfig::new(
                Uuid::new_v4(),
                "Lab",
                Uuid::new_v4(),
                "root@example.org",
                "Root",
                "valid-but\nunsafe",
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn postgres_root_sync_is_idempotent_rotates_password_and_redacts_audit() {
        use std::sync::Arc;

        use crate::{AuthError, NewSession, PostgresAuthBackend, SessionBackend, token_hash};

        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
        );
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        muriarc_core::MuriArcStore::migrate(store.as_ref())
            .await
            .unwrap();

        let lab_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let email = format!("root-{}@example.org", Uuid::new_v4());
        let original_password = format!("root-original-{}", Uuid::new_v4());
        let config = EnvironmentRootConfig::new(
            lab_id,
            "Root lifecycle lab",
            user_id,
            &email,
            "Environment Owner",
            &original_password,
        )
        .unwrap();

        let first = sync_postgres_environment_root(store.as_ref(), &config)
            .await
            .unwrap();
        assert!(first.lab_created);
        assert!(first.user_created);
        assert!(first.membership_created);
        assert!(first.credential_created);
        assert_eq!(first.sessions_revoked, 0);
        assert_eq!(
            sync_postgres_environment_root(store.as_ref(), &config)
                .await
                .unwrap(),
            EnvironmentRootOutcome::default()
        );

        let auth = PostgresAuthBackend::new(store.as_ref().clone(), store.clone(), lab_id, user_id)
            .unwrap();
        let now = Utc::now();
        let raw_session = format!("mas_{}", Uuid::new_v4().simple());
        let session = NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash(&raw_session),
            csrf_hash: token_hash(&format!("mac_{}", Uuid::new_v4().simple())),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        let authenticated = auth
            .login(&email, &original_password, &session)
            .await
            .unwrap();
        assert!(authenticated.principal.is_environment_root());
        assert_eq!(
            auth.change_password(
                &authenticated.principal,
                session.id,
                &original_password,
                "application-change-must-fail",
                "root-self-change",
            )
            .await
            .unwrap_err(),
            AuthError::EnvironmentRootManaged
        );

        let replacement_password = format!("root-replacement-{}", Uuid::new_v4());
        let replacement = EnvironmentRootConfig::new(
            lab_id,
            "Root lifecycle lab",
            user_id,
            format!("updated-{email}"),
            "Updated Environment Owner",
            &replacement_password,
        )
        .unwrap();
        let changed = sync_postgres_environment_root(store.as_ref(), &replacement)
            .await
            .unwrap();
        assert!(changed.user_updated);
        assert!(changed.credential_updated);
        assert_eq!(changed.sessions_revoked, 1);
        assert_eq!(
            auth.authenticate_session(&raw_session).await.unwrap_err(),
            AuthError::InvalidCredentials
        );

        let old_attempt = NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash(&format!("mas_{}", Uuid::new_v4().simple())),
            csrf_hash: token_hash(&format!("mac_{}", Uuid::new_v4().simple())),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        assert_eq!(
            auth.login(&replacement.user_email, &original_password, &old_attempt)
                .await
                .unwrap_err(),
            AuthError::InvalidCredentials
        );
        let new_session = NewSession {
            id: Uuid::new_v4(),
            token_hash: token_hash(&format!("mas_{}", Uuid::new_v4().simple())),
            csrf_hash: token_hash(&format!("mac_{}", Uuid::new_v4().simple())),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        let relogged = auth
            .login(&replacement.user_email, &replacement_password, &new_session)
            .await
            .unwrap();
        assert!(relogged.principal.is_environment_root());
        assert_eq!(relogged.principal.display_name, "Updated Environment Owner");

        let credential: (String, bool, i64) = sqlx::query_as(
            "SELECT password_hash, must_change_password, revision FROM user_credentials WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert!(verify_password(&credential.0, replacement_password.as_bytes()).unwrap());
        assert!(!credential.1);
        assert_eq!(credential.2, 2);
        assert!(!credential.0.contains(&replacement_password));

        let audit_payloads: Vec<String> = sqlx::query_scalar(
            "SELECT concat_ws(' ', coalesce(before_json::text, ''), coalesce(after_json::text, ''), operation_params_json::text) FROM audit_entries WHERE request_id = $1",
        )
        .bind(ROOT_REQUEST_ID)
        .fetch_all(store.pool())
        .await
        .unwrap();
        let audit_payloads = audit_payloads.join("\n");
        assert!(!audit_payloads.contains(&original_password));
        assert!(!audit_payloads.contains(&replacement_password));
        assert!(!audit_payloads.contains("$argon2id$"));
        assert!(audit_payloads.contains("redacted"));
    }

    #[tokio::test]
    async fn postgres_root_sync_rejects_duplicate_cross_lab_and_soft_deleted_identities() {
        use std::sync::Arc;

        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(database_url.contains("muriarc_test"));
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        muriarc_core::MuriArcStore::migrate(store.as_ref())
            .await
            .unwrap();

        let lab_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let email = format!("identity-root-{}@example.org", Uuid::new_v4());
        let config = EnvironmentRootConfig::new(
            lab_id,
            "Root identity lab",
            user_id,
            &email,
            "Identity Root",
            "identity-root-password",
        )
        .unwrap();
        sync_postgres_environment_root(store.as_ref(), &config)
            .await
            .unwrap();

        let cross_lab = EnvironmentRootConfig::new(
            Uuid::new_v4(),
            "Another root lab",
            user_id,
            format!("cross-{email}"),
            "Cross Lab Root",
            "cross-lab-password",
        )
        .unwrap();
        assert!(matches!(
            sync_postgres_environment_root(store.as_ref(), &cross_lab).await,
            Err(EnvironmentRootError::IdentityConflict(message)) if message.contains("another lab")
        ));

        let duplicate_email = EnvironmentRootConfig::new(
            lab_id,
            "Root identity lab",
            Uuid::new_v4(),
            &email,
            "Duplicate Email Root",
            "duplicate-email-password",
        )
        .unwrap();
        assert!(matches!(
            sync_postgres_environment_root(store.as_ref(), &duplicate_email).await,
            Err(EnvironmentRootError::IdentityConflict(message)) if message.contains("another user")
        ));

        let deleted_email = format!("deleted-email-{}@example.org", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, 'Deleted identity', 'active', now(), now(), now(), 1)",
        )
        .bind(Uuid::new_v4())
        .bind(lab_id)
        .bind(deleted_email.to_ascii_uppercase())
        .execute(store.pool())
        .await
        .unwrap();
        let duplicate_deleted_email = EnvironmentRootConfig::new(
            lab_id,
            "Root identity lab",
            Uuid::new_v4(),
            &deleted_email,
            "Deleted Email Root",
            "deleted-email-password",
        )
        .unwrap();
        assert!(matches!(
            sync_postgres_environment_root(store.as_ref(), &duplicate_deleted_email).await,
            Err(EnvironmentRootError::IdentityConflict(message)) if message.contains("another user")
        ));

        let deleted_user_config = EnvironmentRootConfig::new(
            Uuid::new_v4(),
            "Deleted root user lab",
            Uuid::new_v4(),
            format!("deleted-user-{}@example.org", Uuid::new_v4()),
            "Deleted Root User",
            "deleted-user-password",
        )
        .unwrap();
        sync_postgres_environment_root(store.as_ref(), &deleted_user_config)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
            .bind(deleted_user_config.user_id)
            .execute(store.pool())
            .await
            .unwrap();
        assert!(matches!(
            sync_postgres_environment_root(store.as_ref(), &deleted_user_config).await,
            Err(EnvironmentRootError::IdentityConflict(message)) if message.contains("soft-deleted")
        ));

        let deleted_membership_config = EnvironmentRootConfig::new(
            Uuid::new_v4(),
            "Deleted root membership lab",
            Uuid::new_v4(),
            format!("deleted-membership-{}@example.org", Uuid::new_v4()),
            "Deleted Root Membership",
            "deleted-membership-password",
        )
        .unwrap();
        sync_postgres_environment_root(store.as_ref(), &deleted_membership_config)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE memberships SET deleted_at = now(), updated_at = now(), revision = revision + 1 WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL",
        )
        .bind(deleted_membership_config.lab_id)
        .bind(deleted_membership_config.user_id)
        .execute(store.pool())
        .await
        .unwrap();
        assert!(matches!(
            sync_postgres_environment_root(store.as_ref(), &deleted_membership_config).await,
            Err(EnvironmentRootError::IdentityConflict(message)) if message.contains("membership is soft-deleted")
        ));
    }
}
