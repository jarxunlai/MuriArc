use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context as _, Result};
use chrono::Utc;
use muriarc_upgrade::{
    ActiveGeneration, BackupEvidence, DeploymentProfile, RecoveryComponent, RestoreEvidence,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    context::{
        DatabaseEndpoint, DriverContext, require_database_name, require_real_directory,
        require_regular_file, safe_status, set_mode, write_bytes_atomic,
    },
    model::{DriverOperationState, RECOVERY_SET_FORMAT, RecoverySetEntry, RecoverySetManifest},
};

pub(crate) async fn create_backup(
    context: &DriverContext,
    state: &mut DriverOperationState,
    source: &ActiveGeneration,
) -> Result<BackupEvidence> {
    if let Some(existing) = &state.backup {
        verify_existing_artifact(context, existing, state.recovery_set_digest.as_deref())?;
        return Ok(existing.clone());
    }
    anyhow::ensure!(
        source.generation_id == state.source_generation_id,
        "backup source generation differs from operation"
    );
    let backup_id = Uuid::new_v4();
    let operation_root = context.operation_root(state.operation_id);
    let staging = operation_root.join(format!("backup-staging-{backup_id}"));
    anyhow::ensure!(
        !staging.exists() && !staging.is_symlink(),
        "backup staging already exists"
    );
    fs::create_dir(&staging)?;
    set_mode(&staging, 0o700)?;
    let result = async {
        let dump = staging.join("database.dump");
        dump_database(
            context,
            &context.endpoint(&state.source_database)?,
            &dump,
            &operation_root,
        )?;
        let generation = staging.join("generation");
        copy_source_generation(context, source.generation_id, &generation)?;
        require_regular_file(
            &generation.join("deployment-generation.json"),
            "source generation manifest",
        )?;
        let configuration = staging.join("configuration");
        fs::create_dir(&configuration)?;
        copy_regular_file(
            &context.config.environment_file,
            &configuration.join("server.env"),
            0o600,
        )?;
        copy_regular_file(
            &context.config.activation_file,
            &configuration.join("active.env"),
            0o600,
        )?;

        let mut entries = inventory(&staging, Some("recovery-set.json"))?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = RecoverySetManifest {
            format_version: RECOVERY_SET_FORMAT,
            backup_id,
            source_generation: source.clone(),
            entries,
            created_at: Utc::now(),
        };
        validate_recovery_manifest(&manifest)?;
        let manifest_bytes = serde_json::to_vec(&manifest)?;
        let recovery_set_digest = digest_bytes(&manifest_bytes);
        let mut pretty = serde_json::to_vec_pretty(&manifest)?;
        pretty.push(b'\n');
        write_bytes_atomic(&staging.join("recovery-set.json"), &pretty, 0o600)?;

        let backups = context.config.paths.data_root.join("backups");
        if !backups.exists() {
            fs::create_dir_all(&backups)?;
            set_mode(&backups, 0o700)?;
        }
        require_real_directory(&backups, "backup root")?;
        let artifact = backups.join(format!("{backup_id}.tar.age"));
        encrypt_directory(context, &staging, &artifact)?;
        let artifact_digest = hash_file(&artifact)?.1;
        let evidence = BackupEvidence {
            backup_id,
            source_generation_id: source.generation_id,
            artifact_digest,
            recovery_set_digest: recovery_set_digest.clone(),
            components: RecoveryComponent::required(),
            created_at: manifest.created_at,
        };
        evidence.validate(source.generation_id)?;
        state.backup = Some(evidence.clone());
        state.recovery_set_digest = Some(recovery_set_digest);
        state.updated_at = Utc::now();
        context.save_operation_state(state)?;
        Ok(evidence)
    }
    .await;
    let _ = fs::remove_dir_all(&staging);
    result
}

