use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context as _, Result};
use muriarc_delivery::{
    DeliveryConfig, InstallReceipt, ProcessCommandRunner, ServerServiceController,
    VerifiedServerBundle, load_delivery_config, load_install_state, verify_server_bundle,
};
use muriarc_upgrade::{
    DeploymentProfile, PostgresAdvisoryLock, PostgresUpgradeRepository, UpgradeSnapshot,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use url::Url;
use uuid::Uuid;

use crate::model::{DRIVER_STATE_FORMAT, DriverOperationState, Environment};

const STATE_ROOT_ENV: &str = "MURIARCCTL_STATE_ROOT";
const INSTALL_ROOT_ENV: &str = "MURIARCCTL_INSTALL_ROOT";
const POSTGRES_ADMIN_URL_ENV: &str = "MURIARCCTL_POSTGRES_ADMIN_URL";
const PG_DUMP_EXECUTABLE_ENV: &str = "MURIARCCTL_PG_DUMP_EXECUTABLE";
const PG_RESTORE_EXECUTABLE_ENV: &str = "MURIARCCTL_PG_RESTORE_EXECUTABLE";

pub(crate) struct DriverContext {
    pub(crate) config: DeliveryConfig,
    pub(crate) receipt: InstallReceipt,
    pub(crate) bundle: VerifiedServerBundle,
    pub(crate) environment: Environment,
    pub(crate) activation: Environment,
    admin: DatabaseEndpoint,
    active_database: String,
}

#[derive(Clone)]
pub(crate) struct DatabaseEndpoint {
    url: Url,
    password: String,
}

impl DatabaseEndpoint {
    fn parse(value: &str) -> Result<Self> {
        let url = Url::parse(value).context("PostgreSQL endpoint is invalid")?;
        anyhow::ensure!(
            matches!(url.scheme(), "postgres" | "postgresql")
                && !url.username().is_empty()
                && url.password().is_some()
                && url.host_str().is_some()
                && url.port_or_known_default().is_some()
                && database_name_from_url(&url).is_some(),
            "PostgreSQL endpoint is incomplete"
        );
        let password = percent_encoding::percent_decode_str(
            url.password()
                .expect("endpoint validation requires password"),
        )
        .decode_utf8()
        .context("PostgreSQL password encoding is invalid")?
        .into_owned();
        Ok(Self { url, password })
    }

    pub(crate) fn database_name(&self) -> Result<String> {
        database_name_from_url(&self.url)
            .map(str::to_owned)
            .context("PostgreSQL endpoint has no database")
    }

    pub(crate) fn with_database(&self, database: &str) -> Result<Self> {
        require_database_name(database)?;
        let mut url = self.url.clone();
        url.set_path(&format!("/{database}"));
        Ok(Self {
            url,
            password: self.password.clone(),
        })
    }

    pub(crate) fn connection_url(&self) -> String {
        self.url.to_string()
    }

    pub(crate) fn host(&self) -> &str {
        self.url
            .host_str()
            .expect("endpoint validation requires host")
    }

    pub(crate) fn port(&self) -> u16 {
        self.url
            .port_or_known_default()
            .expect("endpoint validation requires port")
    }

    pub(crate) fn username(&self) -> &str {
        self.url.username()
    }

    pub(crate) fn password(&self) -> &str {
        &self.password
    }
}

impl DriverContext {
    pub(crate) fn load(profile: DeploymentProfile) -> Result<Self> {
        anyhow::ensure!(
            profile != DeploymentProfile::Desktop,
            "Desktop is not a Server Driver"
        );
        let state_root = state_root();
        require_real_directory(&state_root, "control root")?;
        let config = load_delivery_config(&state_root).context("delivery config is invalid")?;
        anyhow::ensure!(
            config.profile == profile,
            "driver profile differs from delivery config"
        );
        let receipt = load_install_state(&config)
            .context("install receipt could not be loaded")?
            .context("install receipt is missing")?;
        let (_, bundle) =
            verify_server_bundle(&receipt.release_path, Some(&receipt.manifest_digest))
                .context("installed bundle verification failed")?;
        anyhow::ensure!(
            bundle
                == VerifiedServerBundle {
                    application_version: receipt.application_version.clone(),
                    profile: receipt.profile,
                    manifest_digest: receipt.manifest_digest.clone(),
                    content_digest: receipt.content_digest.clone(),
                    file_count: bundle.file_count,
                    total_bytes: bundle.total_bytes,
                },
            "install receipt differs from installed bundle"
        );
        let environment = read_environment(&config.environment_file)?;
        let activation = read_environment(&config.activation_file)?;
        let (admin, active_database) = match profile {
            DeploymentProfile::NativeSystem => {
                let admin = DatabaseEndpoint::parse(&required_env(POSTGRES_ADMIN_URL_ENV)?)?;
                let active =
                    DatabaseEndpoint::parse(required_value(&activation, "MURIARC_DATABASE_URL")?)?;
                (admin, active.database_name()?)
            }
            DeploymentProfile::ManagedCompose => {
                let host = compose_database_host(&config)?;
                let user = required_value(&environment, "MURIARC_POSTGRES_USER")?;
                let password = required_value(&environment, "MURIARC_POSTGRES_PASSWORD")?;
                let database = required_value(&activation, "MURIARC_POSTGRES_DB")?;
                require_database_name(database)?;
                let mut url = Url::parse(&format!("postgresql://{host}:5432/{database}"))?;
                url.set_username(user)
                    .map_err(|_| anyhow::anyhow!("PostgreSQL user is invalid"))?;
                url.set_password(Some(password))
                    .map_err(|_| anyhow::anyhow!("PostgreSQL password is invalid"))?;
                (DatabaseEndpoint::parse(url.as_str())?, database.to_owned())
            }
            DeploymentProfile::Desktop => unreachable!(),
        };
        Ok(Self {
            config,
            receipt,
            bundle,
            environment,
            activation,
            admin,
            active_database,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(database_url: &str, active_database: &str, root: &Path) -> Result<Self> {
        require_database_name(active_database)?;
        let paths = muriarc_delivery::DeliveryPaths::managed_compose(root);
        for directory in [
            &paths.release_root,
            &paths.config_root,
            &paths.data_root,
            &paths.control_root,
            &paths.data_root.join("generations"),
        ] {
            fs::create_dir_all(directory)?;
        }
        let release_path = paths.release_root.join("test-release");
        fs::create_dir_all(&release_path)?;
        let environment_file = paths.config_root.join("server.env");
        let activation_file = paths.control_root.join("active.env");
        fs::write(&environment_file, b"MURIARC_TEST=1\n")?;
        fs::write(&activation_file, b"MURIARC_TEST=1\n")?;
        let profile = DeploymentProfile::NativeSystem;
        let application_version: muriarc_core::ApplicationVersion =
            "1.0.0".parse().map_err(anyhow::Error::msg)?;
        let manifest_digest = format!("sha256:{}", "a".repeat(64));
        let content_digest = format!("sha256:{}", "b".repeat(64));
        Ok(Self {
            config: DeliveryConfig {
                format_version: muriarc_delivery::DELIVERY_CONFIG_FORMAT,
                profile,
                paths,
                service_user: "muriarc".to_owned(),
                loopback_origin: "http://127.0.0.1:8787".to_owned(),
                environment_file,
                activation_file,
                compose_project: None,
                compose_file: None,
            },
            receipt: InstallReceipt {
                format_version: muriarc_delivery::INSTALL_RECEIPT_FORMAT,
                profile,
                application_version: application_version.clone(),
                manifest_digest: manifest_digest.clone(),
                content_digest: content_digest.clone(),
                release_path,
                installed_at: chrono::Utc::now(),
            },
            bundle: VerifiedServerBundle {
                application_version,
                profile,
                manifest_digest,
                content_digest,
                file_count: 1,
                total_bytes: 1,
            },
            environment: Environment::from([("MURIARC_TEST".to_owned(), "1".to_owned())]),
            activation: Environment::from([("MURIARC_TEST".to_owned(), "1".to_owned())]),
            admin: DatabaseEndpoint::parse(database_url)?,
            active_database: active_database.to_owned(),
        })
    }

    pub(crate) fn profile(&self) -> DeploymentProfile {
        self.config.profile
    }

    pub(crate) fn state_root(&self) -> &Path {
        &self.config.paths.control_root
    }

    pub(crate) fn active_database(&self) -> &str {
        &self.active_database
    }

    pub(crate) fn endpoint(&self, database: &str) -> Result<DatabaseEndpoint> {
        self.admin.with_database(database)
    }

    pub(crate) async fn pool(&self, database: &str) -> Result<PgPool> {
        let endpoint = self.endpoint(database)?;
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&endpoint.connection_url())
            .await
            .context("PostgreSQL connection failed")
    }

    pub(crate) async fn repository(&self, database: &str) -> Result<PostgresUpgradeRepository> {
        Ok(PostgresUpgradeRepository::new(self.pool(database).await?))
    }

    pub(crate) async fn repository_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<PostgresUpgradeRepository> {
        let database = match self.load_operation_state(operation_id) {
            Ok(state) if state.switched && !state.recovered => state
                .candidate_database
                .context("switched operation has no Candidate database")?,
            Ok(state) => state.source_database,
            Err(_) => self.active_database.clone(),
        };
        self.repository(&database).await
    }

    pub(crate) async fn acquire_backend_lock(&self) -> Result<PostgresAdvisoryLock> {
        PostgresAdvisoryLock::acquire(&self.endpoint(&self.active_database)?.connection_url())
            .await
            .context("PostgreSQL advisory lock was not acquired")
    }

    pub(crate) fn operation_root(&self, operation_id: Uuid) -> PathBuf {
        self.state_root()
            .join("physical-driver")
            .join(operation_id.to_string())
    }

    pub(crate) fn create_operation_state(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<DriverOperationState> {
        let root = self.operation_root(snapshot.operation_id);
        if root.exists() || root.is_symlink() {
            let state = self.load_operation_state(snapshot.operation_id)?;
            anyhow::ensure!(
                state.source_generation_id == snapshot.source_generation_id,
                "existing operation state belongs to another source generation"
            );
            return Ok(state);
        }
        fs::create_dir_all(&root).context("operation root could not be created")?;
        set_mode(&root, 0o700)?;
        let state = DriverOperationState::new(
            snapshot,
            self.active_database.clone(),
            self.receipt.release_path.clone(),
            self.bundle.clone(),
        );
        self.save_operation_state(&state)?;
        Ok(state)
    }

    pub(crate) fn load_operation_state(&self, operation_id: Uuid) -> Result<DriverOperationState> {
        let path = self.operation_root(operation_id).join("state.json");
        let metadata = fs::symlink_metadata(&path).context("driver operation state is missing")?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "driver operation state is not a regular file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            anyhow::ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "driver operation state permissions are too broad"
            );
        }
        let state: DriverOperationState = serde_json::from_slice(&fs::read(path)?)?;
        state.validate()?;
        anyhow::ensure!(
            state.operation_id == operation_id && state.format_version == DRIVER_STATE_FORMAT,
            "driver operation state identity differs"
        );
        Ok(state)
    }

    pub(crate) fn save_operation_state(&self, state: &DriverOperationState) -> Result<()> {
        state.validate()?;
        let root = self.operation_root(state.operation_id);
        require_real_directory(&root, "operation root")?;
        let path = root.join("state.json");
        write_json_atomic(&path, state, 0o600)
    }

    pub(crate) fn service_controller(
        &self,
    ) -> Result<ServerServiceController<ProcessCommandRunner>> {
        ServerServiceController::new(self.config.clone(), ProcessCommandRunner)
            .context("service controller policy is invalid")
    }

    pub(crate) fn compose_command(&self) -> Result<Command> {
        anyhow::ensure!(
            self.profile() == DeploymentProfile::ManagedCompose,
            "Compose command requested for non-Compose profile"
        );
        let mut command = Command::new("/usr/bin/docker");
        command.args(compose_prefix(&self.config)?);
        Ok(command)
    }

    pub(crate) fn compose_server_container(&self) -> Result<String> {
        let mut command = self.compose_command()?;
        command.args(["ps", "--all", "--quiet", "server"]);
        let output = safe_output(&mut command)?;
        let value = String::from_utf8(output.stdout)?;
        let id = value.trim();
        anyhow::ensure!(
            !id.is_empty() && !id.contains(char::is_whitespace),
            "Server container is missing"
        );
        Ok(id.to_owned())
    }

    pub(crate) fn target_artifact_path(&self) -> Result<PathBuf> {
        required_regular_file_env("MURIARCCTL_TARGET_ARTIFACT", false)
    }

    pub(crate) fn backup_recipient_file(&self) -> Result<PathBuf> {
        required_regular_file_env("MURIARCCTL_BACKUP_RECIPIENT_FILE", false)
    }

    pub(crate) fn backup_identity_file(&self) -> Result<PathBuf> {
        required_regular_file_env("MURIARCCTL_BACKUP_IDENTITY_FILE", true)
    }

    pub(crate) fn pg_dump_executable(&self) -> Result<PathBuf> {
        configured_postgres_executable(
            PG_DUMP_EXECUTABLE_ENV,
            "/usr/lib/postgresql/17/bin/pg_dump",
            "pg_dump executable",
        )
    }

    pub(crate) fn pg_restore_executable(&self) -> Result<PathBuf> {
        configured_postgres_executable(
            PG_RESTORE_EXECUTABLE_ENV,
            "/usr/lib/postgresql/17/bin/pg_restore",
            "pg_restore executable",
        )
    }

    pub(crate) fn age_executable(&self) -> Result<PathBuf> {
        let path = env::var_os("MURIARCCTL_AGE_EXECUTABLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/bin/age"));
        require_executable(&path, "age executable")?;
        Ok(path)
    }

    pub(crate) fn generation_root(&self, generation_id: Uuid) -> PathBuf {
        self.config
            .paths
            .data_root
            .join("generations")
            .join(generation_id.to_string())
    }

    pub(crate) fn environment_backup_path(&self, operation_id: Uuid) -> PathBuf {
        self.operation_root(operation_id).join("source-server.env")
    }

    pub(crate) fn activation_backup_path(&self, operation_id: Uuid) -> PathBuf {
        self.operation_root(operation_id).join("source-active.env")
    }
}

pub(crate) fn read_environment(path: &Path) -> Result<Environment> {
    let metadata = fs::symlink_metadata(path).context("environment file is unavailable")?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "environment input must be a regular file"
    );
    let mut values = BTreeMap::new();
    for (line_number, line) in fs::read_to_string(path)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .with_context(|| format!("environment line {} is malformed", line_number + 1))?;
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
                && !values.contains_key(name),
            "environment name is invalid or duplicated"
        );
        values.insert(name.to_owned(), value.to_owned());
    }
    anyhow::ensure!(!values.is_empty(), "environment file is empty");
    Ok(values)
}

