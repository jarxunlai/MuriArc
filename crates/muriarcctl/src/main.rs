mod cli;
mod formal;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::Arc,
};

use cli::{CommandResponse, CtlCommand, HELP, OutputFormat, ParsedCommand, parse_args};
use formal::{
    OperationSelection, PruneResponse, ReadOnlyVerificationResponse, RestoreOperationResponse,
    VerifiedBackupResponse, control_context, load_verified_release, physical_driver_client,
    valid_digest,
};
use muriarc_delivery::{
    CommandRunner, CommandSpec, DELIVERY_CONFIG_FORMAT, DeliveryCapabilities, DeliveryConfig,
    DeliveryError, DeliveryPaths, ProcessCommandRunner, ServerServiceController,
    VerifiedServerBundle, install_server_bundle, load_delivery_config, load_install_state,
    validate_compose_policy, validate_digest_pinned_image, verify_server_bundle,
};
use muriarc_upgrade::{
    ActiveGeneration, BackupEvidence, DeploymentProfile, HostUpgradeLock, RecoveryPointCatalog,
    UpgradeEngine, UpgradeError,
};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection};

const BUNDLE_ROOT_ENV: &str = "MURIARCCTL_BUNDLE_ROOT";
const TRUSTED_MANIFEST_ENV: &str = "MURIARCCTL_TRUSTED_BUNDLE_MANIFEST_DIGEST";
#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err((format, command, error)) => {
            emit(
                format,
                &CommandResponse {
                    ok: false,
                    command,
                    code: error.code(),
                    message: error.safe_detail(),
                    data: Value::Null,
                },
            );
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<ExitCode, (OutputFormat, &'static str, UpgradeError)> {
    let parsed = parse_args(env::args_os().skip(1))
        .map_err(|error| (OutputFormat::Human, "parse", error))?;
    dispatch(parsed).await
}

