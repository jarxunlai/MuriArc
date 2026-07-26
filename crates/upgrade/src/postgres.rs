use chrono::{DateTime, Duration, Utc};
use muriarc_core::{BackendKind, ReleaseIdentity};
use sqlx::{Connection, PgConnection, PgPool, Row};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    ActiveGeneration, FreezeEvidence, SwitchEvidence, UpgradeError, UpgradePhase, UpgradeSnapshot,
    UpgradeStatus, WriteLeaseEvidence,
};

pub const POSTGRES_UPGRADE_ADVISORY_LOCK_KEY: i64 = 0x4d55_5249_5550_4752_i64;

/// The dedicated connection is intentionally retained for the full operation.
/// PostgreSQL releases the session advisory lock when this value is dropped.
pub struct PostgresAdvisoryLock {
    #[allow(dead_code)]
    connection: Mutex<PgConnection>,
}

impl PostgresAdvisoryLock {
    pub async fn acquire(database_url: &str) -> Result<Self, UpgradeError> {
        let mut connection = PgConnection::connect(database_url)
            .await
            .map_err(database)?;
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(POSTGRES_UPGRADE_ADVISORY_LOCK_KEY)
            .fetch_one(&mut connection)
            .await
            .map_err(database)?;
        if !acquired {
            return Err(UpgradeError::LockBusy);
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[derive(Clone)]
pub struct PostgresUpgradeRepository {
    pool: PgPool,
}

impl PostgresUpgradeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn current_generation(&self) -> Result<ActiveGeneration, UpgradeError> {
        let row = sqlx::query(
            "SELECT application_version, data_epoch, backend_state_digest, gateway_contract_revision, generation_id, first_write_at
             FROM muriarc_deployment_state WHERE singleton = TRUE",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: "deployment state is missing".to_owned(),
        })?;
        let identity = ReleaseIdentity::parse(
            row.try_get("application_version").map_err(database)?,
            row.try_get("data_epoch").map_err(database)?,
            row.try_get("backend_state_digest").map_err(database)?,
            row.try_get("gateway_contract_revision").map_err(database)?,
        )
        .map_err(|message| UpgradeError::Prerequisite { message })?;
        Ok(ActiveGeneration {
            generation_id: row.try_get("generation_id").map_err(database)?,
            identity,
            backend: BackendKind::Postgres,
            first_write_at: row.try_get("first_write_at").map_err(database)?,
        })
    }

    pub async fn create_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        if snapshot.phase != UpgradePhase::LocksAcquired
            || snapshot.status != UpgradeStatus::Running
        {
            return Err(UpgradeError::InvalidTransition {
                from: snapshot.phase,
                to: UpgradePhase::LocksAcquired,
            });
        }
        let journal = serde_json::to_value(snapshot).map_err(serialization)?;
        let result = sqlx::query(
            "INSERT INTO muriarc_upgrade_operations (
                operation_id, source_generation_id, candidate_generation_id,
                target_application_version, target_data_epoch, target_backend_state_digest,
                target_gateway_contract_revision, maintenance_class, phase, status,
                journal_version, journal_json, started_at, updated_at, completed_at
             )
             SELECT $1, state.generation_id, NULL, $2, $3, $4, $5, $6, $7, 'running', $8, $9, $10, $10, NULL
               FROM muriarc_deployment_state AS state
               JOIN muriarc_generation_sets AS generation
                 ON generation.generation_id = state.generation_id
                AND generation.status = 'active'
              WHERE state.singleton = TRUE AND state.generation_id = $11",
        )
        .bind(snapshot.operation_id)
        .bind(&snapshot.target_application_version)
        .bind(&snapshot.target_data_epoch)
        .bind(&snapshot.target_backend_state_digest)
        .bind(&snapshot.target_gateway_contract_revision)
        .bind(migration_class(snapshot))
        .bind(phase_name(snapshot.phase))
        .bind(i32::try_from(snapshot.journal_version).map_err(|_| UpgradeError::Persistence {
            message: "journal version exceeds PostgreSQL integer range".to_owned(),
        })?)
        .bind(journal)
        .bind(snapshot.started_at)
        .bind(snapshot.source_generation_id)
        .execute(&self.pool)
        .await
        .map_err(database)?;
        if result.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "source generation is not the active deployment generation".to_owned(),
            });
        }
        Ok(())
    }

    pub async fn save_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        let journal = serde_json::to_value(snapshot).map_err(serialization)?;
        let result = sqlx::query(
            "UPDATE muriarc_upgrade_operations
                SET candidate_generation_id = $2,
                    phase = $3,
                    status = $4,
                    journal_json = $5,
                    updated_at = $6,
                    completed_at = $7
              WHERE operation_id = $1
                AND COALESCE((journal_json ->> 'revision')::BIGINT, -1) < $8",
        )
        .bind(snapshot.operation_id)
        .bind(snapshot.candidate_generation_id)
        .bind(phase_name(snapshot.phase))
        .bind(status_name(snapshot.status))
        .bind(journal)
        .bind(snapshot.updated_at)
        .bind(snapshot.completed_at)
        .bind(
            i64::try_from(snapshot.revision).map_err(|_| UpgradeError::Persistence {
                message: "operation revision exceeds PostgreSQL bigint range".to_owned(),
            })?,
        )
        .execute(&self.pool)
        .await
        .map_err(database)?;
        if result.rows_affected() != 1 {
            return Err(UpgradeError::Persistence {
                message: "operation revision was stale or operation is missing".to_owned(),
            });
        }
        Ok(())
    }

    pub async fn load_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<UpgradeSnapshot, UpgradeError> {
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT journal_json FROM muriarc_upgrade_operations WHERE operation_id = $1",
        )
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?;
        serde_json::from_value(value.ok_or(UpgradeError::OperationNotFound { operation_id })?)
            .map_err(serialization)
    }

    pub async fn begin_drain(
        &self,
        source_generation_id: Uuid,
    ) -> Result<(Uuid, i64), UpgradeError> {
        let row = sqlx::query(
            "UPDATE muriarc_write_leases AS lease
                SET status = 'draining'
               FROM muriarc_deployment_state AS state
              WHERE state.singleton = TRUE
                AND state.generation_id = $1
                AND state.write_lease_id = lease.lease_id
                AND lease.generation_id = $1
                AND lease.status = 'active'
                AND lease.expires_at > CURRENT_TIMESTAMP
          RETURNING lease.lease_id, lease.fencing_token",
        )
        .bind(source_generation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: "active source Write Lease could not enter draining state".to_owned(),
        })?;
        Ok((
            row.try_get("lease_id").map_err(database)?,
            row.try_get("fencing_token").map_err(database)?,
        ))
    }

    pub async fn freeze_writes(
        &self,
        source_generation_id: Uuid,
    ) -> Result<FreezeEvidence, UpgradeError> {
        let mut transaction = self.pool.begin().await.map_err(database)?;
        let row = sqlx::query(
            "UPDATE muriarc_write_leases AS lease
                SET status = 'revoked', revoked_at = CURRENT_TIMESTAMP
               FROM muriarc_deployment_state AS state
              WHERE state.singleton = TRUE
                AND state.generation_id = $1
                AND state.write_lease_id = lease.lease_id
                AND lease.generation_id = $1
                AND lease.status = 'draining'
          RETURNING lease.lease_id, lease.fencing_token, lease.revoked_at",
        )
        .bind(source_generation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database)?
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: "draining Write Lease could not be revoked".to_owned(),
        })?;
        sqlx::query(
            "UPDATE muriarc_deployment_state
                SET write_lease_id = NULL, updated_at = CURRENT_TIMESTAMP
              WHERE singleton = TRUE AND generation_id = $1",
        )
        .bind(source_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        transaction.commit().await.map_err(database)?;
        Ok(FreezeEvidence {
            source_generation_id,
            revoked_lease_id: row.try_get("lease_id").map_err(database)?,
            fencing_token: row.try_get("fencing_token").map_err(database)?,
            frozen_at: row.try_get("revoked_at").map_err(database)?,
        })
    }

    pub async fn create_candidate_generation(
        &self,
        generation_id: Uuid,
        target_data_epoch: &str,
        target_backend_state_digest: &str,
    ) -> Result<(), UpgradeError> {
        sqlx::query(
            "INSERT INTO muriarc_generation_sets (
                generation_id, data_epoch, backend_state_digest, status, created_at
             ) VALUES ($1, $2, $3, 'candidate', CURRENT_TIMESTAMP)
             ON CONFLICT (generation_id) DO NOTHING",
        )
        .bind(generation_id)
        .bind(target_data_epoch)
        .bind(target_backend_state_digest)
        .execute(&self.pool)
        .await
        .map_err(database)?;
        Ok(())
    }

    pub async fn activate_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate_generation_id: Uuid,
    ) -> Result<SwitchEvidence, UpgradeError> {
        let mut transaction = self.pool.begin().await.map_err(database)?;
        let state_lease: Option<Uuid> = sqlx::query_scalar(
            "SELECT write_lease_id FROM muriarc_deployment_state
              WHERE singleton = TRUE AND generation_id = $1 FOR UPDATE",
        )
        .bind(snapshot.source_generation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database)?
        .flatten();
        if state_lease.is_some() {
            return Err(UpgradeError::Prerequisite {
                message: "source generation still has a Write Lease".to_owned(),
            });
        }
        let source = sqlx::query(
            "UPDATE muriarc_generation_sets
                SET status = 'retired'
              WHERE generation_id = $1 AND status = 'active'",
        )
        .bind(snapshot.source_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        let candidate = sqlx::query(
            "UPDATE muriarc_generation_sets
                SET status = 'active', activated_at = CURRENT_TIMESTAMP
              WHERE generation_id = $1 AND status = 'candidate'",
        )
        .bind(candidate_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        if source.rows_affected() != 1 || candidate.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "source/candidate generation states cannot be atomically switched"
                    .to_owned(),
            });
        }
        sqlx::query(
            "UPDATE muriarc_deployment_state
                SET application_version = $2,
                    data_epoch = $3,
                    backend_state_digest = $4,
                    gateway_contract_revision = $5,
                    generation_id = $6,
                    write_lease_id = NULL,
                    first_write_at = NULL,
                    updated_at = CURRENT_TIMESTAMP
              WHERE singleton = TRUE AND generation_id = $1",
        )
        .bind(snapshot.source_generation_id)
        .bind(&snapshot.target_application_version)
        .bind(&snapshot.target_data_epoch)
        .bind(&snapshot.target_backend_state_digest)
        .bind(&snapshot.target_gateway_contract_revision)
        .bind(candidate_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        transaction.commit().await.map_err(database)?;
        Ok(SwitchEvidence {
            source_generation_id: snapshot.source_generation_id,
            candidate_generation_id,
            atomic: true,
            switched_at: Utc::now(),
        })
    }

    pub async fn open_write_lease(
        &self,
        generation_id: Uuid,
        holder: &str,
        ttl: Duration,
    ) -> Result<WriteLeaseEvidence, UpgradeError> {
        if holder.trim().is_empty() || ttl <= Duration::zero() {
            return Err(UpgradeError::Prerequisite {
                message: "Write Lease holder and positive TTL are required".to_owned(),
            });
        }
        let mut transaction = self.pool.begin().await.map_err(database)?;
        let fencing_token: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(fencing_token), 0) + 1 FROM muriarc_write_leases",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(database)?;
        let lease_id = Uuid::new_v4();
        let issued_at = Utc::now();
        let expires_at = issued_at + ttl;
        sqlx::query(
            "INSERT INTO muriarc_write_leases (
                lease_id, generation_id, holder, fencing_token, status, issued_at, expires_at
             )
             SELECT $1, $2, $3, $4, 'active', $5, $6
               FROM muriarc_generation_sets
              WHERE generation_id = $2 AND status = 'active'",
        )
        .bind(lease_id)
        .bind(generation_id)
        .bind(holder)
        .bind(fencing_token)
        .bind(issued_at)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        let updated = sqlx::query(
            "UPDATE muriarc_deployment_state
                SET write_lease_id = $2, updated_at = $3
              WHERE singleton = TRUE AND generation_id = $1 AND write_lease_id IS NULL",
        )
        .bind(generation_id)
        .bind(lease_id)
        .bind(issued_at)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        if updated.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "deployment is not ready to receive a new Write Lease".to_owned(),
            });
        }
        transaction.commit().await.map_err(database)?;
        Ok(WriteLeaseEvidence {
            generation_id,
            lease_id,
            fencing_token,
            expires_at,
        })
    }

    pub async fn first_write_at(
        &self,
        generation_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, UpgradeError> {
        sqlx::query_scalar(
            "SELECT first_write_at FROM muriarc_generation_sets WHERE generation_id = $1",
        )
        .bind(generation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)
        .map(Option::flatten)
    }

    pub async fn restore_source_write_lease(
        &self,
        source_generation_id: Uuid,
        holder: &str,
        ttl: Duration,
    ) -> Result<WriteLeaseEvidence, UpgradeError> {
        self.open_write_lease(source_generation_id, holder, ttl)
            .await
    }

    pub async fn rollback_to_source_before_first_write(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate_generation_id: Uuid,
        holder: &str,
        ttl: Duration,
    ) -> Result<WriteLeaseEvidence, UpgradeError> {
        if holder.trim().is_empty() || ttl <= Duration::zero() {
            return Err(UpgradeError::Prerequisite {
                message: "rollback Write Lease holder and positive TTL are required".to_owned(),
            });
        }
        let mut transaction = self.pool.begin().await.map_err(database)?;
        let first_write_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT first_write_at FROM muriarc_generation_sets
              WHERE generation_id = $1 FOR UPDATE",
        )
        .bind(candidate_generation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database)?
        .flatten();
        if let Some(first_write_at) = first_write_at {
            return Err(UpgradeError::FirstWriteBlocksRollback { first_write_at });
        }
        sqlx::query(
            "UPDATE muriarc_write_leases AS lease
                SET status = 'revoked', revoked_at = CURRENT_TIMESTAMP
               FROM muriarc_deployment_state AS state
              WHERE state.singleton = TRUE
                AND state.generation_id = $1
                AND state.write_lease_id = lease.lease_id
                AND lease.status IN ('active', 'draining')",
        )
        .bind(candidate_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        let candidate = sqlx::query(
            "UPDATE muriarc_generation_sets
                SET status = 'retired'
              WHERE generation_id = $1 AND status = 'active'",
        )
        .bind(candidate_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        let source = sqlx::query(
            "UPDATE muriarc_generation_sets
                SET status = 'active'
              WHERE generation_id = $1 AND status = 'retired'",
        )
        .bind(snapshot.source_generation_id)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        if candidate.rows_affected() != 1 || source.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "candidate/source generation cannot be rolled back atomically".to_owned(),
            });
        }
        let fencing_token: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(fencing_token), 0) + 1 FROM muriarc_write_leases",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(database)?;
        let lease_id = Uuid::new_v4();
        let issued_at = Utc::now();
        let expires_at = issued_at + ttl;
        sqlx::query(
            "INSERT INTO muriarc_write_leases (
                lease_id, generation_id, holder, fencing_token, status, issued_at, expires_at
             ) VALUES ($1, $2, $3, $4, 'active', $5, $6)",
        )
        .bind(lease_id)
        .bind(snapshot.source_generation_id)
        .bind(holder)
        .bind(fencing_token)
        .bind(issued_at)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        let updated = sqlx::query(
            "UPDATE muriarc_deployment_state AS state
                SET application_version = $2,
                    data_epoch = $3,
                    backend_state_digest = $4,
                    gateway_contract_revision = $5,
                    generation_id = $6,
                    write_lease_id = $7,
                    first_write_at = generation.first_write_at,
                    updated_at = $8
               FROM muriarc_generation_sets AS generation
              WHERE state.singleton = TRUE
                AND state.generation_id = $1
                AND generation.generation_id = $6",
        )
        .bind(candidate_generation_id)
        .bind(snapshot.source_identity.application_version.as_str())
        .bind(snapshot.source_identity.data_epoch.as_str())
        .bind(snapshot.source_identity.backend_state_digest.as_str())
        .bind(snapshot.source_identity.gateway_contract_revision.as_str())
        .bind(snapshot.source_generation_id)
        .bind(lease_id)
        .bind(issued_at)
        .execute(&mut *transaction)
        .await
        .map_err(database)?;
        if updated.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "deployment state did not point to rollback candidate".to_owned(),
            });
        }
        transaction.commit().await.map_err(database)?;
        Ok(WriteLeaseEvidence {
            generation_id: snapshot.source_generation_id,
            lease_id,
            fencing_token,
            expires_at,
        })
    }
}