pub(crate) fn write_environment(path: &Path, values: &Environment) -> Result<()> {
    anyhow::ensure!(!values.is_empty(), "environment output is empty");
    let mut bytes = Vec::new();
    for (name, value) in values {
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
                && !value.contains(['\r', '\n']),
            "environment output contains an unsafe value"
        );
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'=');
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(b'\n');
    }
    write_bytes_atomic(path, &bytes, 0o600)
}

pub(crate) fn write_json_atomic(
    path: &Path,
    value: &impl serde::Serialize,
    mode: u32,
) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes, mode)
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("state path has no parent")?;
    require_real_directory(parent, "state parent")?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "refusing to replace non-regular state"
        );
    }
    let temporary = parent.join(format!(".driver-tmp-{}", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    let mut output = options.open(&temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("{label} is unavailable"))?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a real directory"
    );
    Ok(())
}

pub(crate) fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("{label} is unavailable"))?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() > 0,
        "{label} must be a non-empty regular file"
    );
    Ok(())
}

pub(crate) fn require_executable(path: &Path, label: &str) -> Result<()> {
    require_regular_file(path, label)?;
    anyhow::ensure!(path.is_absolute(), "{label} must be absolute");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        anyhow::ensure!(
            fs::metadata(path)?.permissions().mode() & 0o111 != 0,
            "{label} is not executable"
        );
    }
    Ok(())
}

