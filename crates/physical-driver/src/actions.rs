use std::{
    fs::{self, OpenOptions},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration as StdDuration,
};

use anyhow::{Context as _, Result};
use chrono::{Duration, Utc};
use flate2::read::GzDecoder;
use fs2::available_space;
use muriarc_core::{BackendKind, ReleaseIdentity};
use muriarc_delivery::{
    DeliveryError, VerifiedServerBundle, activate_staged_release, stage_verified_release,
    validate_digest_pinned_image, verify_server_bundle,
};
use muriarc_upgrade::{
    ActiveGeneration, BackupEvidence, CandidateEvidence, DeploymentProfile, DrainEvidence,
    FreezeEvidence, MigrationEvidence, PreflightEvidence, ReadOnlyActivationEvidence,
    RestoreEvidence, SwitchEvidence, UpgradeSnapshot, WriteLeaseEvidence,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, Row};
use uuid::Uuid;

use crate::{
    backup::{
        backup_artifact_path, create_backup, hash_file, prepare_candidate_copy,
        prune_backup_artifact, restore_backup_isolated, sync_candidate_to_compose, tree_size,
    },
    context::{
        DriverContext, read_environment, require_executable, require_real_directory,
        require_regular_file, safe_status, set_mode, write_bytes_atomic, write_environment,
        write_json_atomic,
    },
    model::{
        DriverOperationState, GenerationPayload, ImageLock, OperationPayload, OperationSelection,
        PruneResponse, ReadOnlyVerificationResponse, RecoveryPrunePayload, RecoveryRestorePayload,
        RestoreOperationResponse, SnapshotBackupPayload, SnapshotCandidatePayload,
        SnapshotCandidateTargetPayload, SnapshotPayload, SnapshotRestoreTargetPayload,
        SnapshotTargetPayload, StandaloneBackupPayload, TargetEnvelope, VerifiedBackupResponse,
    },
    parse_payload,
    verify::{probe_current_read_only, verify_activated_service, verify_candidate},
};

pub(crate) async fn dispatch(
    context: &DriverContext,
    action: &str,
    payload: Value,
) -> Result<Value> {
    match action {
        "current_generation" => {
            parse_empty(payload)?;
            serialize(current_generation(context).await?)
        }
        "create_operation" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            create_operation(context, &payload.snapshot).await?;
            serialize(())
        }
        "save_operation" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            save_operation(context, &payload.snapshot).await?;
            serialize(())
        }
        "load_operation" => {
            let payload: OperationPayload = parse_payload(payload)?;
            serialize(load_operation(context, payload.operation_id).await?)
        }
        "preflight" => {
            let payload: SnapshotTargetPayload = parse_payload(payload)?;
            serialize(preflight(context, &payload.snapshot, &payload.target).await?)
        }
        "drain" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            serialize(drain(context, &payload.snapshot).await?)
        }
        "freeze_writes" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            serialize(freeze_writes(context, &payload.snapshot).await?)
        }
        "create_backup" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            let mut state = context.load_operation_state(payload.snapshot.operation_id)?;
            let source = source_generation_from_snapshot(&payload.snapshot);
            serialize(create_backup(context, &mut state, &source).await?)
        }
        "verify_backup_restore" => {
            let payload: SnapshotBackupPayload = parse_payload(payload)?;
            let mut state = context.load_operation_state(payload.snapshot.operation_id)?;
            serialize(restore_backup_isolated(context, &mut state, &payload.backup).await?)
        }
        "prepare_candidate" => {
            let payload: SnapshotRestoreTargetPayload = parse_payload(payload)?;
            serialize(
                prepare_candidate(
                    context,
                    &payload.snapshot,
                    &payload.restore,
                    &payload.target,
                )
                .await?,
            )
        }
        "migrate_candidate" => {
            let payload: SnapshotCandidateTargetPayload = parse_payload(payload)?;
            serialize(
                migrate_candidate(
                    context,
                    &payload.snapshot,
                    &payload.candidate,
                    &payload.target,
                )
                .await?,
            )
        }
        "verify_candidate" => {
            let payload: SnapshotCandidatePayload = parse_payload(payload)?;
            let mut state = context.load_operation_state(payload.snapshot.operation_id)?;
            anyhow::ensure!(
                state.candidate_generation_id == Some(payload.candidate.generation_id),
                "Candidate identity differs from operation state"
            );
            let evidence = verify_candidate(context, &state, &payload.snapshot).await?;
            state.candidate_verification = Some(evidence.clone());
            state.updated_at = Utc::now();
            context.save_operation_state(&state)?;
            serialize(evidence)
        }
        "switch_generation" => {
            let payload: SnapshotCandidatePayload = parse_payload(payload)?;
            serialize(switch_generation(context, &payload.snapshot, &payload.candidate).await?)
        }
        "activate_read_only" => {
            let payload: SnapshotCandidatePayload = parse_payload(payload)?;
            serialize(activate_read_only(context, &payload.snapshot, &payload.candidate).await?)
        }
        "verify_activated" => {
            let payload: SnapshotCandidatePayload = parse_payload(payload)?;
            let state = context.load_operation_state(payload.snapshot.operation_id)?;
            anyhow::ensure!(
                state.candidate_generation_id == Some(payload.candidate.generation_id),
                "activated Candidate identity differs"
            );
            serialize(verify_activated_service(context, &state).await?)
        }
        "open_write_lease" => {
            let payload: SnapshotCandidatePayload = parse_payload(payload)?;
            serialize(open_write_lease(context, &payload.snapshot, &payload.candidate).await?)
        }
        "first_write_at" => {
            let payload: GenerationPayload = parse_payload(payload)?;
            let repository = context.repository(context.active_database()).await?;
            serialize(repository.first_write_at(payload.generation_id).await?)
        }
        "recover_before_first_write" => {
            let payload: SnapshotPayload = parse_payload(payload)?;
            recover_before_first_write(context, &payload.snapshot).await?;
            serialize(())
        }
        "standalone_backup_create" => {
            let payload: StandaloneBackupPayload = parse_payload(payload)?;
            serialize(standalone_backup_create(context, payload.source_generation).await?)
        }
        "standalone_backup_verify" => {
            parse_empty(payload)?;
            serialize(standalone_backup_verify(context).await?)
        }
        "verify_read_only" => {
            parse_empty(payload)?;
            serialize(verify_read_only(context).await?)
        }
        "latest_resumable_operation" => {
            parse_empty(payload)?;
            serialize(latest_resumable_operation(context).await?)
        }
        "recovery_restore" => {
            let payload: RecoveryRestorePayload = parse_payload(payload)?;
            serialize(recovery_restore(context, payload).await?)
        }
        "recovery_prune" => {
            let payload: RecoveryPrunePayload = parse_payload(payload)?;
            let backup = &payload.recovery_point.backup;
            payload.recovery_point.restore.validate(backup)?;
            prune_backup_artifact(context, backup)?;
            serialize(PruneResponse {
                backup_id: backup.backup_id,
                artifact_deleted: !backup_artifact_path(context, backup.backup_id).exists(),
            })
        }
        _ => anyhow::bail!("unknown physical Driver action"),
    }
}