fn phase_name(phase: UpgradePhase) -> &'static str {
    match phase {
        UpgradePhase::Initialized => "initialized",
        UpgradePhase::LocksAcquired => "locks_acquired",
        UpgradePhase::PreflightPassed => "preflight_passed",
        UpgradePhase::Drained => "drained",
        UpgradePhase::WritesFrozen => "writes_frozen",
        UpgradePhase::BackupCreated => "backup_created",
        UpgradePhase::BackupRestored => "backup_restored",
        UpgradePhase::CandidatePrepared => "candidate_prepared",
        UpgradePhase::CandidateMigrated => "candidate_migrated",
        UpgradePhase::CandidateVerified => "candidate_verified",
        UpgradePhase::Switched => "switched",
        UpgradePhase::ReadOnlyActivated => "read_only_activated",
        UpgradePhase::ActivationVerified => "activation_verified",
        UpgradePhase::WriteLeaseOpened => "write_lease_opened",
        UpgradePhase::Completed => "completed",
    }
}

fn status_name(status: UpgradeStatus) -> &'static str {
    match status {
        UpgradeStatus::Running => "running",
        UpgradeStatus::Succeeded => "succeeded",
        UpgradeStatus::Failed => "failed",
        UpgradeStatus::RecoveryRequired => "recovery_required",
    }
}