pub(crate) async fn restore_backup_isolated(
    context: &DriverContext,
    state: &mut DriverOperationState,
    backup: &BackupEvidence,
) -> Result<RestoreEvidence> {
    anyhow::ensure!(
        state.backup.as_ref() == Some(backup),
        "restore backup differs from operation state"
    );
    verify_existing_artifact(context, backup, state.recovery_set_digest.as_deref())?;
    if let (Some(database), Some(root)) = (&state.restore_database, &state.restore_root) {
        let observed_digest = verify_isolated_restore(context, backup, database, root).await?;
        anyhow::ensure!(
            state.restore_database_digest.as_deref() == Some(observed_digest.as_str()),
            "isolated restore database changed after verification"
        );
        return Ok(RestoreEvidence {
            backup_id: backup.backup_id,
            backup_artifact_digest: backup.artifact_digest.clone(),
            restored_generation_id: state.source_generation_id,
            isolated_restore: true,
            verified_at: Utc::now(),
        });
    }
    anyhow::ensure!(
        state.restore_database.is_none()
            && state.restore_database_digest.is_none()
            && state.restore_root.is_none(),
        "isolated restore state is partial"
    );
    let root = context
        .operation_root(state.operation_id)
        .join(format!("restore-{}", backup.backup_id));
    anyhow::ensure!(
        !root.exists() && !root.is_symlink(),
        "restore root already exists"
    );
    fs::create_dir(&root)?;
    set_mode(&root, 0o700)?;
    let restore_database = isolated_database_name("muriarc_restore", backup.backup_id);
    let result = async {
        decrypt_archive(context, backup.backup_id, &root)?;
        let manifest: RecoverySetManifest =
            serde_json::from_slice(&fs::read(root.join("recovery-set.json"))?)?;
        validate_recovery_manifest(&manifest)?;
        anyhow::ensure!(
            manifest.backup_id == backup.backup_id
                && manifest.source_generation.generation_id == backup.source_generation_id
                && digest_bytes(&serde_json::to_vec(&manifest)?) == backup.recovery_set_digest,
            "restored recovery-set manifest differs from backup evidence"
        );
        verify_inventory(&root, &manifest)?;
        recreate_database(context, &restore_database, None).await?;
        restore_database_dump(
            context,
            &context.endpoint(&restore_database)?,
            &root.join("database.dump"),
            &context.operation_root(state.operation_id),
        )?;
        let observed_digest =
            verify_isolated_restore(context, backup, &restore_database, &root).await?;
        state.restore_database = Some(restore_database.clone());
        state.restore_database_digest = Some(observed_digest);
        state.restore_root = Some(root.clone());
        state.updated_at = Utc::now();
        context.save_operation_state(state)?;
        Ok(RestoreEvidence {
            backup_id: backup.backup_id,
            backup_artifact_digest: backup.artifact_digest.clone(),
            restored_generation_id: manifest.source_generation.generation_id,
            isolated_restore: true,
            verified_at: Utc::now(),
        })
    }
    .await;
    if result.is_err() {
        let _ = drop_isolated_database(context, &restore_database).await;
        let _ = fs::remove_dir_all(&root);
    }
    result
}

async fn verify_isolated_restore(
    context: &DriverContext,
    backup: &BackupEvidence,
    database: &str,
    root: &Path,
) -> Result<String> {
    require_isolated_database(database)?;
    require_real_directory(root, "isolated restore root")?;
    let manifest: RecoverySetManifest =
        serde_json::from_slice(&fs::read(root.join("recovery-set.json"))?)?;
    validate_recovery_manifest(&manifest)?;
    anyhow::ensure!(
        manifest.backup_id == backup.backup_id
            && manifest.source_generation.generation_id == backup.source_generation_id
            && digest_bytes(&serde_json::to_vec(&manifest)?) == backup.recovery_set_digest,
        "isolated recovery manifest differs from backup evidence"
    );
    verify_inventory(root, &manifest)?;
    let pool = context.pool(database).await?;
    let generation: Uuid = sqlx::query_scalar(
        "SELECT generation_id FROM muriarc_deployment_state WHERE singleton = TRUE",
    )
    .fetch_one(&pool)
    .await?;
    let attachment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE deleted_at IS NULL")
            .fetch_one(&pool)
            .await?;
    pool.close().await;
    anyhow::ensure!(
        generation == manifest.source_generation.generation_id,
        "isolated restore generation differs from recovery manifest"
    );
    if attachment_count > 0 {
        anyhow::ensure!(
            directory_has_file(&root.join("generation/attachments"))?,
            "attachment rows were restored without attachment bytes"
        );
    }
    database_dump_digest(
        context,
        database,
        &format!("restore-{}", backup.backup_id.simple()),
    )
}

pub(crate) async fn prepare_candidate_copy(
    context: &DriverContext,
    state: &mut DriverOperationState,
    candidate_generation_id: Uuid,
) -> Result<(String, PathBuf)> {
    if let (Some(database), Some(root), Some(generation_id)) = (
        &state.candidate_database,
        &state.candidate_root,
        state.candidate_generation_id,
    ) {
        anyhow::ensure!(
            generation_id == candidate_generation_id,
            "Candidate identity changed"
        );
        require_real_directory(root, "Candidate generation root")?;
        return Ok((database.clone(), root.clone()));
    }
    let restore_database = state
        .restore_database
        .as_deref()
        .context("verified restore database is missing")?;
    let restore_root = state
        .restore_root
        .as_ref()
        .context("verified restore root is missing")?;
    let candidate_database = isolated_database_name("muriarc_candidate", candidate_generation_id);
    recreate_database(context, &candidate_database, Some(restore_database)).await?;

    let candidate_root = match context.profile() {
        DeploymentProfile::NativeSystem => context.generation_root(candidate_generation_id),
        DeploymentProfile::ManagedCompose => context
            .operation_root(state.operation_id)
            .join("candidate-generation"),
        DeploymentProfile::Desktop => unreachable!(),
    };
    let copy_result = (|| {
        anyhow::ensure!(
            !candidate_root.exists() && !candidate_root.is_symlink(),
            "Candidate root already exists"
        );
        copy_tree(&restore_root.join("generation"), &candidate_root)
    })();
    if let Err(error) = copy_result {
        if candidate_root.exists() && !candidate_root.is_symlink() {
            let _ = fs::remove_dir_all(&candidate_root);
        }
        let _ = drop_isolated_database(context, &candidate_database).await;
        return Err(error);
    }
    state.candidate_generation_id = Some(candidate_generation_id);
    state.candidate_database = Some(candidate_database.clone());
    state.candidate_root = Some(candidate_root.clone());
    state.updated_at = Utc::now();
    context.save_operation_state(state)?;
    Ok((candidate_database, candidate_root))
}