async fn current_generation(context: &DriverContext) -> Result<ActiveGeneration> {
    context
        .repository(context.active_database())
        .await?
        .current_generation()
        .await
        .map_err(Into::into)
}

async fn create_operation(context: &DriverContext, snapshot: &UpgradeSnapshot) -> Result<()> {
    anyhow::ensure!(
        snapshot.profile == context.profile(),
        "snapshot profile differs"
    );
    let repository = context.repository(context.active_database()).await?;
    match repository.create_operation(snapshot).await {
        Ok(()) => {}
        Err(_) => {
            let existing = repository.load_operation(snapshot.operation_id).await?;
            anyhow::ensure!(existing == *snapshot, "persisted operation differs");
        }
    }
    context.create_operation_state(snapshot)?;
    Ok(())
}

async fn save_operation(context: &DriverContext, snapshot: &UpgradeSnapshot) -> Result<()> {
    let repository = context
        .repository_for_operation(snapshot.operation_id)
        .await?;
    let existing = repository.load_operation(snapshot.operation_id).await?;
    if existing == *snapshot {
        return Ok(());
    }
    anyhow::ensure!(
        existing.revision < snapshot.revision,
        "operation revision moved backwards"
    );
    repository.save_operation(snapshot).await?;
    Ok(())
}

async fn load_operation(context: &DriverContext, operation_id: Uuid) -> Result<UpgradeSnapshot> {
    context
        .repository_for_operation(operation_id)
        .await?
        .load_operation(operation_id)
        .await
        .map_err(Into::into)
}

async fn preflight(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    target: &TargetEnvelope,
) -> Result<PreflightEvidence> {
    validate_target(context, snapshot, target)?;
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    let (release_path, bundle, target_server_image) =
        stage_target_artifact(context, &state, target)?;
    state.target_release_path = Some(release_path);
    state.target_bundle = Some(bundle);
    state.target_server_image = target_server_image;
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    let target_artifact = context.target_artifact_path()?;
    let artifact_size = fs::metadata(&target_artifact)?.len();
    let pool = context.pool(&state.source_database).await?;
    let database_size: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(&pool)
        .await?;
    pool.close().await;
    let generation_size = if context.profile() == DeploymentProfile::NativeSystem {
        tree_size(&context.generation_root(snapshot.source_generation_id))?
    } else {
        0
    };
    let required_bytes = u64::try_from(database_size)?
        .saturating_mul(4)
        .saturating_add(generation_size.saturating_mul(3))
        .saturating_add(artifact_size.saturating_mul(3))
        .saturating_add(4 * 1024 * 1024 * 1024);
    let free_bytes = available_space(&context.config.paths.data_root)?;
    Ok(PreflightEvidence {
        source_generation_id: snapshot.source_generation_id,
        target_application_version: target.manifest.application_version.to_string(),
        free_bytes,
        required_bytes,
        maintenance_class: target.manifest.migration_class,
        recovery_prerequisites_satisfied: backup_prerequisites_ready(context),
        checked_at: Utc::now(),
    })
}

async fn drain(context: &DriverContext, snapshot: &UpgradeSnapshot) -> Result<DrainEvidence> {
    drain_generation(
        context,
        snapshot.operation_id,
        snapshot.source_generation_id,
    )
    .await
}

async fn drain_generation(
    context: &DriverContext,
    operation_id: Uuid,
    source_generation_id: Uuid,
) -> Result<DrainEvidence> {
    let state = context.load_operation_state(operation_id)?;
    let mut connection =
        PgConnection::connect(&context.endpoint(&state.source_database)?.connection_url()).await?;
    let changed = sqlx::query(
        "UPDATE muriarc_write_leases AS lease
            SET status = 'draining'
           FROM muriarc_deployment_state AS state
          WHERE state.singleton = TRUE
            AND state.generation_id = $1
            AND state.write_lease_id = lease.lease_id
            AND lease.generation_id = $1
            AND lease.status = 'active'
            AND lease.expires_at > CURRENT_TIMESTAMP",
    )
    .bind(source_generation_id)
    .execute(&mut connection)
    .await?;
    if changed.rows_affected() == 0 {
        let status: Option<String> = sqlx::query_scalar(
            "SELECT lease.status FROM muriarc_write_leases AS lease
               JOIN muriarc_deployment_state AS state ON state.write_lease_id = lease.lease_id
              WHERE state.singleton = TRUE AND state.generation_id = $1",
        )
        .bind(source_generation_id)
        .fetch_optional(&mut connection)
        .await?;
        anyhow::ensure!(
            matches!(status.as_deref(), Some("draining" | "revoked")) || status.is_none(),
            "source Write Lease did not enter draining"
        );
    }
    drop(connection);
    context.service_controller()?.stop_for_drain()?;
    let mut connection =
        PgConnection::connect(&context.endpoint(&state.source_database)?.connection_url()).await?;
    let other_clients: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_stat_activity AS activity
          WHERE activity.datname = current_database()
            AND activity.pid <> pg_backend_pid()
            AND activity.backend_type = 'client backend'
            AND NOT EXISTS (
                SELECT 1 FROM pg_locks AS lock
                 WHERE lock.pid = activity.pid AND lock.locktype = 'advisory'
            )",
    )
    .fetch_one(&mut connection)
    .await?;
    anyhow::ensure!(
        other_clients == 0,
        "live database clients remained after drain"
    );
    Ok(DrainEvidence {
        inflight_requests: 0,
        running_jobs: 0,
        pending_attachment_writes: 0,
        provider_requests: 0,
        drained_at: Utc::now(),
    })
}

async fn freeze_writes(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
) -> Result<FreezeEvidence> {
    freeze_generation(
        context,
        snapshot.operation_id,
        snapshot.source_generation_id,
    )
    .await
}