async fn dispatch(
    parsed: ParsedCommand,
) -> Result<ExitCode, (OutputFormat, &'static str, UpgradeError)> {
    let command_name = parsed.command.name();
    match parsed.command {
        CtlCommand::Help => {
            if parsed.output == OutputFormat::Json {
                emit(
                    parsed.output,
                    &CommandResponse {
                        ok: true,
                        command: command_name,
                        code: "ok",
                        message: "command help".to_owned(),
                        data: json!({ "usage": HELP }),
                    },
                );
            } else {
                print!("{HELP}");
            }
            Ok(ExitCode::SUCCESS)
        }
        CtlCommand::Install { profile } => {
            let data = install(profile)
                .map_err(|error| (parsed.output, command_name, delivery_error(error)))?;
            emit(
                parsed.output,
                &CommandResponse {
                    ok: true,
                    command: command_name,
                    code: "ok",
                    message: "signed Server bundle installed; configure the root-only environment and activation files before starting the service".to_owned(),
                    data,
                },
            );
            Ok(ExitCode::SUCCESS)
        }
        CtlCommand::Status => {
            let data =
                status().map_err(|error| (parsed.output, command_name, delivery_error(error)))?;
            emit(
                parsed.output,
                &CommandResponse {
                    ok: true,
                    command: command_name,
                    code: "ok",
                    message: "local control-plane status inspected".to_owned(),
                    data,
                },
            );
            Ok(ExitCode::SUCCESS)
        }
        CtlCommand::Doctor => {
            let report = doctor()
                .await
                .map_err(|error| (parsed.output, command_name, delivery_error(error)))?;
            let ready = report
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            emit(
                parsed.output,
                &CommandResponse {
                    ok: ready,
                    command: command_name,
                    code: if ready { "ok" } else { "prerequisite_missing" },
                    message: if ready {
                        "deployment and upgrade prerequisites are present".to_owned()
                    } else {
                        "one or more deployment or upgrade prerequisites are missing".to_owned()
                    },
                    data: report,
                },
            );
            Ok(if ready {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        CtlCommand::UpdateCheck => {
            let config = load_delivery_config(&state_root())
                .map_err(|error| (parsed.output, command_name, delivery_error(error)))?;
            let target = load_verified_release(config.profile, None)
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "signed update metadata and final target artifact verified",
                json!({
                    "applicationVersion": target.manifest.application_version,
                    "dataEpoch": target.manifest.data_epoch,
                    "targetName": target.target_name,
                    "targetDigest": target.target_digest,
                    "targetLength": target.target_length,
                    "metadataVersions": target.metadata_versions,
                    "metadataExpiresAt": target.metadata_expires_at,
                }),
            )
        }
        CtlCommand::Upgrade { to } => {
            require_doctor_ready()
                .await
                .map_err(|error| (parsed.output, command_name, error))?;
            let (config, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let target = load_verified_release(config.profile, to.as_deref())
                .map_err(|error| (parsed.output, command_name, error))?;
            let engine = UpgradeEngine::new(Arc::new(driver), state_root());
            let snapshot = engine
                .run(target)
                .await
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "upgrade completed through the shared state machine and physical Driver",
                snapshot,
            )
        }
        CtlCommand::BackupCreate => {
            require_doctor_ready()
                .await
                .map_err(|error| (parsed.output, command_name, error))?;
            let (_, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let generation: ActiveGeneration = driver
                .invoke("current_generation", json!({}))
                .map_err(|error| (parsed.output, command_name, error))?;
            let backup: BackupEvidence = driver
                .invoke(
                    "standalone_backup_create",
                    json!({ "source_generation": generation }),
                )
                .map_err(|error| (parsed.output, command_name, error))?;
            backup
                .validate(generation.generation_id)
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "complete recovery-set backup created; run backup verify before relying on it",
                backup,
            )
        }
        CtlCommand::BackupVerify => {
            require_doctor_ready()
                .await
                .map_err(|error| (parsed.output, command_name, error))?;
            let (_, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let verified: VerifiedBackupResponse = driver
                .invoke("standalone_backup_verify", json!({}))
                .map_err(|error| (parsed.output, command_name, error))?;
            verified
                .backup
                .validate(verified.backup.source_generation_id)
                .and_then(|()| verified.restore.validate(&verified.backup))
                .map_err(|error| (parsed.output, command_name, error))?;
            let root = state_root();
            let mut catalog = RecoveryPointCatalog::load(&root)
                .map_err(|error| (parsed.output, command_name, error))?;
            catalog
                .register_verified(verified.backup.clone(), verified.restore.clone())
                .and_then(|()| catalog.save_atomic(&root))
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "backup was actually restored in isolation and registered as a recovery point",
                verified,
            )
        }
        CtlCommand::VerifyReadOnly => {
            let (_, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let report: ReadOnlyVerificationResponse = driver
                .invoke("verify_read_only", json!({}))
                .map_err(|error| (parsed.output, command_name, error))?;
            report
                .verification
                .validate()
                .map_err(|error| (parsed.output, command_name, error))?;
            if report.state_digest_before != report.state_digest_after
                || !valid_digest(&report.state_digest_before)
            {
                return Err((
                    parsed.output,
                    command_name,
                    UpgradeError::Prerequisite {
                        message: "read-only verification changed persistent state".to_owned(),
                    },
                ));
            }
            success(
                parsed.output,
                command_name,
                "all read-only verification layers passed without persistent side effects",
                report,
            )
        }
        CtlCommand::RecoveryResume { operation_id } => {
            let (config, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let operation_id = match operation_id {
                Some(value) => value,
                None => {
                    driver
                        .invoke::<OperationSelection>("latest_resumable_operation", json!({}))
                        .map_err(|error| (parsed.output, command_name, error))?
                        .operation_id
                }
            };
            let target = load_verified_release(config.profile, None)
                .map_err(|error| (parsed.output, command_name, error))?;
            let engine = UpgradeEngine::new(Arc::new(driver), state_root());
            let snapshot = engine
                .resume(operation_id, target)
                .await
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "persisted upgrade operation resumed through the physical Driver",
                snapshot,
            )
        }
        CtlCommand::RecoveryRestore {
            backup_id,
            confirm_data_loss,
        } => {
            let (_, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let catalog = RecoveryPointCatalog::load(&state_root())
                .map_err(|error| (parsed.output, command_name, error))?;
            let backup_id = backup_id
                .or(catalog.last_verified_backup_id)
                .ok_or_else(|| {
                    (
                        parsed.output,
                        command_name,
                        UpgradeError::Prerequisite {
                            message: "no verified recovery point is available".to_owned(),
                        },
                    )
                })?;
            let point = catalog.points.get(&backup_id).ok_or_else(|| {
                (
                    parsed.output,
                    command_name,
                    UpgradeError::Prerequisite {
                        message: "requested verified recovery point does not exist".to_owned(),
                    },
                )
            })?;
            let restored: RestoreOperationResponse = driver
                .invoke(
                    "recovery_restore",
                    json!({
                        "recovery_point": point,
                        "confirm_data_loss": confirm_data_loss,
                    }),
                )
                .map_err(|error| (parsed.output, command_name, error))?;
            if restored.backup_id != backup_id
                || restored.backup_artifact_digest != point.backup.artifact_digest
                || restored.restored_generation_id.is_nil()
                || (confirm_data_loss && !restored.data_loss_confirmation_recorded)
            {
                return Err((
                    parsed.output,
                    command_name,
                    UpgradeError::Prerequisite {
                        message: "physical Driver returned mismatched recovery evidence".to_owned(),
                    },
                ));
            }
            success(
                parsed.output,
                command_name,
                "verified recovery point restored by the physical Driver",
                restored,
            )
        }
        CtlCommand::RecoveryPrune { backup_id } => {
            let (_, driver) =
                control_context().map_err(|error| (parsed.output, command_name, error))?;
            let root = state_root();
            let mut catalog = RecoveryPointCatalog::load(&root)
                .map_err(|error| (parsed.output, command_name, error))?;
            let point = catalog
                .require_prunable(backup_id)
                .map_err(|error| (parsed.output, command_name, error))?;
            let pruned: PruneResponse = driver
                .invoke("recovery_prune", json!({ "recovery_point": point }))
                .map_err(|error| (parsed.output, command_name, error))?;
            if pruned.backup_id != backup_id || !pruned.artifact_deleted {
                return Err((
                    parsed.output,
                    command_name,
                    UpgradeError::Prerequisite {
                        message: "physical Driver did not prove exact recovery artifact deletion"
                            .to_owned(),
                    },
                ));
            }
            catalog
                .commit_pruned(backup_id)
                .and_then(|()| catalog.save_atomic(&root))
                .map_err(|error| (parsed.output, command_name, error))?;
            success(
                parsed.output,
                command_name,
                "older verified recovery point pruned explicitly",
                pruned,
            )
        }
    }
}

fn success<T: Serialize>(
    format: OutputFormat,
    command: &'static str,
    message: &str,
    data: T,
) -> Result<ExitCode, (OutputFormat, &'static str, UpgradeError)> {
    emit(
        format,
        &CommandResponse {
            ok: true,
            command,
            code: "ok",
            message: message.to_owned(),
            data,
        },
    );
    Ok(ExitCode::SUCCESS)
}

async fn require_doctor_ready() -> Result<(), UpgradeError> {
    let report = doctor().await.map_err(delivery_error)?;
    if report.get("ready").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(UpgradeError::Prerequisite {
            message: "muriarcctl doctor did not prove every physical upgrade capability".to_owned(),
        })
    }
}

fn install(profile: DeploymentProfile) -> Result<Value, DeliveryError> {
    let bundle_root = required_absolute_path(BUNDLE_ROOT_ENV)?;
    let trusted_manifest = required_env(TRUSTED_MANIFEST_ENV)?;
    let config = install_config(profile)?;
    preflight_install(&bundle_root, &trusted_manifest, &config)?;
    let (receipt, verified) = install_server_bundle(&bundle_root, &trusted_manifest, &config)?;
    match profile {
        DeploymentProfile::NativeSystem => register_native_system(&receipt.release_path)?,
        DeploymentProfile::ManagedCompose => {
            install_managed_examples(&receipt.release_path, &config)?
        }
        DeploymentProfile::Desktop => {
            return Err(DeliveryError::InvalidPolicy(
                "Desktop installation must use the signed Tauri updater".to_owned(),
            ));
        }
    }
    Ok(json!({
        "profile": profile,
        "applicationVersion": receipt.application_version,
        "releasePath": receipt.release_path,
        "manifestDigest": verified.manifest_digest,
        "contentDigest": verified.content_digest,
        "serviceStarted": false,
        "next": "populate the root-only environment and activation files, then run muriarcctl doctor",
    }))
}

fn preflight_install(
    bundle_root: &Path,
    trusted_manifest: &str,
    config: &DeliveryConfig,
) -> Result<VerifiedServerBundle, DeliveryError> {
    let (manifest, verified) = verify_server_bundle(bundle_root, Some(trusted_manifest))?;
    if manifest.profile != config.profile {
        return Err(DeliveryError::InvalidPolicy(
            "bundle profile differs from the requested install profile".to_owned(),
        ));
    }
    match config.profile {
        DeploymentProfile::NativeSystem => {
            require_root()?;
            for program in [
                "/usr/bin/systemctl",
                "/usr/bin/systemd-sysusers",
                "/usr/bin/systemd-tmpfiles",
            ] {
                require_program(program)?;
            }
        }
        DeploymentProfile::ManagedCompose => {
            require_program("/usr/bin/docker")?;
            let compose = bundle_root.join("deploy/compose.yaml");
            validate_compose_policy(&fs::read_to_string(compose).map_err(io)?)?;
            run_program("/usr/bin/docker", &["compose", "version"])?;
        }
        DeploymentProfile::Desktop => unreachable!("Desktop rejected by install_config"),
    }
    Ok(verified)
}

fn install_config(profile: DeploymentProfile) -> Result<DeliveryConfig, DeliveryError> {
    let paths = match profile {
        DeploymentProfile::NativeSystem => DeliveryPaths::native_system(),
        DeploymentProfile::ManagedCompose => {
            DeliveryPaths::managed_compose(required_absolute_path("MURIARCCTL_INSTALL_ROOT")?)
        }
        DeploymentProfile::Desktop => {
            return Err(DeliveryError::InvalidPolicy(
                "Desktop uses its dedicated updater Driver".to_owned(),
            ));
        }
    };
    let config = DeliveryConfig {
        format_version: DELIVERY_CONFIG_FORMAT,
        profile,
        environment_file: match profile {
            DeploymentProfile::NativeSystem => "/etc/muriarc/server.env".into(),
            DeploymentProfile::ManagedCompose => paths.config_root.join("server.env"),
            DeploymentProfile::Desktop => unreachable!(),
        },
        activation_file: paths.control_root.join("active.env"),
        compose_project: (profile == DeploymentProfile::ManagedCompose).then(|| {
            env::var("MURIARCCTL_COMPOSE_PROJECT").unwrap_or_else(|_| "muriarc".to_owned())
        }),
        compose_file: (profile == DeploymentProfile::ManagedCompose)
            .then(|| paths.current_release.join("deploy/compose.yaml")),
        paths,
        service_user: "muriarc".to_owned(),
        loopback_origin: "http://127.0.0.1:8787".to_owned(),
    };
    config.validate()?;
    Ok(config)
}

fn register_native_system(release: &Path) -> Result<(), DeliveryError> {
    require_root()?;
    let files = [
        (
            release.join("deploy/muriarc.service"),
            PathBuf::from("/etc/systemd/system/muriarc.service"),
            0o644,
        ),
        (
            release.join("deploy/muriarc.sysusers"),
            PathBuf::from("/usr/lib/sysusers.d/muriarc.conf"),
            0o644,
        ),
        (
            release.join("deploy/muriarc.tmpfiles"),
            PathBuf::from("/usr/lib/tmpfiles.d/muriarc.conf"),
            0o644,
        ),
    ];
    for (source, target, mode) in files {
        copy_atomic(&source, &target, mode, true)?;
    }
    copy_atomic(
        &release.join("deploy/server.env.example"),
        Path::new("/etc/muriarc/server.env.example"),
        0o640,
        false,
    )?;
    copy_atomic(
        &release.join("deploy/active.env.example"),
        Path::new("/var/lib/muriarc/control/active.env.example"),
        0o600,
        false,
    )?;
    run_program(
        "/usr/bin/systemd-sysusers",
        &["/usr/lib/sysusers.d/muriarc.conf"],
    )?;
    run_program(
        "/usr/bin/systemd-tmpfiles",
        &["--create", "/usr/lib/tmpfiles.d/muriarc.conf"],
    )?;
    run_program("/usr/bin/systemctl", &["daemon-reload"])?;
    run_program("/usr/bin/systemctl", &["enable", "muriarc.service"])
}

fn install_managed_examples(release: &Path, config: &DeliveryConfig) -> Result<(), DeliveryError> {
    fs::create_dir_all(&config.paths.config_root).map_err(io)?;
    fs::create_dir_all(&config.paths.control_root).map_err(io)?;
    copy_atomic(
        &release.join("deploy/.env.example"),
        &config.paths.config_root.join("server.env.example"),
        0o600,
        false,
    )?;
    copy_atomic(
        &release.join("deploy/active.env.example"),
        &config.paths.control_root.join("active.env.example"),
        0o600,
        false,
    )
}

fn status() -> Result<Value, DeliveryError> {
    let root = state_root();
    let lock = HostUpgradeLock::inspect(&root)
        .map_err(|error| DeliveryError::InvalidPolicy(error.safe_detail()))?;
    let config_path = root.join("delivery.json");
    let config = match fs::symlink_metadata(&config_path) {
        Ok(_) => Some(load_delivery_config(&root)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io(error)),
    };
    let (receipt, service_active, recovery_points) = if let Some(config) = &config {
        let receipt = load_install_state(config)?;
        let service_active = ServerServiceController::new(config.clone(), ProcessCommandRunner)
            .and_then(|controller| controller.is_active())
            .unwrap_or(false);
        let catalog = RecoveryPointCatalog::load(&root)
            .map_err(|error| DeliveryError::InvalidPolicy(error.safe_detail()))?;
        (receipt, service_active, catalog.points.len())
    } else {
        (None, false, 0)
    };
    Ok(json!({
        "configured": config.is_some(),
        "profile": config.as_ref().map(|value| value.profile),
        "stateRoot": path_class(&root),
        "receipt": receipt,
        "serviceActive": service_active,
        "upgradeLock": lock,
        "verifiedRecoveryPointCount": recovery_points,
    }))
}

async fn doctor() -> Result<Value, DeliveryError> {
    let root = state_root();
    let config = load_delivery_config(&root)?;
    let receipt = load_install_state(&config)?
        .ok_or_else(|| DeliveryError::Prerequisite("install receipt is missing".to_owned()))?;
    let (_, verified) =
        verify_server_bundle(&receipt.release_path, Some(&receipt.manifest_digest))?;
    let current_matches =
        fs::read_link(&config.paths.current_release).is_ok_and(|path| path == receipt.release_path);
    let environment = read_env_file(&config.environment_file).ok();
    let activation = read_env_file(&config.activation_file).ok();
    let environment_ready =
        environment
            .as_ref()
            .zip(activation.as_ref())
            .is_some_and(|(environment, activation)| {
                required_environment_present(config.profile, environment, activation)
            });
    let service_control = match config.profile {
        DeploymentProfile::NativeSystem => program_available("/usr/bin/systemctl"),
        DeploymentProfile::ManagedCompose => program_available("/usr/bin/docker"),
        DeploymentProfile::Desktop => false,
    };
    let physical_driver = physical_driver_client(&config).is_ok();
    let service_active = if service_control && environment_ready {
        ServerServiceController::new(config.clone(), ProcessCommandRunner)
            .and_then(|controller| controller.is_active())
            .unwrap_or(false)
    } else {
        false
    };
    let postgres_major =
        probe_postgres_major(&config, environment.as_ref(), activation.as_ref()).await;
    let executor = receipt.release_path.join("bin/muriarc-upgrade-executor");
    let verifier = receipt.release_path.join("bin/muriarc-verifier");
    let isolated_storage = directory_available(&config.paths.data_root)
        && directory_available(&config.paths.control_root);
    let postgres_client_tools = [
        (
            "MURIARCCTL_PG_DUMP_EXECUTABLE",
            "/usr/lib/postgresql/17/bin/pg_dump",
        ),
        (
            "MURIARCCTL_PG_RESTORE_EXECUTABLE",
            "/usr/lib/postgresql/17/bin/pg_restore",
        ),
    ]
    .iter()
    .all(|(name, default)| configured_program_available(name, Path::new(default), 17));
    let age_executable =
        configured_executable("MURIARCCTL_AGE_EXECUTABLE", Path::new("/usr/bin/age"));
    let backup_recipient = configured_control_file("MURIARCCTL_BACKUP_RECIPIENT_FILE", false);
    let backup_identity = configured_control_file("MURIARCCTL_BACKUP_IDENTITY_FILE", true);
    let recovery_material_ready =
        age_executable && backup_recipient && backup_identity && postgres_client_tools;
    let backup_restore = recovery_material_ready
        && match config.profile {
            DeploymentProfile::NativeSystem => true,
            DeploymentProfile::ManagedCompose => {
                service_control && service_active && postgres_major == Some(17)
            }
            DeploymentProfile::Desktop => false,
        };
    let postgres_admin_url =
        env::var("MURIARCCTL_POSTGRES_ADMIN_URL").is_ok_and(|value| !value.trim().is_empty());
    let candidate_database = match config.profile {
        DeploymentProfile::NativeSystem => postgres_admin_url,
        DeploymentProfile::ManagedCompose => service_active && postgres_major == Some(17),
        DeploymentProfile::Desktop => false,
    };
    let mut unavailable = BTreeSet::new();
    if !current_matches {
        unavailable.insert("current release pointer differs from receipt".to_owned());
    }
    if !environment_ready {
        unavailable.insert("root-only environment or activation file is missing".to_owned());
    }
    if !service_active {
        unavailable.insert("service is not running".to_owned());
    }
    if !physical_driver {
        unavailable.insert("physical upgrade Driver is unavailable".to_owned());
    }
    if postgres_major != Some(17) {
        unavailable.insert("PostgreSQL 17 could not be verified".to_owned());
    }
    if !postgres_client_tools {
        unavailable.insert("PostgreSQL dump/restore clients are unavailable".to_owned());
    }
    if !age_executable {
        unavailable.insert("age backup encryption executable is unavailable".to_owned());
    }
    if !backup_recipient {
        unavailable.insert("backup encryption recipient file is unavailable".to_owned());
    }
    if !backup_identity {
        unavailable.insert("private backup decryption identity file is unavailable".to_owned());
    }
    if !backup_restore {
        unavailable.insert("physical backup/restore prerequisites are incomplete".to_owned());
    }
    if config.profile == DeploymentProfile::NativeSystem && !postgres_admin_url {
        unavailable.insert("Native PostgreSQL admin URL is unavailable".to_owned());
    }
    if !candidate_database {
        unavailable.insert("isolated Candidate database capability is unavailable".to_owned());
    }
    if !isolated_storage {
        unavailable.insert("isolated generation storage is unavailable".to_owned());
    }
    let capabilities = DeliveryCapabilities {
        service_control,
        physical_driver,
        postgres_major,
        backup_restore,
        isolated_candidate_database: candidate_database,
        isolated_candidate_storage: isolated_storage,
        ddl_executor: executable_file(&executor),
        verifier: executable_file(&verifier),
        bundle_signature_verified: verified.manifest_digest == receipt.manifest_digest,
        unavailable_reasons: unavailable,
    };
    let upgrade_ready = capabilities.require_upgrade_ready().is_ok();
    Ok(json!({
        "ready": upgrade_ready,
        "profile": config.profile,
        "applicationVersion": receipt.application_version,
        "bundle": verified,
        "currentReleaseMatchesReceipt": current_matches,
        "environmentReady": environment_ready,
        "serviceActive": service_active,
        "physicalPrerequisites": {
            "postgresClientTools": postgres_client_tools,
            "ageExecutable": age_executable,
            "backupRecipient": backup_recipient,
            "backupIdentity": backup_identity,
            "nativePostgresAdminUrl": config.profile != DeploymentProfile::NativeSystem || postgres_admin_url,
        },
        "capabilities": capabilities,
        "note": "doctor proves prerequisites only; every upgrade still performs a fresh backup and an actual isolated restore",
    }))
}

async fn probe_postgres_major(
    config: &DeliveryConfig,
    environment: Option<&BTreeMap<String, String>>,
    activation: Option<&BTreeMap<String, String>>,
) -> Option<u16> {
    match config.profile {
        DeploymentProfile::NativeSystem => {
            let database_url = activation?.get("MURIARC_DATABASE_URL")?;
            let mut connection = PgConnection::connect(database_url).await.ok()?;
            let version: String = sqlx::query_scalar("SHOW server_version_num")
                .fetch_one(&mut connection)
                .await
                .ok()?;
            version
                .parse::<u32>()
                .ok()
                .map(|value| (value / 10_000) as u16)
        }
        DeploymentProfile::ManagedCompose => {
            let user = environment?.get("MURIARC_POSTGRES_USER")?;
            let database = activation?.get("MURIARC_POSTGRES_DB")?;
            let output = Command::new("/usr/bin/docker")
                .args(compose_prefix(config))
                .args([
                    "exec",
                    "-T",
                    "db",
                    "psql",
                    "-U",
                    user,
                    "-d",
                    database,
                    "-Atqc",
                    "SHOW server_version_num",
                ])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let version = String::from_utf8(output.stdout).ok()?;
            version
                .trim()
                .parse::<u32>()
                .ok()
                .map(|value| (value / 10_000) as u16)
        }
        DeploymentProfile::Desktop => None,
    }
}

fn compose_prefix(config: &DeliveryConfig) -> Vec<String> {
    vec![
        "compose".to_owned(),
        "--project-name".to_owned(),
        config.compose_project.clone().unwrap_or_default(),
        "--env-file".to_owned(),
        config.environment_file.display().to_string(),
        "--env-file".to_owned(),
        config.activation_file.display().to_string(),
        "--file".to_owned(),
        config
            .compose_file
            .as_deref()
            .unwrap_or(Path::new("/invalid"))
            .display()
            .to_string(),
    ]
}

fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>, DeliveryError> {
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidPolicy(
            "environment input must be a regular non-symlink file".to_owned(),
        ));
    }
    let mut values = BTreeMap::new();
    for (line_number, line) in fs::read_to_string(path).map_err(io)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(|| {
            DeliveryError::InvalidPolicy(format!(
                "environment file line {} is malformed",
                line_number + 1
            ))
        })?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(DeliveryError::InvalidPolicy(format!(
                "environment file line {} has an invalid name",
                line_number + 1
            )));
        }
        values.insert(name.to_owned(), value.to_owned());
    }
    if values.is_empty() {
        return Err(DeliveryError::InvalidPolicy(
            "environment file contains no values".to_owned(),
        ));
    }
    if let Some(image) = values
        .get("MURIARC_SERVER_IMAGE")
        .filter(|value| !value.is_empty())
    {
        validate_digest_pinned_image(image)?;
    }
    if let Some(image) = values
        .get("MURIARC_POSTGRES_IMAGE")
        .filter(|value| !value.is_empty())
    {
        validate_digest_pinned_image(image)?;
    }
    Ok(values)
}