pub(crate) fn sync_candidate_to_compose(
    context: &DriverContext,
    generation_id: Uuid,
    root: &Path,
) -> Result<()> {
    if context.profile() != DeploymentProfile::ManagedCompose {
        return Ok(());
    }
    require_real_directory(root, "Candidate generation root")?;
    let container = context.compose_server_container()?;
    let container_path = format!("/var/lib/muriarc/generations/{generation_id}");
    let inspection = context
        .state_root()
        .join(format!(".compose-generation-inspect-{}", Uuid::new_v4()));
    anyhow::ensure!(
        !inspection.exists() && !inspection.is_symlink(),
        "Compose inspection path exists"
    );
    fs::create_dir(&inspection)?;
    set_mode(&inspection, 0o700)?;
    let existing_status = Command::new("/usr/bin/docker")
        .args(["cp"])
        .arg(format!("{container}:{container_path}"))
        .arg(&inspection)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if existing_status.is_ok_and(|status| status.success()) {
        let existing = inspection.join(generation_id.to_string());
        let comparison = (|| {
            require_real_directory(&existing, "existing Compose generation")?;
            anyhow::ensure!(
                tree_content_digest(&existing)? == tree_content_digest(root)?,
                "existing Compose generation differs from Candidate bytes"
            );
            Ok(())
        })();
        let _ = fs::remove_dir_all(&inspection);
        return comparison;
    }
    let _ = fs::remove_dir_all(&inspection);

    let destination = format!("{container}:{container_path}");
    let mut command = Command::new("/usr/bin/docker");
    command.args(["cp"]);
    command.arg(root);
    command.arg(destination);
    safe_status(&mut command)
}

pub(crate) fn backup_artifact_path(context: &DriverContext, backup_id: Uuid) -> PathBuf {
    context
        .config
        .paths
        .data_root
        .join("backups")
        .join(format!("{backup_id}.tar.age"))
}

pub(crate) fn prune_backup_artifact(
    context: &DriverContext,
    backup: &BackupEvidence,
) -> Result<()> {
    let path = backup_artifact_path(context, backup.backup_id);
    require_regular_file(&path, "recovery artifact")?;
    anyhow::ensure!(
        hash_file(&path)?.1 == backup.artifact_digest,
        "recovery artifact digest changed"
    );
    fs::remove_file(path)?;
    Ok(())
}

pub(crate) fn database_dump_digest(
    context: &DriverContext,
    database: &str,
    label: &str,
) -> Result<String> {
    let root = context.state_root().join("physical-driver");
    require_real_directory(&root, "driver state root")?;
    let temporary = root.join(format!("state-digest-{label}-{}.sql", Uuid::new_v4()));
    let endpoint = context.endpoint(database)?;
    let result = (|| {
        let guard = PgPassGuard::new(&root, &endpoint)?;
        let mut command = Command::new(context.pg_dump_executable()?);
        configure_pg_command(&mut command, &endpoint, &guard);
        command.args([
            "--data-only",
            "--inserts",
            "--no-owner",
            "--no-privileges",
            "--file",
        ]);
        command.arg(&temporary);
        safe_status(&mut command)?;
        normalized_database_dump_digest(&temporary)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

pub(crate) async fn recreate_database(
    context: &DriverContext,
    database: &str,
    template: Option<&str>,
) -> Result<()> {
    require_isolated_database(database)?;
    if let Some(template) = template {
        require_isolated_database(template)?;
    }
    let admin = context.endpoint("postgres")?;
    let pool = sqlx::PgPool::connect(&admin.connection_url()).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(database)
            .fetch_one(&pool)
            .await?;
    if exists {
        let query = format!("DROP DATABASE \"{database}\" WITH (FORCE)");
        sqlx::query(&query).execute(&pool).await?;
    }
    if let Some(template) = template {
        sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()")
            .bind(template)
            .execute(&pool)
            .await?;
        let query = format!("CREATE DATABASE \"{database}\" TEMPLATE \"{template}\"");
        sqlx::query(&query).execute(&pool).await?;
    } else {
        let query = format!("CREATE DATABASE \"{database}\"");
        sqlx::query(&query).execute(&pool).await?;
    }
    pool.close().await;
    Ok(())
}

pub(crate) async fn drop_isolated_database(context: &DriverContext, database: &str) -> Result<()> {
    require_isolated_database(database)?;
    let admin = context.endpoint("postgres")?;
    let pool = sqlx::PgPool::connect(&admin.connection_url()).await?;
    let query = format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)");
    sqlx::query(&query).execute(&pool).await?;
    pool.close().await;
    Ok(())
}

pub(crate) fn tree_size(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "tree contains a symlink"
        );
        if metadata.is_file() {
            total = total
                .checked_add(metadata.len())
                .context("tree size overflow")?;
        }
    }
    Ok(total)
}