async fn freeze_generation(
    context: &DriverContext,
    operation_id: Uuid,
    source_generation_id: Uuid,
) -> Result<FreezeEvidence> {
    let state = context.load_operation_state(operation_id)?;
    let pool = context.pool(&state.source_database).await?;
    let mut transaction = pool.begin().await?;
    let row = match sqlx::query(
        "UPDATE muriarc_write_leases AS lease
            SET status = 'revoked', revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP)
           FROM muriarc_deployment_state AS state
          WHERE state.singleton = TRUE
            AND state.generation_id = $1
            AND state.write_lease_id = lease.lease_id
            AND lease.generation_id = $1
            AND lease.status IN ('draining', 'revoked')
      RETURNING lease.lease_id, lease.fencing_token, lease.revoked_at",
    )
    .bind(source_generation_id)
    .fetch_optional(&mut *transaction)
    .await?
    {
        Some(row) => row,
        None => sqlx::query(
            "SELECT lease.lease_id, lease.fencing_token, lease.revoked_at
               FROM muriarc_write_leases AS lease
               JOIN muriarc_deployment_state AS state ON state.singleton = TRUE
              WHERE state.generation_id = $1
                AND state.write_lease_id IS NULL
                AND lease.generation_id = $1
                AND lease.status = 'revoked'
                AND lease.revoked_at IS NOT NULL
              ORDER BY lease.revoked_at DESC, lease.fencing_token DESC LIMIT 1",
        )
        .bind(source_generation_id)
        .fetch_optional(&mut *transaction)
        .await?
        .context("draining Write Lease could not be revoked")?,
    };
    let lease_id: Uuid = row.try_get("lease_id")?;
    let cleared = sqlx::query(
        "UPDATE muriarc_deployment_state
            SET write_lease_id = NULL, updated_at = CURRENT_TIMESTAMP
          WHERE singleton = TRUE
            AND generation_id = $1
            AND (write_lease_id = $2 OR write_lease_id IS NULL)",
    )
    .bind(source_generation_id)
    .bind(lease_id)
    .execute(&mut *transaction)
    .await?;
    anyhow::ensure!(
        cleared.rows_affected() == 1,
        "deployment Write Lease changed during freeze"
    );
    transaction.commit().await?;
    pool.close().await;
    Ok(FreezeEvidence {
        source_generation_id,
        revoked_lease_id: lease_id,
        fencing_token: row.try_get("fencing_token")?,
        frozen_at: row.try_get("revoked_at")?,
    })
}

async fn prepare_candidate(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    restore: &RestoreEvidence,
    target: &TargetEnvelope,
) -> Result<CandidateEvidence> {
    validate_target(context, snapshot, target)?;
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    anyhow::ensure!(
        state
            .backup
            .as_ref()
            .is_some_and(|backup| restore.validate(backup).is_ok()),
        "Candidate preparation lacks verified restore"
    );
    let generation_id = state.candidate_generation_id.unwrap_or_else(Uuid::new_v4);
    let _ = prepare_candidate_copy(context, &mut state, generation_id).await?;
    Ok(CandidateEvidence {
        generation_id,
        isolated: true,
        private_endpoint: true,
        external_providers_disabled: true,
        background_jobs_disabled: true,
        real_user_writes_disabled: true,
        prepared_at: Utc::now(),
    })
}

async fn migrate_candidate(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    candidate: &CandidateEvidence,
    target: &TargetEnvelope,
) -> Result<MigrationEvidence> {
    validate_target(context, snapshot, target)?;
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    anyhow::ensure!(
        state.candidate_generation_id == Some(candidate.generation_id),
        "Candidate migration identity differs"
    );
    let database = state
        .candidate_database
        .as_deref()
        .context("Candidate database is missing")?;
    let root = state
        .candidate_root
        .as_ref()
        .context("Candidate generation root is missing")?;
    let release = state
        .target_release_path
        .as_ref()
        .context("target release path is missing")?;
    let executor = release.join("bin/muriarc-upgrade-executor");
    require_executable(&executor, "final upgrade executor")?;
    let manifest_path = context
        .operation_root(snapshot.operation_id)
        .join("signed-target-release-manifest.json");
    write_json_atomic(&manifest_path, &target.manifest, 0o600)?;
    let output = Command::new(&executor)
        .arg("apply")
        .args(["--release-manifest"])
        .arg(&manifest_path)
        .args(["--operation", &snapshot.operation_id.to_string()])
        .args([
            "--source-generation",
            &snapshot.source_generation_id.to_string(),
        ])
        .args([
            "--candidate-generation",
            &candidate.generation_id.to_string(),
        ])
        .args(["--output", "json"])
        .env_clear()
        .env(
            "MURIARC_CANDIDATE_DATABASE_URL",
            context.endpoint(database)?.connection_url(),
        )
        .env("MURIARC_CANDIDATE_DATABASE_NAME", database)
        .env("MURIARC_CANDIDATE_DATA_ROOT", root.join("data"))
        .env(
            "MURIARC_CANDIDATE_ATTACHMENT_ROOT",
            root.join("attachments"),
        )
        .env(
            "MURIARC_CANDIDATE_MASTER_KEY_FILE",
            root.join("secrets/ai-master-key"),
        )
        .env("MURIARC_CANDIDATE_EXTERNAL_PROVIDERS_DISABLED", "true")
        .env("MURIARC_CANDIDATE_BACKGROUND_JOBS_DISABLED", "true")
        .env("MURIARC_CANDIDATE_REAL_USER_WRITES_DISABLED", "true")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    anyhow::ensure!(
        output.status.success() && output.stdout.len() <= 1024 * 1024,
        "final executor failed"
    );
    let response: ExecutorResponse = serde_json::from_slice(&output.stdout)?;
    let expected_identity = release_identity(&target.manifest)?;
    anyhow::ensure!(
        response.ok
            && response.operation_id == snapshot.operation_id
            && response.source_generation_id == snapshot.source_generation_id
            && response.candidate_generation_id == candidate.generation_id
            && response.target_identity == expected_identity
            && response.write_lease_absent,
        "final executor response differs from signed target"
    );
    require_regular_file(
        &PathBuf::from(&response.generation_manifest),
        "Candidate generation manifest",
    )?;
    sync_candidate_to_compose(context, candidate.generation_id, root)?;
    state.candidate_identity = Some(expected_identity.clone());
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    Ok(MigrationEvidence {
        generation_id: candidate.generation_id,
        identity: expected_identity,
        migration_path: vec![
            snapshot.source_identity.data_epoch.to_string(),
            target.manifest.data_epoch.to_string(),
        ],
        completed_at: Utc::now(),
    })
}