fn required_environment_present(
    profile: DeploymentProfile,
    environment: &BTreeMap<String, String>,
    activation: &BTreeMap<String, String>,
) -> bool {
    let base = [
        "MURIARC_LAB_ID",
        "MURIARC_LAB_NAME",
        "MURIARC_ROOT_USER_ID",
        "MURIARC_ROOT_USER_EMAIL",
        "MURIARC_ROOT_USER_NAME",
        "MURIARC_ROOT_PASSWORD",
    ];
    let profile_keys: &[&str] = match profile {
        DeploymentProfile::NativeSystem => &[],
        DeploymentProfile::ManagedCompose => &[
            "MURIARC_SERVER_IMAGE",
            "MURIARC_POSTGRES_IMAGE",
            "MURIARC_POSTGRES_USER",
            "MURIARC_POSTGRES_PASSWORD",
        ],
        DeploymentProfile::Desktop => return false,
    };
    let activation_keys: &[&str] = match profile {
        DeploymentProfile::NativeSystem => &[
            "MURIARC_ACTIVE_GENERATION",
            "MURIARC_DATABASE_URL",
            "MURIARC_DATA_ROOT",
            "MURIARC_ATTACHMENT_ROOT",
            "MURIARC_AI_MASTER_KEY_FILE",
            "MURIARC_ACTIVATION_MODE",
        ],
        DeploymentProfile::ManagedCompose => &[
            "MURIARC_POSTGRES_DB",
            "MURIARC_ACTIVE_GENERATION",
            "MURIARC_ACTIVATION_MODE",
        ],
        DeploymentProfile::Desktop => return false,
    };
    base.into_iter()
        .chain(profile_keys.iter().copied())
        .all(|key| {
            environment
                .get(key)
                .is_some_and(|value| !value.trim().is_empty())
        })
        && activation_keys.iter().all(|key| {
            activation
                .get(*key)
                .is_some_and(|value| !value.trim().is_empty())
        })
}

