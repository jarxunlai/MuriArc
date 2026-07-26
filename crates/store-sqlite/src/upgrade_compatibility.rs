use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use muriarc_core::{
    BackendKind, CompatibilityIssue, CompatibilityReport, DeploymentState, MigrationFingerprint,
    PersistentRecoveryInventory, ReleaseIdentity, StoreError, StoreResult, bytes_hex,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::MIGRATOR;

pub(crate) fn compiled_migrations() -> Vec<MigrationFingerprint> {
    MIGRATOR
        .iter()
        .map(|migration| MigrationFingerprint {
            version: migration.version,
            description: migration.description.to_string(),
            checksum_sha384: bytes_hex(migration.checksum.as_ref()),
        })
        .collect()
}

pub(crate) async fn compatibility_report(pool: &SqlitePool) -> StoreResult<CompatibilityReport> {
    let expected_migrations = compiled_migrations();
    let expected = ReleaseIdentity::current(BackendKind::Sqlite, &expected_migrations);
    let rows = sqlx::query(
        "SELECT version, description, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(database)?;

    let mut applied = BTreeMap::new();
    for row in rows {
        let version: i64 = row.try_get("version").map_err(database)?;
        let description: String = row.try_get("description").map_err(database)?;
        let success: bool = row.try_get("success").map_err(database)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(database)?;
        applied.insert(version, (description, success, bytes_hex(&checksum)));
    }

    let mut issues = Vec::new();
    for migration in &expected_migrations {
        match applied.remove(&migration.version) {
            None => issues.push(CompatibilityIssue::new(
                "migration_missing",
                format!("migration {} is not applied", migration.version),
            )),
            Some((_description, false, _)) => issues.push(CompatibilityIssue::new(
                "migration_failed",
                format!(
                    "migration {} is recorded as unsuccessful",
                    migration.version
                ),
            )),
            Some((description, true, checksum)) => {
                if description != migration.description {
                    issues.push(CompatibilityIssue::new(
                        "migration_description_mismatch",
                        format!("migration {} description differs", migration.version),
                    ));
                }
                if checksum != migration.checksum_sha384 {
                    issues.push(CompatibilityIssue::new(
                        "migration_checksum_mismatch",
                        format!("migration {} checksum differs", migration.version),
                    ));
                }
            }
        }
    }
    for version in applied.keys() {
        issues.push(CompatibilityIssue::new(
            "migration_unknown",
            format!("database contains unknown migration {version}"),
        ));
    }

    let observed = load_deployment_state(pool).await?;
    match &observed {
        None => issues.push(CompatibilityIssue::new(
            "deployment_state_missing",
            "database has not been adopted by the upgrade control plane",
        )),
        Some(state) => {
            compare_identity(&expected, state, &mut issues);
            validate_active_generation_and_lease(pool, state, &mut issues).await?;
        }
    }

    Ok(CompatibilityReport {
        backend: BackendKind::Sqlite,
        expected,
        observed,
        issues,
    })
}

pub(crate) async fn ensure_adopted_after_control_plane_migration(
    pool: &SqlitePool,
) -> StoreResult<()> {
    let report = compatibility_report(pool).await?;
    if report.is_compatible() {
        return Ok(());
    }
    if report.observed.is_none()
        && report.issues.len() == 1
        && report.issues[0].code == "deployment_state_missing"
    {
        adopt_current_release(pool, Uuid::new_v4()).await?;
        return Ok(());
    }
    Err(StoreError::Conflict(format!(
        "control-plane migration did not produce an activatable state: {}",
        report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

pub(crate) async fn adopt_current_release(
    pool: &SqlitePool,
    generation_id: Uuid,
) -> StoreResult<DeploymentState> {
    let report = compatibility_report(pool).await?;
    let blocking = report
        .issues
        .iter()
        .filter(|issue| issue.code != "deployment_state_missing")
        .collect::<Vec<_>>();
    if !blocking.is_empty() {
        return Err(StoreError::Conflict(format!(
            "database cannot be adopted: {}",
            blocking
                .iter()
                .map(|issue| issue.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if let Some(existing) = report.observed {
        if existing.generation_id == generation_id && existing.identity == report.expected {
            return Ok(existing);
        }
        return Err(StoreError::Conflict(
            "deployment state already belongs to another release or generation".to_owned(),
        ));
    }

    let now = Utc::now();
    let expires_at = now + Duration::days(3650);
    let lease_id = Uuid::new_v4();
    let mut tx = pool.begin().await.map_err(database)?;
    sqlx::query(
        "INSERT INTO muriarc_generation_sets (generation_id, data_epoch, backend_state_digest, status, created_at, activated_at) VALUES (?, ?, ?, 'active', ?, ?)",
    )
    .bind(generation_id.to_string())
    .bind(report.expected.data_epoch.as_str())
    .bind(report.expected.backend_state_digest.as_str())
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    sqlx::query(
        "INSERT INTO muriarc_write_leases (lease_id, generation_id, holder, fencing_token, status, issued_at, expires_at) VALUES (?, ?, 'preview-bootstrap', 1, 'active', ?, ?)",
    )
    .bind(lease_id.to_string())
    .bind(generation_id.to_string())
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    sqlx::query(
        "INSERT INTO muriarc_deployment_state (singleton, application_version, data_epoch, backend_state_digest, gateway_contract_revision, generation_id, write_lease_id, updated_at) VALUES (1, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(report.expected.application_version.as_str())
    .bind(report.expected.data_epoch.as_str())
    .bind(report.expected.backend_state_digest.as_str())
    .bind(report.expected.gateway_contract_revision.as_str())
    .bind(generation_id.to_string())
    .bind(lease_id.to_string())
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    tx.commit().await.map_err(database)?;

    Ok(DeploymentState {
        identity: report.expected,
        generation_id,
        write_lease_id: Some(lease_id),
        first_write_at: None,
        updated_at: now,
    })
}

pub(crate) async fn persistent_recovery_inventory(
    pool: &SqlitePool,
) -> StoreResult<PersistentRecoveryInventory> {
    Ok(PersistentRecoveryInventory {
        attachment_records: count(pool, "SELECT COUNT(*) FROM attachments").await?,
        encrypted_secret_records: 0,
        secret_reference_records: count(
            pool,
            "SELECT COUNT(*) FROM ai_model_profile_secret_refs WHERE credential_state = 'present'",
        )
        .await?,
        ai_history_records: count(pool, "SELECT COUNT(*) FROM ai_conversations").await?,
        audit_records: count(pool, "SELECT COUNT(*) FROM audit_entries").await?,
    })
}

async fn count(pool: &SqlitePool, statement: &str) -> StoreResult<u64> {
    let value: i64 = sqlx::query_scalar(statement)
        .fetch_one(pool)
        .await
        .map_err(database)?;
    u64::try_from(value).map_err(|_| StoreError::Database("negative row count".to_owned()))
}

async fn load_deployment_state(pool: &SqlitePool) -> StoreResult<Option<DeploymentState>> {
    let row = sqlx::query(
        "SELECT application_version, data_epoch, backend_state_digest, gateway_contract_revision, generation_id, write_lease_id, first_write_at, updated_at FROM muriarc_deployment_state WHERE singleton = 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(database)?;
    row.map(|row| {
        let generation: String = row.try_get("generation_id").map_err(database)?;
        let write_lease: Option<String> = row.try_get("write_lease_id").map_err(database)?;
        let identity = ReleaseIdentity::parse(
            row.try_get("application_version").map_err(database)?,
            row.try_get("data_epoch").map_err(database)?,
            row.try_get("backend_state_digest").map_err(database)?,
            row.try_get("gateway_contract_revision").map_err(database)?,
        )
        .map_err(StoreError::Serialization)?;
        Ok(DeploymentState {
            identity,
            generation_id: Uuid::parse_str(&generation)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
            write_lease_id: write_lease
                .map(|value| Uuid::parse_str(&value))
                .transpose()
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
            first_write_at: row.try_get("first_write_at").map_err(database)?,
            updated_at: row.try_get("updated_at").map_err(database)?,
        })
    })
    .transpose()
}

fn compare_identity(
    expected: &ReleaseIdentity,
    state: &DeploymentState,
    issues: &mut Vec<CompatibilityIssue>,
) {
    for (code, name, wanted, observed) in [
        (
            "application_version_mismatch",
            "application version",
            expected.application_version.as_str(),
            state.identity.application_version.as_str(),
        ),
        (
            "data_epoch_mismatch",
            "data epoch",
            expected.data_epoch.as_str(),
            state.identity.data_epoch.as_str(),
        ),
        (
            "backend_state_digest_mismatch",
            "backend state digest",
            expected.backend_state_digest.as_str(),
            state.identity.backend_state_digest.as_str(),
        ),
        (
            "gateway_contract_revision_mismatch",
            "gateway contract revision",
            expected.gateway_contract_revision.as_str(),
            state.identity.gateway_contract_revision.as_str(),
        ),
    ] {
        if wanted != observed {
            issues.push(CompatibilityIssue::new(
                code,
                format!("{name}: expected {wanted}, observed {observed}"),
            ));
        }
    }
}

async fn validate_active_generation_and_lease(
    pool: &SqlitePool,
    state: &DeploymentState,
    issues: &mut Vec<CompatibilityIssue>,
) -> StoreResult<()> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM muriarc_generation_sets WHERE generation_id = ?")
            .bind(state.generation_id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(database)?;
    if status.as_deref() != Some("active") {
        issues.push(CompatibilityIssue::new(
            "generation_not_active",
            "deployment generation is missing or is not active",
        ));
    }
    let Some(lease_id) = state.write_lease_id else {
        issues.push(CompatibilityIssue::new(
            "write_lease_missing",
            "active generation has no write lease",
        ));
        return Ok(());
    };
    let row = sqlx::query(
        "SELECT generation_id, status, expires_at FROM muriarc_write_leases WHERE lease_id = ?",
    )
    .bind(lease_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(database)?;
    match row {
        None => issues.push(CompatibilityIssue::new(
            "write_lease_missing",
            "deployment write lease row is missing",
        )),
        Some(row) => {
            let generation_id: String = row.try_get("generation_id").map_err(database)?;
            let status: String = row.try_get("status").map_err(database)?;
            let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(database)?;
            if generation_id != state.generation_id.to_string()
                || status != "active"
                || expires_at <= Utc::now()
            {
                issues.push(CompatibilityIssue::new(
                    "write_lease_invalid",
                    "write lease is expired, inactive, or belongs to another generation",
                ));
            }
        }
    }
    Ok(())
}

fn database(error: sqlx::Error) -> StoreError {
    StoreError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use muriarc_core::MuriArcStore;

    use super::*;
    use crate::SqliteStore;

    #[tokio::test]
    async fn adoption_is_required_before_runtime_compatibility() {
        let store = SqliteStore::in_memory().await.unwrap();
        MIGRATOR.run(store.pool()).await.unwrap();
        let report = store.compatibility_report().await.unwrap();
        assert!(!report.is_compatible());
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "deployment_state_missing")
        );

        let generation_id = Uuid::new_v4();
        store.adopt_current_release(generation_id).await.unwrap();
        let report = store.compatibility_report().await.unwrap();
        assert!(report.is_compatible(), "{:?}", report.issues);
        assert_eq!(report.observed.unwrap().generation_id, generation_id);
    }

    #[tokio::test]
    async fn deployment_digest_drift_fails_closed() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        sqlx::query(
            "UPDATE muriarc_deployment_state SET backend_state_digest = 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE singleton = 1",
        )
        .execute(store.pool())
        .await
        .unwrap();
        let report = store.compatibility_report().await.unwrap();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| { issue.code == "backend_state_digest_mismatch" })
        );
    }

    #[tokio::test]
    async fn database_write_fence_marks_first_write_and_rejects_late_writes() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let state = store
            .compatibility_report()
            .await
            .unwrap()
            .observed
            .unwrap();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES (?, 'Fence Lab', ?, ?, NULL, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
        let first_write: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT first_write_at FROM muriarc_deployment_state WHERE singleton = 1",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert!(first_write.is_some());

        sqlx::query(
            "UPDATE muriarc_write_leases SET status = 'revoked', revoked_at = ? WHERE lease_id = ?",
        )
        .bind(Utc::now())
        .bind(state.write_lease_id.unwrap().to_string())
        .execute(store.pool())
        .await
        .unwrap();
        let error = sqlx::query(
            "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES (?, 'Late Lab', ?, ?, NULL, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap_err();
        assert!(error.to_string().contains("muriarc_write_lease_required"));
    }
}