async fn switch_generation(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    candidate: &CandidateEvidence,
) -> Result<SwitchEvidence> {
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    if state.switched && !state.recovered {
        return Ok(SwitchEvidence {
            source_generation_id: snapshot.source_generation_id,
            candidate_generation_id: candidate.generation_id,
            atomic: true,
            switched_at: state.updated_at,
        });
    }
    anyhow::ensure!(
        state.candidate_generation_id == Some(candidate.generation_id)
            && state
                .candidate_verification
                .as_ref()
                .is_some_and(|evidence| {
                    evidence.generation_id == candidate.generation_id && evidence.validate().is_ok()
                }),
        "Candidate switch lacks complete seven-layer verification"
    );
    let candidate_database = state
        .candidate_database
        .as_deref()
        .context("Candidate database is missing")?;
    let candidate_repository = context.repository(candidate_database).await?;
    let candidate_snapshot = candidate_repository
        .load_operation(snapshot.operation_id)
        .await?;
    if candidate_snapshot != *snapshot {
        anyhow::ensure!(
            candidate_snapshot.revision < snapshot.revision,
            "Candidate operation journal is ahead of the host snapshot"
        );
        candidate_repository.save_operation(snapshot).await?;
    }
    backup_control_files(context, snapshot.operation_id)?;
    state.cloudflared_was_active = stop_cloudflared_gate()?;
    let mut activation = context.activation.clone();
    activation.insert(
        "MURIARC_ACTIVE_GENERATION".to_owned(),
        candidate.generation_id.to_string(),
    );
    activation.insert("MURIARC_ACTIVATION_MODE".to_owned(), "read-only".to_owned());
    match context.profile() {
        DeploymentProfile::NativeSystem => {
            let root = context.generation_root(candidate.generation_id);
            activation.insert(
                "MURIARC_DATABASE_URL".to_owned(),
                context.endpoint(candidate_database)?.connection_url(),
            );
            activation.insert(
                "MURIARC_DATA_ROOT".to_owned(),
                root.join("data").display().to_string(),
            );
            activation.insert(
                "MURIARC_ATTACHMENT_ROOT".to_owned(),
                root.join("attachments").display().to_string(),
            );
            activation.insert(
                "MURIARC_AI_MASTER_KEY_FILE".to_owned(),
                root.join("secrets/ai-master-key").display().to_string(),
            );
        }
        DeploymentProfile::ManagedCompose => {
            activation.insert(
                "MURIARC_POSTGRES_DB".to_owned(),
                candidate_database.to_owned(),
            );
            let image = state
                .target_server_image
                .as_ref()
                .context("target Server image is missing")?;
            let mut environment = context.environment.clone();
            environment.insert("MURIARC_SERVER_IMAGE".to_owned(), image.clone());
            write_environment(&context.config.environment_file, &environment)?;
        }
        DeploymentProfile::Desktop => unreachable!(),
    }
    let target_release = state
        .target_release_path
        .as_ref()
        .context("target release path is missing")?;
    let target_bundle = state
        .target_bundle
        .as_ref()
        .context("target bundle is missing")?;
    write_environment(&context.config.activation_file, &activation)?;
    if let Err(error) = activate_staged_release(target_release, target_bundle, &context.config) {
        let _ = restore_control_files(context, snapshot.operation_id);
        return Err(error.into());
    }
    state.switched = true;
    state.recovered = false;
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    Ok(SwitchEvidence {
        source_generation_id: snapshot.source_generation_id,
        candidate_generation_id: candidate.generation_id,
        atomic: true,
        switched_at: state.updated_at,
    })
}

async fn activate_read_only(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    candidate: &CandidateEvidence,
) -> Result<ReadOnlyActivationEvidence> {
    let state = context.load_operation_state(snapshot.operation_id)?;
    anyhow::ensure!(
        state.switched
            && !state.recovered
            && state.candidate_generation_id == Some(candidate.generation_id),
        "Candidate was not switched behind the traffic gate"
    );
    context.service_controller()?.start_read_only()?;
    wait_loopback_ready().await?;
    Ok(ReadOnlyActivationEvidence {
        generation_id: candidate.generation_id,
        write_lease_absent: true,
        external_traffic_blocked: true,
        activated_at: Utc::now(),
    })
}

async fn open_write_lease(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    candidate: &CandidateEvidence,
) -> Result<WriteLeaseEvidence> {
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    anyhow::ensure!(
        state.switched
            && !state.recovered
            && state.candidate_generation_id == Some(candidate.generation_id),
        "Candidate is not the active generation"
    );
    let database = state
        .candidate_database
        .as_deref()
        .context("Candidate database is missing")?;
    let repository = context.repository(database).await?;
    let existing = active_write_lease(repository.pool(), candidate.generation_id).await?;
    let evidence = match existing {
        Some(evidence) => evidence,
        None => {
            repository
                .open_write_lease(
                    candidate.generation_id,
                    "muriarcctl-physical-driver",
                    Duration::minutes(15),
                )
                .await?
        }
    };
    let mut activation = read_environment(&context.config.activation_file)?;
    activation.insert(
        "MURIARC_ACTIVATION_MODE".to_owned(),
        "read-write".to_owned(),
    );
    write_environment(&context.config.activation_file, &activation)?;
    context.service_controller()?.restart()?;
    wait_loopback_ready().await?;
    if state.cloudflared_was_active {
        start_cloudflared_gate()?;
    }
    state.write_lease_opened = true;
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    Ok(evidence)
}

async fn recover_before_first_write(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
) -> Result<()> {
    let mut state = context.load_operation_state(snapshot.operation_id)?;
    if !state.switched || state.recovered {
        return Ok(());
    }
    let candidate_id = state
        .candidate_generation_id
        .context("Candidate generation is missing")?;
    let candidate_database = state
        .candidate_database
        .as_deref()
        .context("Candidate database is missing")?;
    let candidate_repository = context.repository(candidate_database).await?;
    anyhow::ensure!(
        candidate_repository
            .first_write_at(candidate_id)
            .await?
            .is_none(),
        "Candidate has already accepted its first write"
    );
    let _ = context.service_controller()?.stop_for_drain();
    let source_repository = context.repository(&state.source_database).await?;
    if active_write_lease(source_repository.pool(), state.source_generation_id)
        .await?
        .is_none()
    {
        source_repository
            .restore_source_write_lease(
                state.source_generation_id,
                "muriarcctl-recovery",
                Duration::minutes(15),
            )
            .await?;
    }
    restore_control_files(context, snapshot.operation_id)?;
    activate_staged_release(
        &state.source_release_path,
        &state.source_bundle,
        &context.config,
    )?;
    context.service_controller()?.start_read_only()?;
    wait_loopback_ready().await?;
    if state.cloudflared_was_active {
        start_cloudflared_gate()?;
    }
    state.recovered = true;
    state.switched = false;
    state.write_lease_opened = false;
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    Ok(())
}

async fn standalone_backup_create(
    context: &DriverContext,
    source: ActiveGeneration,
) -> Result<BackupEvidence> {
    let current = current_generation(context).await?;
    anyhow::ensure!(current == source, "standalone backup source is not active");
    let operation_id = Uuid::new_v4();
    let root = context.operation_root(operation_id);
    fs::create_dir_all(&root)?;
    set_mode(&root, 0o700)?;
    let now = Utc::now();
    let mut state = DriverOperationState {
        format_version: crate::model::DRIVER_STATE_FORMAT,
        operation_id,
        source_generation_id: source.generation_id,
        source_database: context.active_database().to_owned(),
        source_release_path: context.receipt.release_path.clone(),
        source_bundle: context.bundle.clone(),
        target_release_path: None,
        target_bundle: None,
        target_server_image: None,
        backup: None,
        recovery_set_digest: None,
        restore_database: None,
        restore_database_digest: None,
        restore_root: None,
        candidate_generation_id: None,
        candidate_database: None,
        candidate_root: None,
        candidate_identity: None,
        candidate_verification: None,
        cloudflared_was_active: false,
        switched: false,
        write_lease_opened: false,
        recovered: false,
        created_at: now,
        updated_at: now,
    };
    context.save_operation_state(&state)?;
    state.cloudflared_was_active = stop_cloudflared_gate()?;
    state.updated_at = Utc::now();
    context.save_operation_state(&state)?;
    let backup_result = async {
        drain_generation(context, operation_id, source.generation_id).await?;
        freeze_generation(context, operation_id, source.generation_id).await?;
        create_backup(context, &mut state, &source).await
    }
    .await;
    let recovery_result = restore_source_after_maintenance(
        context,
        operation_id,
        source.generation_id,
        state.cloudflared_was_active,
    )
    .await;
    let evidence = match (backup_result, recovery_result) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(recovery)) => {
            anyhow::bail!("backup completed but source service recovery failed: {recovery}")
        }
        (Err(error), Err(recovery)) => {
            anyhow::bail!("backup failed: {error}; source service recovery also failed: {recovery}")
        }
    };
    write_json_atomic(
        &context
            .state_root()
            .join("physical-driver/latest-standalone-backup.json"),
        &json!({ "operation_id": operation_id }),
        0o600,
    )?;
    Ok(evidence)
}