fn copy_atomic(
    source: &Path,
    target: &Path,
    mode: u32,
    replace: bool,
) -> Result<(), DeliveryError> {
    let source_metadata = fs::symlink_metadata(source).map_err(io)?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidBundle(
            "system install source must be a regular non-symlink file".to_owned(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if metadata.file_type().is_symlink() {
            return Err(DeliveryError::InvalidPolicy(
                "refusing to replace a symlinked system file".to_owned(),
            ));
        }
        if !replace {
            return Ok(());
        }
    }
    let parent = target
        .parent()
        .ok_or_else(|| DeliveryError::InvalidPolicy("system file has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = parent.join(format!(".muriarc-install-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut output = options.open(&temporary).map_err(io)?;
    let bytes = fs::read(source).map_err(io)?;
    output.write_all(&bytes).map_err(io)?;
    output.sync_all().map_err(io)?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(io(error));
    }
    Ok(())
}

fn run_program(program: &str, args: &[&str]) -> Result<(), DeliveryError> {
    let runner = ProcessCommandRunner;
    let command = CommandSpec::new(program, args.iter().map(std::ffi::OsString::from));
    let outcome = runner.run(&command)?;
    if outcome.success {
        Ok(())
    } else {
        Err(DeliveryError::Command(format!(
            "{} exited with {:?}",
            Path::new(program)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("program"),
            outcome.exit_code
        )))
    }
}

fn required_absolute_path(name: &'static str) -> Result<PathBuf, DeliveryError> {
    let path = PathBuf::from(required_env(name)?);
    if !path.is_absolute() {
        return Err(DeliveryError::InvalidPolicy(format!(
            "{name} must be an absolute path"
        )));
    }
    Ok(path)
}

fn required_env(name: &'static str) -> Result<String, DeliveryError> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(DeliveryError::Prerequisite(format!(
            "required environment variable {name} is missing"
        ))),
    }
}