pub(crate) fn verify_candidate_assets(
    state: &DriverOperationState,
    candidate_root: &Path,
) -> Result<String> {
    require_real_directory(candidate_root, "Candidate generation root")?;
    let restore_root = state
        .restore_root
        .as_ref()
        .context("verified restore root is missing")?;
    let manifest: RecoverySetManifest =
        serde_json::from_slice(&fs::read(restore_root.join("recovery-set.json"))?)?;
    validate_recovery_manifest(&manifest)?;
    let mut expected = manifest
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .path
                .strip_prefix("generation/")
                .filter(|path| *path != "deployment-generation.json")
                .map(|path| (path.to_owned(), entry.size_bytes, entry.sha256.clone()))
        })
        .collect::<Vec<_>>();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    let mut observed = inventory(candidate_root, None)?
        .into_iter()
        .filter(|entry| entry.path != "deployment-generation.json")
        .map(|entry| (entry.path, entry.size_bytes, entry.sha256))
        .collect::<Vec<_>>();
    observed.sort_by(|left, right| left.0.cmp(&right.0));
    anyhow::ensure!(
        observed == expected,
        "Candidate asset bytes differ from the verified recovery set"
    );
    Ok(digest_bytes(&serde_json::to_vec(&observed)?))
}

pub(crate) fn tree_content_digest(path: &Path) -> Result<String> {
    require_real_directory(path, "content tree")?;
    let mut entries = inventory(path, None)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(digest_bytes(&serde_json::to_vec(&entries)?))
}

pub(crate) fn hash_file(path: &Path) -> Result<(u64, String)> {
    let metadata = fs::symlink_metadata(path).context("hashed file is unavailable")?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "hashed file must be a regular non-symlink file"
    );
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .context("file size overflow")?;
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("sha256:{:x}", hasher.finalize())))
}

fn normalized_database_dump_digest(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut normalized = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"\\restrict ") || line.starts_with(b"\\unrestrict ") {
            continue;
        }
        normalized.extend_from_slice(line);
    }
    Ok(digest_bytes(&normalized))
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn dump_database(
    context: &DriverContext,
    endpoint: &DatabaseEndpoint,
    output: &Path,
    operation_root: &Path,
) -> Result<()> {
    anyhow::ensure!(
        !output.exists() && !output.is_symlink(),
        "database dump output exists"
    );
    let guard = PgPassGuard::new(operation_root, endpoint)?;
    let mut command = Command::new(context.pg_dump_executable()?);
    configure_pg_command(&mut command, endpoint, &guard);
    command.args(["--format=custom", "--no-owner", "--no-acl", "--file"]);
    command.arg(output);
    safe_status(&mut command)?;
    require_regular_file(output, "database dump")?;
    let _ = context;
    Ok(())
}

fn restore_database_dump(
    context: &DriverContext,
    endpoint: &DatabaseEndpoint,
    input: &Path,
    operation_root: &Path,
) -> Result<()> {
    require_regular_file(input, "database dump")?;
    let guard = PgPassGuard::new(operation_root, endpoint)?;
    let mut command = Command::new(context.pg_restore_executable()?);
    configure_pg_command(&mut command, endpoint, &guard);
    command.args(["--exit-on-error", "--no-owner", "--no-acl"]);
    command.arg(input);
    safe_status(&mut command)
}

fn configure_pg_command(command: &mut Command, endpoint: &DatabaseEndpoint, guard: &PgPassGuard) {
    command
        .args(["--host", endpoint.host()])
        .arg("--port")
        .arg(endpoint.port().to_string())
        .args(["--username", endpoint.username()])
        .arg("--dbname")
        .arg(
            endpoint
                .database_name()
                .expect("validated database endpoint"),
        )
        .env("PGPASSFILE", &guard.path)
        .env_remove("PGPASSWORD");
}

struct PgPassGuard {
    path: PathBuf,
}