pub(crate) fn safe_output(command: &mut Command) -> Result<Output> {
    command.stdin(Stdio::null()).stderr(Stdio::null());
    let output = command.output().context("required command could not run")?;
    anyhow::ensure!(output.status.success(), "required command failed");
    Ok(output)
}

pub(crate) fn safe_status(command: &mut Command) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = command.status().context("required command could not run")?;
    anyhow::ensure!(status.success(), "required command failed");
    Ok(())
}

pub(crate) fn require_database_name(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 63
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            && value.as_bytes()[0].is_ascii_lowercase(),
        "database name is invalid"
    );
    Ok(())
}

pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn state_root() -> PathBuf {
    env::var_os(STATE_ROOT_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os(INSTALL_ROOT_ENV)
                .map(PathBuf::from)
                .map(|root| root.join("control"))
        })
        .unwrap_or_else(|| PathBuf::from("/var/lib/muriarc/control"))
}

fn compose_database_host(config: &DeliveryConfig) -> Result<String> {
    let mut command = Command::new("/usr/bin/docker");
    command.args(compose_prefix(config)?);
    command.args(["ps", "--quiet", "db"]);
    let output = safe_output(&mut command)?;
    let container = String::from_utf8(output.stdout)?;
    let container = container.trim();
    anyhow::ensure!(!container.is_empty(), "PostgreSQL container is unavailable");
    let output = safe_output(
        Command::new("/usr/bin/docker")
            .args(["inspect", "--format"])
            .arg("{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}")
            .arg(container),
    )?;
    let host = String::from_utf8(output.stdout)?;
    let host = host.trim();
    anyhow::ensure!(
        host.parse::<std::net::IpAddr>().is_ok(),
        "PostgreSQL container address is invalid"
    );
    Ok(host.to_owned())
}