fn state_root() -> PathBuf {
    env::var_os("MURIARCCTL_STATE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("MURIARCCTL_INSTALL_ROOT")
                .map(PathBuf::from)
                .map(|root| root.join("control"))
        })
        .unwrap_or_else(|| PathBuf::from("/var/lib/muriarc/control"))
}

fn require_root() -> Result<(), DeliveryError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata("/proc/self").map_err(io)?.uid() == 0 {
            return Ok(());
        }
        Err(DeliveryError::Prerequisite(
            "native-system installation must run as root".to_owned(),
        ))
    }
    #[cfg(not(unix))]
    {
        Err(DeliveryError::InvalidPolicy(
            "native-system installation requires Linux".to_owned(),
        ))
    }
}

fn require_program(path: &str) -> Result<(), DeliveryError> {
    if program_available(path) {
        Ok(())
    } else {
        Err(DeliveryError::Prerequisite(format!(
            "required executable {path} is unavailable"
        )))
    }
}

fn program_available(path: &str) -> bool {
    executable_file(Path::new(path))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn configured_program_available(name: &str, default: &Path, minimum_major: u16) -> bool {
    let path = env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.to_path_buf());
    path.is_absolute()
        && strict_executable_file(&path)
        && executable_major_version(&path).is_some_and(|major| major >= minimum_major)
}