async fn standalone_backup_verify(context: &DriverContext) -> Result<VerifiedBackupResponse> {
    let marker: OperationSelection = serde_json::from_slice(&fs::read(
        context
            .state_root()
            .join("physical-driver/latest-standalone-backup.json"),
    )?)?;
    let mut state = context.load_operation_state(marker.operation_id)?;
    let backup = state
        .backup
        .clone()
        .context("standalone backup is missing")?;
    let restore = restore_backup_isolated(context, &mut state, &backup).await?;
    Ok(VerifiedBackupResponse { backup, restore })
}

async fn verify_read_only(context: &DriverContext) -> Result<ReadOnlyVerificationResponse> {
    let generation = current_generation(context).await?;
    let mut state = find_state_for_active_generation(context, generation.generation_id)?;
    let mut verification = state
        .candidate_verification
        .take()
        .context("active generation has no prior seven-layer physical verification")?;
    let (before, after, read_only_digest) =
        probe_current_read_only(context, context.active_database()).await?;
    verification.layers.insert(
        muriarc_upgrade::VerificationLayer::ReadOnlyNoSideEffects,
        muriarc_upgrade::VerificationLayerEvidence {
            evidence_digest: read_only_digest,
            verified_at: Utc::now(),
        },
    );
    verification.validate()?;
    Ok(ReadOnlyVerificationResponse {
        state_digest_before: before,
        state_digest_after: after,
        verification,
    })
}

async fn latest_resumable_operation(context: &DriverContext) -> Result<OperationSelection> {
    let pool = context.pool(context.active_database()).await?;
    let operation_id: Uuid = sqlx::query_scalar(
        "SELECT operation_id FROM muriarc_upgrade_operations
          WHERE status = 'running' ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    pool.close().await;
    Ok(OperationSelection { operation_id })
}

async fn recovery_restore(
    context: &DriverContext,
    payload: RecoveryRestorePayload,
) -> Result<RestoreOperationResponse> {
    let backup = payload.recovery_point.backup;
    payload.recovery_point.restore.validate(&backup)?;
    let state = find_state_for_backup(context, backup.backup_id)?;
    let current = current_generation(context).await?;
    if current.first_write_at.is_some() {
        anyhow::ensure!(
            payload.confirm_data_loss,
            "explicit data-loss confirmation is required after first write"
        );
    }
    anyhow::ensure!(
        state.restore_root.is_some() && state.restore_database.is_some(),
        "recovery point lacks an isolated verified restore"
    );
    let source_root = state
        .restore_root
        .as_ref()
        .expect("checked restore root")
        .join("generation");
    let active_root = context.generation_root(backup.source_generation_id);
    if active_root.exists() {
        anyhow::ensure!(
            crate::backup::tree_content_digest(&active_root)?
                == crate::backup::tree_content_digest(&source_root)?,
            "existing recovery generation differs from verified backup"
        );
    } else if context.profile() == DeploymentProfile::NativeSystem {
        copy_recovery_tree(&source_root, &active_root)?;
    } else {
        sync_candidate_to_compose(context, backup.source_generation_id, &source_root)?;
    }
    let _ = context.service_controller()?.stop_for_drain();
    backup_control_files(context, state.operation_id)?;
    let mut environment = read_environment(
        &state
            .restore_root
            .as_ref()
            .expect("checked restore root")
            .join("configuration/server.env"),
    )?;
    let mut activation = read_environment(
        &state
            .restore_root
            .as_ref()
            .expect("checked restore root")
            .join("configuration/active.env"),
    )?;
    let restore_database = state
        .restore_database
        .as_deref()
        .expect("checked restore database");
    activation.insert(
        "MURIARC_ACTIVE_GENERATION".to_owned(),
        backup.source_generation_id.to_string(),
    );
    activation.insert(
        "MURIARC_ACTIVATION_MODE".to_owned(),
        "read-write".to_owned(),
    );
    match context.profile() {
        DeploymentProfile::NativeSystem => {
            activation.insert(
                "MURIARC_DATABASE_URL".to_owned(),
                context.endpoint(restore_database)?.connection_url(),
            );
            activation.insert(
                "MURIARC_DATA_ROOT".to_owned(),
                active_root.join("data").display().to_string(),
            );
            activation.insert(
                "MURIARC_ATTACHMENT_ROOT".to_owned(),
                active_root.join("attachments").display().to_string(),
            );
            activation.insert(
                "MURIARC_AI_MASTER_KEY_FILE".to_owned(),
                active_root
                    .join("secrets/ai-master-key")
                    .display()
                    .to_string(),
            );
        }
        DeploymentProfile::ManagedCompose => {
            activation.insert(
                "MURIARC_POSTGRES_DB".to_owned(),
                restore_database.to_owned(),
            );
            environment.insert(
                "MURIARC_SERVER_IMAGE".to_owned(),
                state
                    .target_server_image
                    .clone()
                    .or_else(|| context.environment.get("MURIARC_SERVER_IMAGE").cloned())
                    .context("recovery Server image is missing")?,
            );
        }
        DeploymentProfile::Desktop => unreachable!(),
    }
    let repository = context.repository(restore_database).await?;
    if active_write_lease(repository.pool(), backup.source_generation_id)
        .await?
        .is_none()
    {
        repository
            .restore_source_write_lease(
                backup.source_generation_id,
                "muriarcctl-explicit-recovery",
                Duration::minutes(15),
            )
            .await?;
    }
    write_environment(&context.config.environment_file, &environment)?;
    write_environment(&context.config.activation_file, &activation)?;
    activate_staged_release(
        &state.source_release_path,
        &state.source_bundle,
        &context.config,
    )?;
    context.service_controller()?.start_read_only()?;
    wait_loopback_ready().await?;
    Ok(RestoreOperationResponse {
        backup_id: backup.backup_id,
        backup_artifact_digest: backup.artifact_digest,
        restored_generation_id: backup.source_generation_id,
        data_loss_confirmation_recorded: payload.confirm_data_loss,
    })
}

fn validate_target(
    context: &DriverContext,
    snapshot: &UpgradeSnapshot,
    target: &TargetEnvelope,
) -> Result<()> {
    target.manifest.validate().map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        target.metadata_expires_at > Utc::now()
            && target.metadata_versions.root > 0
            && target.metadata_versions.timestamp > 0
            && target.metadata_versions.snapshot > 0
            && target.metadata_versions.targets > 0
            && !target.target_name.trim().is_empty()
            && valid_digest(&target.target_digest)
            && target.target_length > 0
            && snapshot.target_application_version == target.manifest.application_version.as_str(),
        "verified target envelope is invalid"
    );
    let artifact_name = match context.profile() {
        DeploymentProfile::NativeSystem => "native-system",
        DeploymentProfile::ManagedCompose => "managed-compose",
        DeploymentProfile::Desktop => unreachable!(),
    };
    let artifact = target
        .manifest
        .artifacts
        .get(artifact_name)
        .context("target profile artifact is missing")?;
    anyhow::ensure!(
        artifact.digest.as_str() == target.target_digest
            && artifact.size_bytes == target.target_length,
        "target profile artifact differs from target envelope"
    );
    Ok(())
}

