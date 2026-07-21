use argon2::PasswordHash;
use chrono::{DateTime, Utc};
use muriarc_core::{EntityType, Lab, Membership, RecordMeta, User, UserStatus};
use muriarc_store_postgres::PostgresStore;
use serde::Serialize;
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

const BOOTSTRAP_LOCK_ID: i64 = 5_568_604_466_432_177_473;
const BOOTSTRAP_REQUEST_ID: &str = "bootstrap-seed";

#[derive(Clone, PartialEq, Eq)]
pub struct BootstrapSeedConfig {
    pub lab_id: Uuid,
    pub lab_name: String,
    pub user_id: Uuid,
    pub user_email: String,
    pub user_display_name: String,
    password_hash: Option<String>,
}

impl std::fmt::Debug for BootstrapSeedConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BootstrapSeedConfig")
            .field("lab_id", &self.lab_id)
            .field("lab_name", &self.lab_name)
            .field("user_id", &self.user_id)
            .field("user_email", &self.user_email)
            .field("user_display_name", &self.user_display_name)
            .field(
                "password_hash",
                &self.password_hash.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl BootstrapSeedConfig {
    pub fn new(
        lab_id: Uuid,
        lab_name: impl Into<String>,
        user_id: Uuid,
        user_email: impl Into<String>,
        user_display_name: impl Into<String>,
    ) -> Result<Self, BootstrapSeedError> {
        let lab_name = clean_text("lab name", lab_name.into(), 200)?;
        let user_email = clean_text("user email", user_email.into(), 320)?.to_ascii_lowercase();
        let user_display_name = clean_text("user display name", user_display_name.into(), 200)?;
        if !user_email.contains('@') {
            return Err(BootstrapSeedError::InvalidConfig(
                "bootstrap user email must contain @".to_owned(),
            ));
        }
        if lab_id == user_id {
            return Err(BootstrapSeedError::InvalidConfig(
                "bootstrap lab and user UUIDs must be different".to_owned(),
            ));
        }
        Ok(Self {
            lab_id,
            lab_name,
            user_id,
            user_email,
            user_display_name,
            password_hash: None,
        })
    }

    pub fn with_password_hash(
        mut self,
        password_hash: impl Into<String>,
    ) -> Result<Self, BootstrapSeedError> {
        let password_hash = password_hash.into();
        let parsed = PasswordHash::new(&password_hash).map_err(|_| {
            BootstrapSeedError::InvalidConfig(
                "bootstrap credential must be a valid Argon2id password hash".to_owned(),
            )
        })?;
        if parsed.algorithm.as_str() != "argon2id"
            || password_hash.len() < 32
            || password_hash.len() > 1024
            || password_hash.chars().any(char::is_control)
        {
            return Err(BootstrapSeedError::InvalidConfig(
                "bootstrap credential must be a valid Argon2id password hash".to_owned(),
            ));
        }
        self.password_hash = Some(password_hash);
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BootstrapSeedOutcome {
    pub lab_created: bool,
    pub user_created: bool,
    pub membership_created: bool,
    pub credential_created: bool,
}

impl BootstrapSeedOutcome {
    pub fn changed(self) -> bool {
        self.lab_created || self.user_created || self.membership_created || self.credential_created
    }
}

#[derive(Debug, Error)]
pub enum BootstrapSeedError {
    #[error("invalid bootstrap configuration: {0}")]
    InvalidConfig(String),
    #[error("bootstrap identity conflicts with existing data: {0}")]
    IdentityConflict(String),
    #[error("bootstrap database transaction failed: {0}")]
    Database(String),
    #[error("bootstrap audit serialization failed: {0}")]
    Serialization(String),
}

/// Idempotently creates the explicit bootstrap Lab, User, LabAdmin membership,
/// and optional first password credential in one PostgreSQL transaction.
///
/// An advisory transaction lock serializes concurrent first starts. Existing
/// records are never overwritten or revived; a conflicting identity stops
/// startup rather than silently changing laboratory data.
pub async fn seed_postgres_bootstrap(
    store: &PostgresStore,
    config: &BootstrapSeedConfig,
) -> Result<BootstrapSeedOutcome, BootstrapSeedError> {
    let now = Utc::now();
    let lab = Lab {
        id: config.lab_id,
        name: config.lab_name.clone(),
        meta: RecordMeta::new(now),
    };
    let user = User {
        id: config.user_id,
        lab_id: config.lab_id,
        email: config.user_email.clone(),
        display_name: config.user_display_name.clone(),
        status: UserStatus::Active,
        meta: RecordMeta::new(now),
    };
    let membership = Membership::lab(
        config.lab_id,
        config.user_id,
        muriarc_core::LabRole::LabAdmin,
        now,
    );

    let mut tx = store.pool().begin().await.map_err(database)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(BOOTSTRAP_LOCK_ID)
        .fetch_one(&mut *tx)
        .await
        .map_err(database)?;

    let existing_lab = sqlx::query("SELECT name, deleted_at FROM labs WHERE id = $1")
        .bind(config.lab_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?;
    let existing_user = sqlx::query("SELECT lab_id, email, deleted_at FROM users WHERE id = $1")
        .bind(config.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?;

    validate_existing_identity(config, existing_lab.as_ref(), existing_user.as_ref())?;

    let mut outcome = BootstrapSeedOutcome::default();
    if existing_lab.is_none() {
        sqlx::query(
            "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $3, NULL, 1)",
        )
        .bind(lab.id)
        .bind(&lab.name)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        write_bootstrap_audit(&mut tx, config.lab_id, EntityType::Lab, lab.id, &lab, now).await?;
        outcome.lab_created = true;
    }

    if existing_user.is_none() {
        sqlx::query(
            "INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, 'active', $5, $5, NULL, 1)",
        )
        .bind(user.id)
        .bind(user.lab_id)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        write_bootstrap_audit(
            &mut tx,
            config.lab_id,
            EntityType::User,
            user.id,
            &user,
            now,
        )
        .await?;
        outcome.user_created = true;
    }

    let membership_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND lab_role = 'lab_admin' AND deleted_at IS NULL)",
    )
    .bind(config.lab_id)
    .bind(config.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    if !membership_exists {
        sqlx::query(
            "INSERT INTO memberships (id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, NULL, $3, 'lab_admin', NULL, $4, $4, NULL, 1)",
        )
        .bind(membership.id)
        .bind(membership.lab_id)
        .bind(membership.user_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        write_bootstrap_audit(
            &mut tx,
            config.lab_id,
            EntityType::Membership,
            membership.id,
            &membership,
            now,
        )
        .await?;
        outcome.membership_created = true;
    }

    if let Some(password_hash) = &config.password_hash {
        let credential_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_credentials WHERE user_id = $1)")
                .bind(config.user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database)?;
        if !credential_exists {
            sqlx::query(
                "INSERT INTO user_credentials (user_id, password_hash, created_at, password_changed_at) VALUES ($1, $2, $3, $3)",
            )
            .bind(config.user_id)
            .bind(password_hash)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database)?;
            write_bootstrap_credential_audit(&mut tx, config, now).await?;
            outcome.credential_created = true;
        }
    }

    tx.commit().await.map_err(database)?;
    Ok(outcome)
}

fn validate_existing_identity(
    config: &BootstrapSeedConfig,
    lab: Option<&sqlx::postgres::PgRow>,
    user: Option<&sqlx::postgres::PgRow>,
) -> Result<(), BootstrapSeedError> {
    if let Some(lab) = lab {
        let name: String = lab.try_get("name").map_err(database)?;
        let deleted_at: Option<DateTime<Utc>> = lab.try_get("deleted_at").map_err(database)?;
        validate_lab_identity(config, &name, deleted_at)?;
    }
    if let Some(user) = user {
        let lab_id: Uuid = user.try_get("lab_id").map_err(database)?;
        let email: String = user.try_get("email").map_err(database)?;
        let deleted_at: Option<DateTime<Utc>> = user.try_get("deleted_at").map_err(database)?;
        validate_user_identity(config, lab_id, &email, deleted_at)?;
    }
    Ok(())
}

fn validate_lab_identity(
    config: &BootstrapSeedConfig,
    existing_name: &str,
    deleted_at: Option<DateTime<Utc>>,
) -> Result<(), BootstrapSeedError> {
    if deleted_at.is_some() {
        return Err(BootstrapSeedError::IdentityConflict(
            "configured lab UUID belongs to a soft-deleted lab".to_owned(),
        ));
    }
    if existing_name.trim() != config.lab_name {
        return Err(BootstrapSeedError::IdentityConflict(
            "configured lab UUID already exists with a different name".to_owned(),
        ));
    }
    Ok(())
}

fn validate_user_identity(
    config: &BootstrapSeedConfig,
    existing_lab_id: Uuid,
    existing_email: &str,
    deleted_at: Option<DateTime<Utc>>,
) -> Result<(), BootstrapSeedError> {
    if deleted_at.is_some() {
        return Err(BootstrapSeedError::IdentityConflict(
            "configured user UUID belongs to a soft-deleted user".to_owned(),
        ));
    }
    if existing_lab_id != config.lab_id {
        return Err(BootstrapSeedError::IdentityConflict(
            "configured user already belongs to another lab".to_owned(),
        ));
    }
    if !existing_email
        .trim()
        .eq_ignore_ascii_case(&config.user_email)
    {
        return Err(BootstrapSeedError::IdentityConflict(
            "configured user UUID already exists with a different email".to_owned(),
        ));
    }
    Ok(())
}

async fn write_bootstrap_audit<T: Serialize>(
    tx: &mut Transaction<'_, Postgres>,
    lab_id: Uuid,
    entity_type: EntityType,
    entity_id: Uuid,
    after: &T,
    occurred_at: DateTime<Utc>,
) -> Result<(), BootstrapSeedError> {
    let after = serde_json::to_value(after)
        .map_err(|error| BootstrapSeedError::Serialization(error.to_string()))?;
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, $3, $4, 'create', 'system', NULL, 'MuriArc bootstrap', 'api', $5, 'explicit one-time bootstrap seed', NULL, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(lab_id)
    .bind(entity_type.as_str())
    .bind(entity_id)
    .bind(BOOTSTRAP_REQUEST_ID)
    .bind(after)
    .bind(occurred_at)
    .execute(&mut **tx)
    .await
    .map_err(database)?;
    Ok(())
}

async fn write_bootstrap_credential_audit(
    tx: &mut Transaction<'_, Postgres>,
    config: &BootstrapSeedConfig,
    occurred_at: DateTime<Utc>,
) -> Result<(), BootstrapSeedError> {
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, 'user_credential', $3, 'create', 'system', NULL, 'MuriArc bootstrap', 'api', $4, 'explicit one-time bootstrap credential seed', NULL, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(config.lab_id)
    .bind(config.user_id)
    .bind(BOOTSTRAP_REQUEST_ID)
    .bind(serde_json::json!({"password_configured": true}))
    .bind(occurred_at)
    .execute(&mut **tx)
    .await
    .map_err(database)?;
    Ok(())
}

fn clean_text(
    field: &'static str,
    value: String,
    maximum: usize,
) -> Result<String, BootstrapSeedError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(BootstrapSeedError::InvalidConfig(format!(
            "{field} must contain 1-{maximum} non-control characters"
        )))
    } else {
        Ok(value)
    }
}

fn database(error: sqlx::Error) -> BootstrapSeedError {
    BootstrapSeedError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_core::MuriArcStore;

    #[test]
    fn config_is_trimmed_and_validated_without_reading_process_environment() {
        let config = BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "  Respiratory Lab  ",
            Uuid::new_v4(),
            "  admin@example.org ",
            "  Lab Administrator  ",
        )
        .unwrap();
        assert_eq!(config.lab_name, "Respiratory Lab");
        assert_eq!(config.user_email, "admin@example.org");
        assert_eq!(config.user_display_name, "Lab Administrator");
    }

    #[test]
    fn config_rejects_ambiguous_or_malformed_seed_identity() {
        let shared_id = Uuid::new_v4();
        assert!(
            BootstrapSeedConfig::new(shared_id, "Lab", shared_id, "admin@example.org", "Admin")
                .is_err()
        );
        assert!(
            BootstrapSeedConfig::new(
                Uuid::new_v4(),
                "Lab",
                Uuid::new_v4(),
                "not-an-email",
                "Admin"
            )
            .is_err()
        );
    }

    #[test]
    fn config_debug_never_exposes_the_password_hash() {
        let secret_hash = crate::hash_password("debug redaction password").unwrap();
        let config = BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Lab",
            Uuid::new_v4(),
            "admin@example.org",
            "Admin",
        )
        .unwrap()
        .with_password_hash(secret_hash.clone())
        .unwrap();
        let rendered = format!("{config:?}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains(&secret_hash));
    }

    #[test]
    fn outcome_reports_idempotent_noop_separately_from_creation() {
        assert!(!BootstrapSeedOutcome::default().changed());
        assert!(
            BootstrapSeedOutcome {
                lab_created: true,
                ..BootstrapSeedOutcome::default()
            }
            .changed()
        );
    }

    #[test]
    fn existing_identity_must_match_name_and_normalized_email() {
        let config = BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Respiratory Lab",
            Uuid::new_v4(),
            "Admin@Example.ORG",
            "Administrator",
        )
        .unwrap();
        assert!(validate_lab_identity(&config, "Respiratory Lab", None).is_ok());
        assert!(validate_lab_identity(&config, "Another Lab", None).is_err());
        assert!(validate_user_identity(&config, config.lab_id, "ADMIN@example.org", None).is_ok());
        assert!(
            validate_user_identity(&config, config.lab_id, "different@example.org", None).is_err()
        );
    }

    #[tokio::test]
    async fn postgres_seed_is_atomic_audited_and_idempotent_when_test_database_is_configured() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
        );
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let password_hash = crate::hash_password("bootstrap test password").unwrap();
        let config = BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Bootstrap integration lab",
            Uuid::new_v4(),
            format!("admin-{}@example.org", Uuid::new_v4()),
            "Bootstrap integration administrator",
        )
        .unwrap()
        .with_password_hash(password_hash.clone())
        .unwrap();

        let first = seed_postgres_bootstrap(&store, &config).await.unwrap();
        assert_eq!(
            first,
            BootstrapSeedOutcome {
                lab_created: true,
                user_created: true,
                membership_created: true,
                credential_created: true,
            }
        );
        assert_eq!(
            seed_postgres_bootstrap(&store, &config).await.unwrap(),
            BootstrapSeedOutcome::default()
        );

        let replacement_hash = crate::hash_password("replacement test password").unwrap();
        let replacement_config = config.clone().with_password_hash(replacement_hash).unwrap();
        assert_eq!(
            seed_postgres_bootstrap(&store, &replacement_config)
                .await
                .unwrap(),
            BootstrapSeedOutcome::default()
        );

        let membership_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memberships WHERE lab_id = $1 AND user_id = $2 AND lab_role = 'lab_admin' AND deleted_at IS NULL",
        )
        .bind(config.lab_id)
        .bind(config.user_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        let stored_password_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM user_credentials WHERE user_id = $1")
                .bind(config.user_id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_entries WHERE lab_id = $1 AND request_id = $2",
        )
        .bind(config.lab_id)
        .bind(BOOTSTRAP_REQUEST_ID)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(membership_count, 1);
        assert_eq!(stored_password_hash, password_hash);
        assert_eq!(audit_count, 4);
    }
}
