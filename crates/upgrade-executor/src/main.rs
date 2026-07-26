use std::{
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use muriarc_core::{
    BackendKind, DeploymentGenerationManifest, MuriArcStore, ReleaseIdentity, ReleaseManifest,
};
use muriarc_store_postgres::PostgresStore;
use muriarc_upgrade::PostgresAdvisoryLock;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug)]
struct ApplyRequest {
    release_manifest: PathBuf,
    operation_id: Uuid,
    source_generation_id: Uuid,
    candidate_generation_id: Uuid,
    json: bool,
}

#[derive(Debug, Serialize)]
struct ApplyResponse {
    ok: bool,
    operation_id: Uuid,
    source_generation_id: Uuid,
    candidate_generation_id: Uuid,
    target_identity: ReleaseIdentity,
    write_lease_absent: bool,
    generation_manifest: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok((json, response)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&response).expect("response must serialize")
                );
            } else {
                println!(
                    "OK [candidate_migrated] generation {} is prepared read-only",
                    response.candidate_generation_id
                );
            }
            ExitCode::SUCCESS
        }
        Err(_error) => {
            eprintln!(
                "ERROR [candidate_migration_failed] Candidate prerequisite or migration failed; secrets and database diagnostics were suppressed"
            );
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<(bool, ApplyResponse), Box<dyn Error>> {
    let request = parse_args(env::args().skip(1).collect())?;
    require_candidate_guards()?;
    let database_url = required_secret_env("MURIARC_CANDIDATE_DATABASE_URL")?;
    let expected_database_name = required_env("MURIARC_CANDIDATE_DATABASE_NAME")?;
    if !expected_database_name.starts_with("muriarc_candidate_")
        || !expected_database_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("Candidate database name must use the muriarc_candidate_ prefix".into());
    }
    let data_root = real_directory("MURIARC_CANDIDATE_DATA_ROOT")?;
    let attachment_root = real_directory("MURIARC_CANDIDATE_ATTACHMENT_ROOT")?;
    require_source_generation_manifest(&data_root, request.source_generation_id)?;
    let manifest = load_release_manifest(&request.release_manifest)?;
    let target_identity = release_identity(&manifest)?;

    let _candidate_lock = PostgresAdvisoryLock::acquire(&database_url).await?;
    let store = PostgresStore::connect(&database_url).await?;
    let actual_database_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(store.pool())
        .await?;
    if actual_database_name != expected_database_name {
        return Err("Candidate database URL does not select the declared isolated database".into());
    }
    require_persisted_operation(&store, &request, &target_identity).await?;

    store.apply_upgrade_migrations().await?;
    let before_prepare = store.compatibility_report().await?;
    if before_prepare.expected != target_identity {
        return Err("signed Release Manifest does not match executor migrations".into());
    }
    let state = store
        .prepare_upgraded_candidate(
            request.source_generation_id,
            request.candidate_generation_id,
        )
        .await?;
    if state.identity != target_identity || state.write_lease_id.is_some() {
        return Err("Candidate did not enter the exact read-only target identity".into());
    }

    let inventory = store.persistent_recovery_inventory().await?;
    if inventory.attachment_records != 0 && directory_is_empty(&attachment_root)? {
        return Err("Candidate attachment metadata exists but attachment root is empty".into());
    }
    if inventory.encrypted_secret_records != 0 {
        let key_path = PathBuf::from(required_env("MURIARC_CANDIDATE_MASTER_KEY_FILE")?);
        let metadata = fs::symlink_metadata(&key_path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Candidate Master Key must be a regular non-symlink file".into());
        }
    }
    let generation_manifest = data_root.join("deployment-generation.json");
    write_generation_manifest(
        &generation_manifest,
        &DeploymentGenerationManifest::from_state(&state),
    )?;
    Ok((
        request.json,
        ApplyResponse {
            ok: true,
            operation_id: request.operation_id,
            source_generation_id: request.source_generation_id,
            candidate_generation_id: request.candidate_generation_id,
            target_identity,
            write_lease_absent: true,
            generation_manifest: generation_manifest.display().to_string(),
        },
    ))
}

fn parse_args(args: Vec<String>) -> Result<ApplyRequest, Box<dyn Error>> {
    if args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--force" | "--skip-verify" | "migrate" | "raw-migration"
        )
    }) {
        return Err("raw migration and safety bypass arguments are unavailable".into());
    }
    if args.first().map(String::as_str) != Some("apply") {
        return Err("usage: muriarc-upgrade-executor apply --release-manifest <path> --operation <uuid> --source-generation <uuid> --candidate-generation <uuid> [--output json]".into());
    }
    let value = |name: &str| -> Result<&str, Box<dyn Error>> {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
            .ok_or_else(|| format!("{name} is required").into())
    };
    let json = args
        .windows(2)
        .any(|pair| pair[0] == "--output" && pair[1] == "json");
    let request = ApplyRequest {
        release_manifest: PathBuf::from(value("--release-manifest")?),
        operation_id: value("--operation")?.parse()?,
        source_generation_id: value("--source-generation")?.parse()?,
        candidate_generation_id: value("--candidate-generation")?.parse()?,
        json,
    };
    if request.operation_id.is_nil()
        || request.source_generation_id.is_nil()
        || request.candidate_generation_id.is_nil()
        || request.source_generation_id == request.candidate_generation_id
    {
        return Err("operation and generation IDs must be distinct non-nil UUIDs".into());
    }
    Ok(request)
}