fn stage_target_artifact(
    context: &DriverContext,
    state: &DriverOperationState,
    target: &TargetEnvelope,
) -> Result<(PathBuf, VerifiedServerBundle, Option<String>)> {
    if let (Some(path), Some(bundle)) = (&state.target_release_path, &state.target_bundle) {
        let (_, observed) = verify_server_bundle(path, Some(&bundle.manifest_digest))?;
        anyhow::ensure!(&observed == bundle, "staged target bundle changed");
        return Ok((
            path.clone(),
            bundle.clone(),
            state.target_server_image.clone(),
        ));
    }
    let artifact = context.target_artifact_path()?;
    let (size, digest) = hash_file(&artifact)?;
    anyhow::ensure!(
        size == target.target_length && digest == target.target_digest,
        "target artifact changed before staging"
    );
    let unpack = context
        .operation_root(state.operation_id)
        .join("target-artifact");
    anyhow::ensure!(
        !unpack.exists() && !unpack.is_symlink(),
        "target staging exists"
    );
    fs::create_dir(&unpack)?;
    set_mode(&unpack, 0o700)?;
    secure_extract_tar_gz(&artifact, &unpack)?;
    let bundle_root = find_bundle_root(&unpack)?;
    let (manifest, verified) = verify_server_bundle(&bundle_root, None)?;
    anyhow::ensure!(
        manifest.profile == context.profile()
            && manifest.application_version == target.manifest.application_version,
        "inner Server bundle identity differs from signed target"
    );
    let release_path =
        match stage_verified_release(&bundle_root, &manifest, &context.config.paths.release_root) {
            Ok(path) => path,
            Err(DeliveryError::AlreadyInstalled(path)) => {
                let (_, existing) = verify_server_bundle(&path, Some(&verified.manifest_digest))?;
                anyhow::ensure!(existing == verified, "existing staged release differs");
                path
            }
            Err(error) => return Err(error.into()),
        };
    let target_server_image = if context.profile() == DeploymentProfile::ManagedCompose {
        Some(load_compose_images(&release_path)?)
    } else {
        None
    };
    Ok((release_path, verified, target_server_image))
}

fn load_compose_images(release: &Path) -> Result<String> {
    let lock_path = release.join("images/image-lock.json");
    require_regular_file(&lock_path, "image lock")?;
    let lock: ImageLock = serde_json::from_slice(&fs::read(lock_path)?)?;
    anyhow::ensure!(lock.format_version == 1, "image lock format is invalid");
    validate_digest_pinned_image(&lock.server_image)?;
    validate_digest_pinned_image(&lock.postgres_image)?;
    let server_archive = release.join("images/muriarc-server.docker.tar");
    let postgres_archive = release.join("images/postgres-17.docker.tar");
    anyhow::ensure!(
        hash_file(&server_archive)?.1 == lock.server_image_archive_digest
            && hash_file(&postgres_archive)?.1 == lock.postgres_image_archive_digest,
        "image archives differ from image lock"
    );
    for archive in [&server_archive, &postgres_archive] {
        safe_status(
            Command::new("/usr/bin/docker")
                .args(["image", "load", "--input"])
                .arg(archive),
        )?;
    }
    safe_status(Command::new("/usr/bin/docker").args(["image", "inspect", &lock.server_image]))?;
    safe_status(Command::new("/usr/bin/docker").args(["image", "inspect", &lock.postgres_image]))?;
    Ok(lock.server_image)
}

fn secure_extract_tar_gz(artifact: &Path, output: &Path) -> Result<()> {
    require_regular_file(artifact, "target artifact")?;
    require_real_directory(output, "target staging")?;
    let decoder = GzDecoder::new(fs::File::open(artifact)?);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        validate_relative_path(&path)?;
        let kind = entry.header().entry_type();
        anyhow::ensure!(
            kind.is_file() || kind.is_dir(),
            "target archive contains unsupported entry"
        );
        anyhow::ensure!(
            entry.unpack_in(output)?,
            "target archive entry escaped staging"
        );
    }
    Ok(())
}

fn find_bundle_root(root: &Path) -> Result<PathBuf> {
    let mut matches = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false).max_depth(3) {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "target tree contains symlink"
        );
        if metadata.is_file() && entry.file_name() == "bundle-manifest.json" {
            matches.push(
                entry
                    .path()
                    .parent()
                    .context("bundle manifest has no parent")?
                    .to_path_buf(),
            );
        }
    }
    anyhow::ensure!(
        matches.len() == 1,
        "target artifact must contain exactly one Server bundle"
    );
    Ok(matches.remove(0))
}

fn backup_prerequisites_ready(context: &DriverContext) -> bool {
    let service_control = match context.profile() {
        DeploymentProfile::NativeSystem => {
            require_executable(Path::new("/usr/bin/systemctl"), "backup prerequisite").is_ok()
        }
        DeploymentProfile::ManagedCompose => {
            require_executable(Path::new("/usr/bin/docker"), "backup prerequisite").is_ok()
        }
        DeploymentProfile::Desktop => false,
    };
    service_control
        && context.pg_dump_executable().is_ok()
        && context.pg_restore_executable().is_ok()
        && context.backup_recipient_file().is_ok()
        && context.backup_identity_file().is_ok()
        && context.age_executable().is_ok()
}

fn release_identity(manifest: &muriarc_core::ReleaseManifest) -> Result<ReleaseIdentity> {
    ReleaseIdentity::parse(
        manifest.application_version.to_string(),
        manifest.data_epoch.to_string(),
        manifest
            .backend_states
            .get(&BackendKind::Postgres)
            .context("Release Manifest has no PostgreSQL state")?
            .to_string(),
        manifest.gateway_contract_revision.to_string(),
    )
    .map_err(anyhow::Error::msg)
}

fn source_generation_from_snapshot(snapshot: &UpgradeSnapshot) -> ActiveGeneration {
    ActiveGeneration {
        generation_id: snapshot.source_generation_id,
        identity: snapshot.source_identity.clone(),
        backend: BackendKind::Postgres,
        first_write_at: None,
    }
}

fn backup_control_files(context: &DriverContext, operation_id: Uuid) -> Result<()> {
    let pairs = [
        (
            &context.config.environment_file,
            context.environment_backup_path(operation_id),
        ),
        (
            &context.config.activation_file,
            context.activation_backup_path(operation_id),
        ),
    ];
    for (source, target) in pairs {
        if target.exists() {
            require_regular_file(&target, "control-file recovery copy")?;
            continue;
        }
        require_regular_file(source, "control file")?;
        let bytes = fs::read(source)?;
        write_bytes_atomic(&target, &bytes, 0o600)?;
    }
    Ok(())
}