fn migration_class(snapshot: &UpgradeSnapshot) -> &'static str {
    match snapshot.maintenance_class {
        muriarc_core::MigrationClass::M0 => "M0",
        muriarc_core::MigrationClass::M1 => "M1",
        muriarc_core::MigrationClass::M2 => "M2",
        muriarc_core::MigrationClass::M3 => "M3",
    }
}

fn database(error: sqlx::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

fn serialization(error: serde_json::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use muriarc_core::{
        BackendStateDigest, MigrationClass, MuriArcStore, ReleaseArtifact, ReleaseManifest,
    };
    use muriarc_store_postgres::PostgresStore;

    use super::*;
    use crate::{DeploymentProfile, UpgradeSnapshot, VerifiedRelease};

    fn target() -> VerifiedRelease {
        let digest: BackendStateDigest = format!("sha256:{}", "a".repeat(64)).parse().unwrap();
        VerifiedRelease::for_test(ReleaseManifest {
            format_version: 1,
            application_version: "1.0.0".parse().unwrap(),
            data_epoch: "E0001".parse().unwrap(),
            gateway_contract_revision: "gateway-v1".parse().unwrap(),
            backend_states: BTreeMap::from([
                (BackendKind::Sqlite, digest.clone()),
                (BackendKind::Postgres, digest.clone()),
            ]),
            postgres_major: 17,
            bootstrap_protocol_revision: 1,
            controller_protocol_min: 1,
            controller_protocol_max: 1,
            migration_class: MigrationClass::M3,
            artifacts: BTreeMap::from([(
                "test".to_owned(),
                ReleaseArtifact {
                    media_type: "application/octet-stream".to_owned(),
                    digest,
                    size_bytes: 1,
                },
            )]),
        })
    }

    #[tokio::test]
    async fn postgres_control_plane_enforces_locks_journal_and_fencing() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping PostgreSQL upgrade control test: MURIARC_TEST_DATABASE_URL is not set"
            );
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "upgrade control test requires a disposable muriarc_test database"
        );
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        store.migrate().await.unwrap();
        let repository = PostgresUpgradeRepository::new(store.pool().clone());

        let first = PostgresAdvisoryLock::acquire(&database_url).await.unwrap();
        assert!(matches!(
            PostgresAdvisoryLock::acquire(&database_url).await,
            Err(UpgradeError::LockBusy)
        ));
        drop(first);
        PostgresAdvisoryLock::acquire(&database_url).await.unwrap();

        let source = repository.current_generation().await.unwrap();
        let target = target();
        let mut snapshot = UpgradeSnapshot::new(
            Uuid::new_v4(),
            DeploymentProfile::NativeSystem,
            &source,
            &target,
        )
        .unwrap();
        snapshot.advance(UpgradePhase::LocksAcquired).unwrap();
        repository.create_operation(&snapshot).await.unwrap();
        snapshot.advance(UpgradePhase::PreflightPassed).unwrap();
        repository.save_operation(&snapshot).await.unwrap();
        assert_eq!(
            repository
                .load_operation(snapshot.operation_id)
                .await
                .unwrap(),
            snapshot
        );

        let (_, draining_token) = repository.begin_drain(source.generation_id).await.unwrap();
        let freeze = repository
            .freeze_writes(source.generation_id)
            .await
            .unwrap();
        assert_eq!(freeze.fencing_token, draining_token);

        let candidate_id = Uuid::new_v4();
        repository
            .create_candidate_generation(
                candidate_id,
                &snapshot.target_data_epoch,
                &snapshot.target_backend_state_digest,
            )
            .await
            .unwrap();
        let switched = repository
            .activate_candidate(&snapshot, candidate_id)
            .await
            .unwrap();
        assert!(switched.atomic);
        let lease = repository
            .open_write_lease(candidate_id, "upgrade-test", Duration::minutes(5))
            .await
            .unwrap();
        assert!(lease.fencing_token > freeze.fencing_token);
        assert_eq!(repository.first_write_at(candidate_id).await.unwrap(), None);
        let rollback_lease = repository
            .rollback_to_source_before_first_write(
                &snapshot,
                candidate_id,
                "upgrade-test-rollback",
                Duration::minutes(5),
            )
            .await
            .unwrap();
        assert_eq!(rollback_lease.generation_id, source.generation_id);
        assert!(rollback_lease.fencing_token > lease.fencing_token);
        assert_eq!(
            repository.current_generation().await.unwrap().identity,
            source.identity
        );
    }
}