impl PgPassGuard {
    fn new(root: &Path, endpoint: &DatabaseEndpoint) -> Result<Self> {
        require_real_directory(root, "pgpass parent")?;
        let path = root.join(format!(".pgpass-{}", Uuid::new_v4()));
        let escape = |value: &str| value.replace('\\', "\\\\").replace(':', "\\:");
        let line = format!(
            "{}:{}:{}:{}:{}\n",
            escape(endpoint.host()),
            endpoint.port(),
            escape(&endpoint.database_name()?),
            escape(endpoint.username()),
            escape(endpoint.password()),
        );
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        Ok(Self { path })
    }
}

impl Drop for PgPassGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn copy_source_generation(
    context: &DriverContext,
    generation_id: Uuid,
    destination: &Path,
) -> Result<()> {
    match context.profile() {
        DeploymentProfile::NativeSystem => {
            copy_tree(&context.generation_root(generation_id), destination)
        }
        DeploymentProfile::ManagedCompose => {
            fs::create_dir(destination)?;
            set_mode(destination, 0o700)?;
            let container = context.compose_server_container()?;
            let source = format!("{container}:/var/lib/muriarc/generations/{generation_id}/.");
            let mut command = Command::new("/usr/bin/docker");
            command.args(["cp"]);
            command.arg(source);
            command.arg(destination);
            safe_status(&mut command)
        }
        DeploymentProfile::Desktop => unreachable!(),
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    require_real_directory(source, "tree source")?;
    anyhow::ensure!(
        !destination.exists() && !destination.is_symlink(),
        "tree destination exists"
    );
    fs::create_dir_all(destination)?;
    set_mode(destination, 0o700)?;
    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        validate_relative_path(relative)?;
        let target = destination.join(relative);
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "tree source contains symlink"
        );
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            set_mode(&target, 0o700)?;
        } else if metadata.is_file() {
            copy_regular_file(entry.path(), &target, 0o600)?;
        } else {
            anyhow::bail!("tree source contains unsupported entry");
        }
    }
    Ok(())
}

fn copy_regular_file(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    let metadata = fs::symlink_metadata(source).context("copy source is unavailable")?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "copy source must be a regular non-symlink file"
    );
    let parent = destination
        .parent()
        .context("copy destination has no parent")?;
    require_real_directory(parent, "copy destination parent")?;
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    let mut output = options.open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn inventory(root: &Path, excluded: Option<&str>) -> Result<Vec<RecoverySetEntry>> {
    require_real_directory(root, "inventory root")?;
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        validate_relative_path(relative)?;
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if excluded == Some(relative_text.as_str()) {
            continue;
        }
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "inventory contains symlink"
        );
        if metadata.is_file() {
            let (size_bytes, sha256) = hash_file(entry.path())?;
            entries.push(RecoverySetEntry {
                path: relative_text,
                size_bytes,
                sha256,
            });
        } else {
            anyhow::ensure!(metadata.is_dir(), "inventory contains unsupported entry");
        }
    }
    Ok(entries)
}

fn validate_recovery_manifest(manifest: &RecoverySetManifest) -> Result<()> {
    anyhow::ensure!(
        manifest.format_version == RECOVERY_SET_FORMAT
            && !manifest.backup_id.is_nil()
            && !manifest.source_generation.generation_id.is_nil()
            && !manifest.entries.is_empty(),
        "recovery-set manifest identity is invalid"
    );
    let mut paths = BTreeSet::new();
    for entry in &manifest.entries {
        let path = Path::new(&entry.path);
        validate_relative_path(path)?;
        anyhow::ensure!(
            valid_digest(&entry.sha256) && paths.insert(entry.path.as_str()),
            "recovery-set entry is invalid"
        );
    }
    for required in [
        "database.dump",
        "configuration/server.env",
        "configuration/active.env",
        "generation/deployment-generation.json",
    ] {
        anyhow::ensure!(paths.contains(required), "recovery set is incomplete");
    }
    Ok(())
}

fn verify_inventory(root: &Path, manifest: &RecoverySetManifest) -> Result<()> {
    let mut observed = inventory(root, Some("recovery-set.json"))?;
    observed.sort_by(|left, right| left.path.cmp(&right.path));
    anyhow::ensure!(
        observed.len() == manifest.entries.len(),
        "restored recovery-set inventory length differs"
    );
    for (observed, expected) in observed.iter().zip(&manifest.entries) {
        anyhow::ensure!(
            observed.path == expected.path
                && observed.size_bytes == expected.size_bytes
                && observed.sha256 == expected.sha256,
            "restored recovery-set entry differs"
        );
    }
    Ok(())
}