fn require_candidate_guards() -> Result<(), Box<dyn Error>> {
    for name in [
        "MURIARC_CANDIDATE_EXTERNAL_PROVIDERS_DISABLED",
        "MURIARC_CANDIDATE_BACKGROUND_JOBS_DISABLED",
        "MURIARC_CANDIDATE_REAL_USER_WRITES_DISABLED",
    ] {
        if !required_env(name)?.trim().eq_ignore_ascii_case("true") {
            return Err(format!("{name} must be true").into());
        }
    }
    Ok(())
}

async fn require_persisted_operation(
    store: &PostgresStore,
    request: &ApplyRequest,
    target: &ReleaseIdentity,
) -> Result<(), Box<dyn Error>> {
    let row = sqlx::query(
        "SELECT source_generation_id, target_application_version, target_data_epoch,
                target_backend_state_digest, target_gateway_contract_revision, status
           FROM muriarc_upgrade_operations
          WHERE operation_id = $1",
    )
    .bind(request.operation_id)
    .fetch_optional(store.pool())
    .await?
    .ok_or("Candidate copy does not contain the persisted upgrade operation")?;
    let matches = row.try_get::<Uuid, _>("source_generation_id")? == request.source_generation_id
        && row.try_get::<String, _>("target_application_version")?
            == target.application_version.as_str()
        && row.try_get::<String, _>("target_data_epoch")? == target.data_epoch.as_str()
        && row.try_get::<String, _>("target_backend_state_digest")?
            == target.backend_state_digest.as_str()
        && row.try_get::<String, _>("target_gateway_contract_revision")?
            == target.gateway_contract_revision.as_str()
        && row.try_get::<String, _>("status")? == "running";
    if !matches {
        return Err("Candidate persisted operation differs from requested signed target".into());
    }
    Ok(())
}

fn load_release_manifest(path: &Path) -> Result<ReleaseManifest, Box<dyn Error>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Release Manifest must be a regular non-symlink file".into());
    }
    let manifest: ReleaseManifest = serde_json::from_slice(&fs::read(path)?)?;
    manifest
        .validate()
        .map_err(|error| -> Box<dyn Error> { error.into() })?;
    Ok(manifest)
}

fn release_identity(manifest: &ReleaseManifest) -> Result<ReleaseIdentity, Box<dyn Error>> {
    ReleaseIdentity::parse(
        manifest.application_version.to_string(),
        manifest.data_epoch.to_string(),
        manifest
            .backend_states
            .get(&BackendKind::Postgres)
            .ok_or("Release Manifest is missing PostgreSQL state")?
            .to_string(),
        manifest.gateway_contract_revision.to_string(),
    )
    .map_err(Into::into)
}

fn real_directory(name: &'static str) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(required_env(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute").into());
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{name} must be a real directory").into());
    }
    Ok(path)
}

fn write_generation_manifest(
    path: &Path,
    manifest: &DeploymentGenerationManifest,
) -> Result<(), Box<dyn Error>> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error.into())
        }
    }
}

fn require_source_generation_manifest(
    data_root: &Path,
    source_generation_id: Uuid,
) -> Result<(), Box<dyn Error>> {
    let path = data_root.join("deployment-generation.json");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Candidate source generation manifest must be a regular file".into());
    }
    let manifest: DeploymentGenerationManifest = serde_json::from_slice(&fs::read(path)?)?;
    if manifest.generation_id != source_generation_id {
        return Err("Candidate data root belongs to another source generation".into());
    }
    Ok(())
}

fn directory_is_empty(path: &Path) -> Result<bool, Box<dyn Error>> {
    Ok(fs::read_dir(path)?.next().transpose()?.is_none())
}

fn required_secret_env(name: &'static str) -> Result<String, Box<dyn Error>> {
    required_env(name)
}

fn required_env(name: &'static str) -> Result<String, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) | Err(env::VarError::NotPresent) => {
            Err(format!("required environment variable {name} is not set").into())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_raw_migration_and_nil_generations() {
        assert!(parse_args(vec!["migrate".to_owned()]).is_err());
        assert!(
            parse_args(vec![
                "apply".to_owned(),
                "--force".to_owned(),
                "--release-manifest".to_owned(),
                "x".to_owned(),
            ])
            .is_err()
        );
    }
}