fn compose_prefix(config: &DeliveryConfig) -> Result<Vec<String>> {
    let project = config
        .compose_project
        .as_deref()
        .context("Compose project is missing")?;
    let file = config
        .compose_file
        .as_ref()
        .context("Compose file is missing")?;
    Ok(vec![
        "compose".to_owned(),
        "--project-name".to_owned(),
        project.to_owned(),
        "--env-file".to_owned(),
        config.environment_file.display().to_string(),
        "--env-file".to_owned(),
        config.activation_file.display().to_string(),
        "--file".to_owned(),
        file.display().to_string(),
    ])
}

fn configured_executable(name: &'static str, default: &str, label: &str) -> Result<PathBuf> {
    let path = env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));
    require_executable(&path, label)?;
    Ok(path)
}

fn configured_postgres_executable(
    name: &'static str,
    default: &str,
    label: &str,
) -> Result<PathBuf> {
    let path = configured_executable(name, default, label)?;
    let output = Command::new(&path)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .with_context(|| format!("{label} version could not be checked"))?;
    anyhow::ensure!(output.status.success(), "{label} version check failed");
    let major = String::from_utf8(output.stdout)?
        .split_whitespace()
        .find_map(|part| part.split('.').next()?.parse::<u16>().ok());
    anyhow::ensure!(
        major.is_some_and(|value| value >= 17),
        "{label} is older than PostgreSQL 17"
    );
    Ok(path)
}