fn encrypt_directory(context: &DriverContext, root: &Path, output: &Path) -> Result<()> {
    require_real_directory(root, "backup staging root")?;
    anyhow::ensure!(
        !output.exists() && !output.is_symlink(),
        "encrypted backup output exists"
    );
    let parent = output
        .parent()
        .context("encrypted backup output has no parent")?;
    require_real_directory(parent, "encrypted backup parent")?;
    let temporary = parent.join(format!(".backup-{}.tmp", Uuid::new_v4()));
    let recipient = context.backup_recipient_file()?;
    let result = (|| {
        let mut child = Command::new(context.age_executable()?)
            .args(["--encrypt", "--recipients-file"])
            .arg(recipient)
            .args(["--output"])
            .arg(&temporary)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().context("age stdin is unavailable")?;
        let mut archive = tar::Builder::new(stdin);
        archive.follow_symlinks(false);
        let archive_result = (|| {
            for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
                let entry = entry?;
                let relative = entry.path().strip_prefix(root)?;
                validate_relative_path(relative)?;
                let metadata = entry.path().symlink_metadata()?;
                anyhow::ensure!(
                    !metadata.file_type().is_symlink(),
                    "backup tree contains symlink"
                );
                if metadata.is_dir() {
                    archive.append_dir(relative, entry.path())?;
                } else if metadata.is_file() {
                    archive.append_path_with_name(entry.path(), relative)?;
                } else {
                    anyhow::bail!("backup tree contains unsupported entry");
                }
            }
            archive.finish()?;
            Ok::<_, anyhow::Error>(())
        })();
        drop(archive);
        let status = child.wait()?;
        archive_result?;
        anyhow::ensure!(status.success(), "backup encryption failed");
        require_regular_file(&temporary, "temporary encrypted backup artifact")?;
        anyhow::ensure!(
            !output.exists() && !output.is_symlink(),
            "encrypted backup output appeared during creation"
        );
        fs::rename(&temporary, output)?;
        require_regular_file(output, "encrypted backup artifact")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn decrypt_archive(context: &DriverContext, backup_id: Uuid, output: &Path) -> Result<()> {
    require_real_directory(output, "restore output")?;
    let artifact = backup_artifact_path(context, backup_id);
    require_regular_file(&artifact, "encrypted backup artifact")?;
    let identity = context.backup_identity_file()?;
    let mut child = Command::new(context.age_executable()?)
        .args(["--decrypt", "--identity"])
        .arg(identity)
        .arg(artifact)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().context("age stdout is unavailable")?;
    let mut archive = tar::Archive::new(stdout);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        validate_relative_path(&path)?;
        let kind = entry.header().entry_type();
        anyhow::ensure!(
            kind.is_file() || kind.is_dir(),
            "recovery archive contains unsupported entry"
        );
        anyhow::ensure!(
            entry.unpack_in(output)?,
            "recovery archive entry escaped output"
        );
    }
    drop(archive);
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "backup decryption failed");
    Ok(())
}

fn verify_existing_artifact(
    context: &DriverContext,
    backup: &BackupEvidence,
    recovery_set_digest: Option<&str>,
) -> Result<()> {
    backup.validate(backup.source_generation_id)?;
    anyhow::ensure!(
        recovery_set_digest == Some(backup.recovery_set_digest.as_str()),
        "recovery-set digest differs from operation state"
    );
    let artifact = backup_artifact_path(context, backup.backup_id);
    anyhow::ensure!(
        hash_file(&artifact)?.1 == backup.artifact_digest,
        "encrypted backup artifact digest changed"
    );
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    anyhow::ensure!(
        !path.as_os_str().is_empty() && !path.is_absolute(),
        "path is unsafe"
    );
    for component in path.components() {
        anyhow::ensure!(matches!(component, Component::Normal(_)), "path is unsafe");
    }
    Ok(())
}

fn require_isolated_database(database: &str) -> Result<()> {
    require_database_name(database)?;
    anyhow::ensure!(
        [
            "muriarc_restore_",
            "muriarc_candidate_",
            "muriarc_verify_",
            "muriarc_recovery_",
        ]
        .iter()
        .any(|prefix| database.starts_with(prefix)),
        "database is outside the isolated namespace"
    );
    Ok(())
}

fn isolated_database_name(prefix: &str, id: Uuid) -> String {
    format!("{prefix}_{}", id.simple())
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn directory_has_file(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    require_real_directory(path, "attachment directory")?;
    for entry in WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "attachment tree contains symlink"
        );
        if metadata.is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_database_namespace_rejects_user_database() {
        assert!(require_isolated_database("muriarc_candidate_123").is_ok());
        assert!(require_isolated_database("muriarc").is_err());
        assert!(require_isolated_database("postgres").is_err());
    }

    #[test]
    fn recovery_paths_reject_traversal_and_absolute_paths() {
        assert!(validate_relative_path(Path::new("generation/data/file")).is_ok());
        assert!(validate_relative_path(Path::new("../secret")).is_err());
        assert!(validate_relative_path(Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn generation_copy_preserves_legitimate_empty_files() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("empty-state"), []).unwrap();
        copy_tree(&source, &destination).unwrap();
        assert_eq!(
            fs::metadata(destination.join("empty-state")).unwrap().len(),
            0
        );
    }
    #[test]
    fn logical_dump_digest_ignores_pg17_random_restrict_keys() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first.sql");
        let second = temporary.path().join("second.sql");
        fs::write(
            &first,
            b"\\restrict random-one\nINSERT INTO labs VALUES (1);\n\\unrestrict random-one\n",
        )
        .unwrap();
        fs::write(
            &second,
            b"\\restrict random-two\nINSERT INTO labs VALUES (1);\n\\unrestrict random-two\n",
        )
        .unwrap();
        assert_eq!(
            normalized_database_dump_digest(&first).unwrap(),
            normalized_database_dump_digest(&second).unwrap()
        );
    }
}