fn restore_control_files(context: &DriverContext, operation_id: Uuid) -> Result<()> {
    for (source, target) in [
        (
            context.environment_backup_path(operation_id),
            &context.config.environment_file,
        ),
        (
            context.activation_backup_path(operation_id),
            &context.config.activation_file,
        ),
    ] {
        require_regular_file(&source, "control-file recovery copy")?;
        write_bytes_atomic(target, &fs::read(source)?, 0o600)?;
    }
    Ok(())
}

fn stop_cloudflared_gate() -> Result<bool> {
    if !Path::new("/usr/bin/systemctl").is_file() {
        return Ok(false);
    }
    let active = Command::new("/usr/bin/systemctl")
        .args(["is-active", "--quiet", "cloudflared.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if active {
        safe_status(Command::new("/usr/bin/systemctl").args(["stop", "cloudflared.service"]))?;
    }
    Ok(active)
}

fn start_cloudflared_gate() -> Result<()> {
    safe_status(Command::new("/usr/bin/systemctl").args(["start", "cloudflared.service"]))
}

async fn wait_loopback_ready() -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(StdDuration::from_secs(2))
        .build()?;
    for _ in 0..90 {
        if client
            .get("http://127.0.0.1:8787/readyz")
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(StdDuration::from_secs(1)).await;
    }
    anyhow::bail!("activated Server did not become ready")
}

async fn active_write_lease(
    pool: &sqlx::PgPool,
    generation_id: Uuid,
) -> Result<Option<WriteLeaseEvidence>> {
    let row = sqlx::query(
        "SELECT lease.lease_id, lease.fencing_token, lease.expires_at
           FROM muriarc_deployment_state AS state
           JOIN muriarc_write_leases AS lease ON lease.lease_id = state.write_lease_id
          WHERE state.singleton = TRUE
            AND state.generation_id = $1
            AND lease.generation_id = $1
            AND lease.status = 'active'
            AND lease.expires_at > CURRENT_TIMESTAMP",
    )
    .bind(generation_id)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        Ok(WriteLeaseEvidence {
            generation_id,
            lease_id: row.try_get("lease_id")?,
            fencing_token: row.try_get("fencing_token")?,
            expires_at: row.try_get("expires_at")?,
        })
    })
    .transpose()
}

async fn restore_source_after_maintenance(
    context: &DriverContext,
    operation_id: Uuid,
    source_generation_id: Uuid,
    cloudflared_was_active: bool,
) -> Result<()> {
    let mut errors = Vec::new();
    match context.repository(context.active_database()).await {
        Ok(repository) => match active_write_lease(repository.pool(), source_generation_id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) =
                    freeze_generation(context, operation_id, source_generation_id).await
                {
                    errors.push(format!("source lease fencing failed: {error}"));
                }
                match active_write_lease(repository.pool(), source_generation_id).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if let Err(error) = repository
                            .restore_source_write_lease(
                                source_generation_id,
                                "muriarcctl-standalone-backup",
                                Duration::minutes(15),
                            )
                            .await
                        {
                            errors.push(format!("source Write Lease recovery failed: {error}"));
                        }
                    }
                    Err(error) => errors.push(format!(
                        "source Write Lease recovery recheck failed: {error}"
                    )),
                }
            }
            Err(error) => errors.push(format!("source Write Lease check failed: {error}")),
        },
        Err(error) => errors.push(format!("source repository recovery failed: {error}")),
    }

    let service_ready = match context.service_controller() {
        Ok(controller) => match controller.start_read_only() {
            Ok(()) => match wait_loopback_ready().await {
                Ok(()) => true,
                Err(error) => {
                    errors.push(format!("source readiness recovery failed: {error}"));
                    false
                }
            },
            Err(error) => {
                errors.push(format!("source service recovery failed: {error}"));
                false
            }
        },
        Err(error) => {
            errors.push(format!(
                "source service controller recovery failed: {error}"
            ));
            false
        }
    };
    if cloudflared_was_active
        && service_ready
        && let Err(error) = start_cloudflared_gate()
    {
        errors.push(format!("Cloudflare traffic gate recovery failed: {error}"));
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
    Ok(())
}

fn find_state_for_active_generation(
    context: &DriverContext,
    generation_id: Uuid,
) -> Result<DriverOperationState> {
    let root = context.state_root().join("physical-driver");
    require_real_directory(&root, "driver state root")?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(operation_id) = Uuid::parse_str(&name) else {
            continue;
        };
        if let Ok(state) = context.load_operation_state(operation_id)
            && state.candidate_generation_id == Some(generation_id)
            && state.switched
            && !state.recovered
        {
            return Ok(state);
        }
    }
    anyhow::bail!("active generation has no completed physical Driver state")
}

fn find_state_for_backup(context: &DriverContext, backup_id: Uuid) -> Result<DriverOperationState> {
    let root = context.state_root().join("physical-driver");
    require_real_directory(&root, "driver state root")?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(operation_id) = Uuid::parse_str(name) else {
            continue;
        };
        if let Ok(state) = context.load_operation_state(operation_id)
            && state
                .backup
                .as_ref()
                .is_some_and(|backup| backup.backup_id == backup_id)
        {
            return Ok(state);
        }
    }
    anyhow::bail!("recovery point has no physical Driver state")
}

fn copy_recovery_tree(source: &Path, target: &Path) -> Result<()> {
    require_real_directory(source, "recovery generation source")?;
    anyhow::ensure!(
        !target.exists() && !target.is_symlink(),
        "recovery generation target exists"
    );
    fs::create_dir_all(target)?;
    set_mode(target, 0o700)?;
    for entry in walkdir::WalkDir::new(source)
        .follow_links(false)
        .min_depth(1)
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        validate_relative_path(relative)?;
        let destination = target.join(relative);
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "recovery tree contains symlink"
        );
        if metadata.is_dir() {
            fs::create_dir(&destination)?;
            set_mode(&destination, 0o700)?;
        } else if metadata.is_file() {
            let mut input = fs::File::open(entry.path())?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut output = options.open(destination)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        } else {
            anyhow::bail!("recovery tree contains unsupported entry");
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    anyhow::ensure!(
        !path.as_os_str().is_empty() && !path.is_absolute(),
        "path is unsafe"
    );
    anyhow::ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "path is unsafe"
    );
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn parse_empty(value: Value) -> Result<()> {
    anyhow::ensure!(
        value.as_object().is_some_and(serde_json::Map::is_empty),
        "action payload must be empty"
    );
    Ok(())
}