fn required_env(name: &'static str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("required environment variable {name} is missing"))
}

fn required_regular_file_env(name: &'static str, owner_only: bool) -> Result<PathBuf> {
    let path = PathBuf::from(required_env(name)?);
    anyhow::ensure!(path.is_absolute(), "required file path must be absolute");
    require_regular_file(&path, "required control file")?;
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::PermissionsExt as _;
        anyhow::ensure!(
            fs::metadata(&path)?.permissions().mode() & 0o077 == 0,
            "private control file permissions are too broad"
        );
    }
    Ok(path)
}

fn required_value<'a>(values: &'a Environment, name: &str) -> Result<&'a str> {
    values
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("required protected environment value {name} is missing"))
}

fn database_name_from_url(url: &Url) -> Option<&str> {
    url.path_segments()?.find(|segment| !segment.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_endpoints_change_only_the_database() {
        let endpoint = DatabaseEndpoint::parse(
            "postgresql://user:p%40ss@127.0.0.1:5432/source?sslmode=disable",
        )
        .unwrap();
        let candidate = endpoint.with_database("muriarc_candidate_123").unwrap();
        assert_eq!(candidate.database_name().unwrap(), "muriarc_candidate_123");
        assert_eq!(candidate.username(), "user");
        assert_eq!(candidate.password(), "p@ss");
        assert_eq!(candidate.host(), "127.0.0.1");
        assert_eq!(candidate.port(), 5432);
    }

    #[test]
    fn database_names_are_strict_identifiers() {
        assert!(require_database_name("muriarc_candidate_123").is_ok());
        assert!(require_database_name("MuriArc").is_err());
        assert!(require_database_name("muriarc;drop database").is_err());
    }
}