fn configured_executable(name: &str, default: &Path) -> bool {
    let path = env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.to_path_buf());
    path.is_absolute() && strict_executable_file(&path)
}

fn executable_major_version(path: &Path) -> Option<u16> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .find_map(|part| part.split('.').next()?.parse::<u16>().ok())
}

fn strict_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn configured_control_file(name: &str, owner_only: bool) -> bool {
    let Some(path) = env::var_os(name).map(PathBuf::from) else {
        return false;
    };
    control_file_available(&path, owner_only)
}

fn control_file_available(path: &Path, owner_only: bool) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return false;
        }
    }
    true
}

fn directory_available(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn path_class(path: &Path) -> &'static str {
    if path.is_dir() {
        "directory"
    } else if path.exists() {
        "invalid"
    } else {
        "missing"
    }
}

fn delivery_error(error: DeliveryError) -> UpgradeError {
    UpgradeError::Prerequisite {
        message: error.to_string(),
    }
}

fn io(error: std::io::Error) -> DeliveryError {
    DeliveryError::Io(error.to_string())
}

fn emit<T: Serialize>(format: OutputFormat, response: &T) {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(response).expect("command response must be serializable")
        ),
        OutputFormat::Human => {
            let value =
                serde_json::to_value(response).expect("command response must be serializable");
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let code = value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            println!("{} [{code}] {message}", if ok { "OK" } else { "ERROR" });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn strict_executable_rejects_symlink_and_non_version_output() {
        let shell = Path::new("/bin/sh");
        assert!(!strict_executable_file(shell));
        assert_eq!(executable_major_version(shell), None);
    }

    #[test]
    fn doctor_environment_contract_requires_active_generation_and_signed_images() {
        let environment = values(&[
            ("MURIARC_LAB_ID", "lab"),
            ("MURIARC_LAB_NAME", "Lab"),
            ("MURIARC_ROOT_USER_ID", "root"),
            ("MURIARC_ROOT_USER_EMAIL", "root@example.org"),
            ("MURIARC_ROOT_USER_NAME", "Root"),
            ("MURIARC_ROOT_PASSWORD", "not-a-real-secret"),
            ("MURIARC_SERVER_IMAGE", "ghcr.io/example/server@sha256:fake"),
            (
                "MURIARC_POSTGRES_IMAGE",
                "ghcr.io/example/postgres@sha256:fake",
            ),
            ("MURIARC_POSTGRES_USER", "muriarc"),
            ("MURIARC_POSTGRES_PASSWORD", "not-a-real-secret"),
        ]);
        let activation = values(&[
            ("MURIARC_POSTGRES_DB", "muriarc"),
            ("MURIARC_ACTIVE_GENERATION", "generation"),
            ("MURIARC_ACTIVATION_MODE", "read-write"),
        ]);
        assert!(required_environment_present(
            DeploymentProfile::ManagedCompose,
            &environment,
            &activation
        ));
        let mut missing = activation;
        missing.remove("MURIARC_ACTIVE_GENERATION");
        assert!(!required_environment_present(
            DeploymentProfile::ManagedCompose,
            &environment,
            &missing
        ));
    }
}