fn serialize(value: impl serde::Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorResponse {
    ok: bool,
    operation_id: Uuid,
    source_generation_id: Uuid,
    candidate_generation_id: Uuid,
    target_identity: ReleaseIdentity,
    write_lease_absent: bool,
    generation_manifest: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_payload_and_target_digest_fail_closed() {
        assert!(parse_empty(json!({})).is_ok());
        assert!(parse_empty(json!({ "unexpected": true })).is_err());
        assert!(valid_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!valid_digest("sha256:short"));
    }
}

#[cfg(test)]
mod postgres_tests {
    use std::collections::BTreeMap;

    use anyhow::{Context as _, Result};
    use muriarc_core::{
        BackendKind, MigrationClass, MuriArcStore, ReleaseArtifact, ReleaseManifest,
    };
    use muriarc_store_postgres::PostgresStore;
    use muriarc_upgrade::{DeploymentProfile, UpgradePhase, UpgradeSnapshot, VerifiedRelease};
    use sqlx::{Connection as _, PgConnection};
    use tempfile::tempdir;

    use super::*;
    use crate::backup::{drop_isolated_database, recreate_database};

    fn test_target(source: &ActiveGeneration) -> Result<VerifiedRelease> {
        let digest = source.identity.backend_state_digest.clone();
        let manifest = ReleaseManifest {
            format_version: 1,
            application_version: source.identity.application_version.clone(),
            data_epoch: source.identity.data_epoch.clone(),
            gateway_contract_revision: source.identity.gateway_contract_revision.clone(),
            backend_states: BTreeMap::from([
                (BackendKind::Sqlite, digest.clone()),
                (BackendKind::Postgres, digest.clone()),
            ]),
            postgres_major: 17,
            bootstrap_protocol_revision: 1,
            controller_protocol_min: 1,
            controller_protocol_max: 1,
            migration_class: MigrationClass::M0,
            artifacts: BTreeMap::from([(
                "native-system".to_owned(),
                ReleaseArtifact {
                    media_type: "application/gzip".to_owned(),
                    digest: digest.clone(),
                    size_bytes: 1,
                },
            )]),
        };
        Ok(VerifiedRelease::from_verified_platform_artifact(
            manifest,
            "native-system",
            1,
            digest.to_string(),
            Utc::now() + Duration::hours(1),
        )?)
    }

    async fn database_exists(database_url: &str, database: &str) -> Result<bool> {
        let mut connection = PgConnection::connect(database_url).await?;
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
        )
        .bind(database)
        .fetch_one(&mut connection)
        .await?)
    }

    #[tokio::test]
    async fn physical_postgres_journal_freeze_and_first_write_guard_are_real() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping Physical Driver PostgreSQL integration: MURIARC_TEST_DATABASE_URL is not set"
            );
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "Physical Driver integration requires the disposable muriarc_test server"
        );
        let source_database = format!("muriarc_recovery_{}", Uuid::new_v4().simple());
        let candidate_database = format!("muriarc_candidate_{}", Uuid::new_v4().simple());
        let temporary = tempdir().expect("Physical Driver test root must be created");
        let context = DriverContext::for_test(&database_url, &source_database, temporary.path())
            .expect("Physical Driver test context must be valid");

        let outcome: Result<()> = async {
            recreate_database(&context, &source_database, None).await?;
            let store =
                PostgresStore::connect(&context.endpoint(&source_database)?.connection_url())
                    .await?;
            store.migrate().await?;
            let repository = context.repository(&source_database).await?;
            let source = repository.current_generation().await?;
            let target = test_target(&source)?;
            let mut snapshot = UpgradeSnapshot::new(
                Uuid::new_v4(),
                DeploymentProfile::NativeSystem,
                &source,
                &target,
            )?;
            snapshot.advance(UpgradePhase::LocksAcquired)?;
            create_operation(&context, &snapshot).await?;
            anyhow::ensure!(
                load_operation(&context, snapshot.operation_id).await? == snapshot,
                "physical operation create/load changed the snapshot"
            );
            snapshot.advance(UpgradePhase::PreflightPassed)?;
            save_operation(&context, &snapshot).await?;
            anyhow::ensure!(
                load_operation(&context, snapshot.operation_id).await? == snapshot,
                "physical operation save/load changed the snapshot"
            );

            sqlx::query(
                "UPDATE muriarc_write_leases AS lease
                    SET status = 'draining'
                   FROM muriarc_deployment_state AS state
                  WHERE state.singleton = TRUE
                    AND state.write_lease_id = lease.lease_id
                    AND state.generation_id = $1",
            )
            .bind(source.generation_id)
            .execute(repository.pool())
            .await?;
            let first =
                freeze_generation(&context, snapshot.operation_id, source.generation_id).await?;
            let repeated =
                freeze_generation(&context, snapshot.operation_id, source.generation_id).await?;
            anyhow::ensure!(
                first.revoked_lease_id == repeated.revoked_lease_id
                    && first.fencing_token == repeated.fencing_token,
                "repeated physical freeze returned different fencing evidence"
            );
            repository.pool().close().await;
            store.pool().close().await;

            recreate_database(&context, &candidate_database, Some(&source_database)).await?;
            let candidate_repository = context.repository(&candidate_database).await?;
            let candidate_id = Uuid::new_v4();
            candidate_repository
                .create_candidate_generation(
                    candidate_id,
                    &snapshot.target_data_epoch,
                    &snapshot.target_backend_state_digest,
                )
                .await?;
            candidate_repository
                .activate_candidate(&snapshot, candidate_id)
                .await?;
            candidate_repository
                .open_write_lease(candidate_id, "physical-driver-test", Duration::minutes(5))
                .await?;
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
                 VALUES ($1, 'Physical rollback guard', $2, $2, NULL, 1)",
            )
            .bind(Uuid::new_v4())
            .bind(now)
            .execute(candidate_repository.pool())
            .await?;
            anyhow::ensure!(
                candidate_repository
                    .first_write_at(candidate_id)
                    .await?
                    .is_some(),
                "Candidate first write was not recorded"
            );
            candidate_repository.pool().close().await;

            let mut state = context.load_operation_state(snapshot.operation_id)?;
            state.candidate_generation_id = Some(candidate_id);
            state.candidate_database = Some(candidate_database.clone());
            state.target_release_path = Some(temporary.path().join("candidate-release"));
            state.switched = true;
            state.updated_at = Utc::now();
            context.save_operation_state(&state)?;
            let error = recover_before_first_write(&context, &snapshot)
                .await
                .expect_err("physical rollback must fail after Candidate first write");
            anyhow::ensure!(
                error.to_string().contains("first write"),
                "physical rollback failed for an unexpected reason"
            );
            Ok(())
        }
        .await;

        let _ = drop_isolated_database(&context, &candidate_database).await;
        let _ = drop_isolated_database(&context, &source_database).await;
        let source_residual = database_exists(&database_url, &source_database)
            .await
            .context("Physical Driver source database residual check failed")
            .expect("Physical Driver source database residual check must run");
        let candidate_residual = database_exists(&database_url, &candidate_database)
            .await
            .context("Physical Driver Candidate database residual check failed")
            .expect("Physical Driver Candidate database residual check must run");
        let residual = source_residual || candidate_residual;
        assert!(
            !residual,
            "Physical Driver left an isolated database behind"
        );
        outcome.expect("Physical Driver PostgreSQL lifecycle must pass");
    }
}