#[cfg(test)]
mod postgres_tests {
    use anyhow::{Context as _, Result};
    use muriarc_core::MuriArcStore;
    use muriarc_store_postgres::PostgresStore;
    use sqlx::{Connection as _, PgConnection};
    use tempfile::tempdir;

    use super::*;
    use crate::context::{require_executable, write_json_atomic};

    fn context_client_prerequisites(context: &DriverContext) {
        for executable in [
            context.pg_dump_executable(),
            context.pg_restore_executable(),
        ] {
            let executable = executable.expect("configured PostgreSQL client must be available");
            assert!(
                require_executable(&executable, "PostgreSQL client").is_ok(),
                "Physical Driver integration requires {}",
                executable.display()
            );
        }
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
    async fn encrypted_joint_restore_is_idempotent_and_detects_database_drift() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping encrypted Physical Driver recovery integration: MURIARC_TEST_DATABASE_URL is not set"
            );
            return;
        };
        for name in [
            "MURIARCCTL_BACKUP_RECIPIENT_FILE",
            "MURIARCCTL_BACKUP_IDENTITY_FILE",
            "MURIARCCTL_AGE_EXECUTABLE",
        ] {
            if std::env::var_os(name).is_none() {
                eprintln!(
                    "skipping encrypted Physical Driver recovery integration: {name} is not set"
                );
                return;
            }
        }
        assert!(
            database_url.contains("muriarc_test"),
            "Physical Driver integration requires the disposable muriarc_test server"
        );
        let source_database = format!("muriarc_recovery_{}", Uuid::new_v4().simple());
        let temporary = tempdir().expect("Physical Driver test root must be created");
        let context = DriverContext::for_test(&database_url, &source_database, temporary.path())
            .expect("Physical Driver test context must be valid");
        context_client_prerequisites(&context);
        context
            .age_executable()
            .expect("configured age executable must be available");
        context
            .backup_recipient_file()
            .expect("configured age recipient must be available");
        context
            .backup_identity_file()
            .expect("configured private age identity must be available");
        let operation_id = Uuid::new_v4();
        let operation_root = context.operation_root(operation_id);
        fs::create_dir_all(&operation_root).unwrap();
        set_mode(&operation_root, 0o700).unwrap();
        let mut restore_database = None;
        let mut backup_artifact = None;

        let outcome: Result<()> = async {
            recreate_database(&context, &source_database, None).await?;
            let store =
                PostgresStore::connect(&context.endpoint(&source_database)?.connection_url())
                    .await?;
            store.migrate().await?;
            let repository = context.repository(&source_database).await?;
            let source = repository.current_generation().await?;
            let generation = context.generation_root(source.generation_id);
            fs::create_dir_all(generation.join("data"))?;
            fs::create_dir_all(generation.join("attachments"))?;
            fs::create_dir_all(generation.join("secrets"))?;
            fs::write(generation.join("data/empty-runtime-state"), [])?;
            fs::write(
                generation.join("secrets/ai-master-key"),
                b"synthetic-physical-driver-key-material",
            )?;
            write_json_atomic(
                &generation.join("deployment-generation.json"),
                &muriarc_core::DeploymentGenerationManifest {
                    format_version: muriarc_core::GENERATION_MANIFEST_FORMAT,
                    generation_id: source.generation_id,
                    data_epoch: source.identity.data_epoch.clone(),
                    backend_state_digest: source.identity.backend_state_digest.clone(),
                },
                0o600,
            )?;
            repository.pool().close().await;
            store.pool().close().await;

            let now = Utc::now();
            let mut state = DriverOperationState {
                format_version: crate::model::DRIVER_STATE_FORMAT,
                operation_id,
                source_generation_id: source.generation_id,
                source_database: source_database.clone(),
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
            let backup = create_backup(&context, &mut state, &source).await?;
            backup_artifact = Some(backup_artifact_path(&context, backup.backup_id));
            let first = restore_backup_isolated(&context, &mut state, &backup).await?;
            let repeated = restore_backup_isolated(&context, &mut state, &backup).await?;
            anyhow::ensure!(
                first.restored_generation_id == source.generation_id
                    && repeated.restored_generation_id == source.generation_id,
                "repeated isolated restore changed generation identity"
            );
            restore_database = state.restore_database.clone();
            let database = state
                .restore_database
                .as_deref()
                .context("verified restore database is missing")?;
            let pool = context.pool(database).await?;
            sqlx::query(
                "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
                 VALUES ($1, 'Restore drift', $2, $2, NULL, 1)",
            )
            .bind(Uuid::new_v4())
            .bind(Utc::now())
            .execute(&pool)
            .await?;
            pool.close().await;
            let error = restore_backup_isolated(&context, &mut state, &backup)
                .await
                .expect_err("tampered isolated restore must fail closed");
            anyhow::ensure!(
                error.to_string().contains("changed after verification"),
                "restore drift failed for an unexpected reason"
            );
            Ok(())
        }
        .await;

        if let Some(database) = restore_database.as_deref() {
            let _ = drop_isolated_database(&context, database).await;
        }
        let _ = drop_isolated_database(&context, &source_database).await;
        if let Some(artifact) = backup_artifact {
            let _ = fs::remove_file(artifact);
        }
        let source_residual = database_exists(&database_url, &source_database)
            .await
            .expect("source residual database check must run");
        let restore_residual = if let Some(database) = restore_database.as_deref() {
            database_exists(&database_url, database)
                .await
                .expect("restore residual database check must run")
        } else {
            false
        };
        let residual = source_residual || restore_residual;
        assert!(
            !residual,
            "encrypted recovery test left an isolated database behind"
        );
        outcome.expect("encrypted joint recovery lifecycle must pass");
    }

    #[tokio::test]
    async fn physical_dump_restore_and_candidate_clone_leave_no_databases() {
        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping Physical Driver dump/restore integration: MURIARC_TEST_DATABASE_URL is not set"
            );
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "Physical Driver integration requires the disposable muriarc_test server"
        );
        let source_database = format!("muriarc_recovery_{}", Uuid::new_v4().simple());
        let restore_database = format!("muriarc_restore_{}", Uuid::new_v4().simple());
        let candidate_database = format!("muriarc_candidate_{}", Uuid::new_v4().simple());
        let temporary = tempdir().expect("Physical Driver test root must be created");
        let context = DriverContext::for_test(&database_url, &source_database, temporary.path())
            .expect("Physical Driver test context must be valid");
        context_client_prerequisites(&context);

        let outcome: Result<()> = async {
            recreate_database(&context, &source_database, None).await?;
            let store =
                PostgresStore::connect(&context.endpoint(&source_database)?.connection_url())
                    .await?;
            store.migrate().await?;
            let lab_id = Uuid::new_v4();
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
                 VALUES ($1, 'Physical backup lab', $2, $2, NULL, 1)",
            )
            .bind(lab_id)
            .bind(now)
            .execute(store.pool())
            .await?;
            store.pool().close().await;

            let operation_root = temporary.path().join("operation");
            fs::create_dir(&operation_root)?;
            let dump = operation_root.join("database.dump");
            dump_database(
                &context,
                &context.endpoint(&source_database)?,
                &dump,
                &operation_root,
            )?;
            recreate_database(&context, &restore_database, None).await?;
            restore_database_dump(
                &context,
                &context.endpoint(&restore_database)?,
                &dump,
                &operation_root,
            )?;
            let restore_pool = context.pool(&restore_database).await?;
            let restored_name: String = sqlx::query_scalar("SELECT name FROM labs WHERE id = $1")
                .bind(lab_id)
                .fetch_one(&restore_pool)
                .await?;
            anyhow::ensure!(
                restored_name == "Physical backup lab",
                "restored row differs"
            );
            restore_pool.close().await;

            recreate_database(&context, &candidate_database, Some(&restore_database)).await?;
            let candidate_pool = context.pool(&candidate_database).await?;
            let candidate_name: String = sqlx::query_scalar("SELECT name FROM labs WHERE id = $1")
                .bind(lab_id)
                .fetch_one(&candidate_pool)
                .await?;
            anyhow::ensure!(candidate_name == restored_name, "Candidate clone differs");
            candidate_pool.close().await;
            Ok(())
        }
        .await;

        for database in [&candidate_database, &restore_database, &source_database] {
            let _ = drop_isolated_database(&context, database).await;
        }
        let mut residual = false;
        for database in [&candidate_database, &restore_database, &source_database] {
            residual |= database_exists(&database_url, database)
                .await
                .with_context(|| format!("residual check failed for {database}"))
                .expect("Physical Driver residual database check must run");
        }
        assert!(
            !residual,
            "Physical Driver left an isolated database behind"
        );
        outcome.expect("Physical Driver dump/restore lifecycle must pass");
    }
}
