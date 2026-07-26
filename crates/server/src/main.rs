use std::{
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{ErrorKind, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose};
use chrono::Duration;
use muriarc_core::{
    AiModelProfileStore, AiOperationStore, AiScope, LabRole, MuriArcStore,
    PersistentRecoveryInventory, WriteSource,
};
use muriarc_data::{AttachmentFiles, DataFiles, cleanup_expired_ai_conversation_sources};
use muriarc_server::{
    AiMasterKey, AppState, Authenticator, ChainedAuthenticator, EnvironmentRootConfig,
    LiveBootstrapAuthenticator, PostgresAiProviderStore, PostgresAuthBackend,
    PostgresTechnicalLogService, PostgresUserGovernance, SessionCookieConfig,
    StaticTokenAuthenticator, StoreJobRepository, application_router,
    sync_postgres_environment_root,
};
use muriarc_store_postgres::PostgresStore;
use rand::{RngCore, rngs::OsRng};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zeroize::Zeroizing;

mod runtime_compatibility;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("muriarc_server=info,tower_http=info")),
        )
        .init();

    let database_url = required_env("MURIARC_DATABASE_URL")?;
    let bind_address = env::var("MURIARC_BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_owned())
        .parse::<SocketAddr>()?;
    let server_lab_id = required_env("MURIARC_LAB_ID")?.parse::<Uuid>()?;

    let data_root = PathBuf::from(required_env("MURIARC_DATA_ROOT")?);
    let attachment_root = PathBuf::from(required_env("MURIARC_ATTACHMENT_ROOT")?);
    let store = Arc::new(PostgresStore::connect(&database_url).await?);
    let runtime =
        runtime_compatibility::prepare_server_runtime(store.as_ref(), &data_root, &attachment_root)
            .await?;

    let root_config = environment_root_config(server_lab_id)?;
    let root_outcome = sync_postgres_environment_root(store.as_ref(), &root_config).await?;
    if root_outcome.changed() {
        tracing::warn!(
            lab_created = root_outcome.lab_created,
            lab_updated = root_outcome.lab_updated,
            user_created = root_outcome.user_created,
            user_updated = root_outcome.user_updated,
            membership_created = root_outcome.membership_created,
            membership_updated = root_outcome.membership_updated,
            credential_created = root_outcome.credential_created,
            credential_updated = root_outcome.credential_updated,
            sessions_revoked = root_outcome.sessions_revoked,
            "environment root configuration was reconciled"
        );
    } else {
        tracing::info!("environment root configuration is already synchronized");
    }
    let bootstrap_authenticator = bootstrap_authenticator(&root_config)?;
    let environment_root_user_id = root_config.user_id;
    drop(root_config);
    let persistent_auth = Arc::new(PostgresAuthBackend::new(
        store.as_ref().clone(),
        store.clone(),
        server_lab_id,
        environment_root_user_id,
    )?);
    let authenticators: Vec<Arc<dyn Authenticator>> = vec![
        Arc::new(LiveBootstrapAuthenticator::new(
            bootstrap_authenticator,
            persistent_auth.as_ref().clone(),
        )),
        persistent_auth.clone(),
    ];
    let authenticator = ChainedAuthenticator::new(authenticators);
    let state = AppState::new(
        store.clone(),
        Arc::new(authenticator),
        Arc::new(StoreJobRepository::new(store.clone())),
    )
    .with_sessions(persistent_auth, session_cookie_config()?)
    .with_user_governance(PostgresUserGovernance::new(
        store.as_ref().clone(),
        server_lab_id,
        environment_root_user_id,
    ))
    .with_technical_logs(Arc::new(PostgresTechnicalLogService::new(
        store.as_ref().clone(),
    )));
    let state = configure_ai(
        state,
        store.clone(),
        &data_root,
        &runtime.recovery_inventory,
    )
    .await?;
    let state = state.with_data_storage(DataFiles::new(data_root), attachment_root.clone());
    let cleanup_store: Arc<dyn MuriArcStore> = store.clone();
    let _ai_source_cleanup = spawn_ai_source_cleanup(cleanup_store, server_lab_id, attachment_root);
    let ui_dir = env::var_os("MURIARC_UI_DIR").map(PathBuf::from);
    if ui_dir.is_none() {
        tracing::warn!("MURIARC_UI_DIR is not set; shared Web UI will not be served");
    }
    let state = state.with_runtime_compatibility(ui_dir.clone());
    let listener = tokio::net::TcpListener::bind(bind_address).await?;

    tracing::info!(address = %bind_address, "MuriArc server listening");
    axum::serve(listener, application_router(state, ui_dir)).await?;
    Ok(())
}

fn spawn_ai_source_cleanup(
    store: Arc<dyn MuriArcStore>,
    lab_id: Uuid,
    attachment_root: PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let files = AttachmentFiles::new(attachment_root);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            // Tokio's first interval tick is immediate. Subsequent bounded
            // sweeps run hourly and never overlap.
            interval.tick().await;
            match cleanup_expired_ai_conversation_sources(
                store.as_ref(),
                &files,
                lab_id,
                chrono::Utc::now(),
                muriarc_core::MAX_AI_CONVERSATION_SOURCE_CLEANUP_BATCH,
                WriteSource::Web,
            )
            .await
            {
                Ok(report) => tracing::info!(
                    inspected = report.inspected,
                    discarded = report.discarded,
                    cleaned = report.cleaned,
                    conflicts = report.conflicts,
                    store_failures = report.store_failures,
                    object_failures = report.object_failures,
                    "completed bounded AI conversation source retention sweep"
                ),
                Err(error) => tracing::warn!(
                    error = %error,
                    "AI conversation source retention sweep could not list candidates"
                ),
            }
        }
    })
}

async fn configure_ai(
    state: AppState,
    store: Arc<PostgresStore>,
    data_root: &Path,
    recovery_inventory: &PersistentRecoveryInventory,
) -> Result<AppState, Box<dyn Error>> {
    let encoded_key = match optional_secret_env("MURIARC_AI_MASTER_KEY")? {
        Some(value) => Zeroizing::new(value),
        None => {
            let path = env::var_os("MURIARC_AI_MASTER_KEY_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| data_root.join("secrets/ai-master-key"));
            Zeroizing::new(load_or_create_ai_master_key_file(
                &path,
                recovery_inventory.encrypted_secret_records == 0,
            )?)
        }
    };
    let key_version = optional_positive_i32_env("MURIARC_AI_MASTER_KEY_VERSION", 1)?;
    let master_key = AiMasterKey::from_base64(encoded_key.as_str(), key_version)?;
    let providers = Arc::new(PostgresAiProviderStore::new(
        store.as_ref().clone(),
        master_key,
    ));
    let operations: Arc<dyn AiOperationStore> = store.clone();
    let model_profiles: Arc<dyn AiModelProfileStore> = store;
    tracing::info!(
        key_version,
        "shared AI runtime is enabled with profile-bound encrypted credentials; startup performed no data migration"
    );
    Ok(state.with_ai(operations, model_profiles, providers))
}

fn load_or_create_ai_master_key_file(
    path: &Path,
    allow_create: bool,
) -> Result<String, Box<dyn Error>> {
    match fs::read_to_string(path) {
        Ok(value) => return validated_master_key_file(value),
        Err(error) if error.kind() == ErrorKind::NotFound && allow_create => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(format!(
                "AI ciphertext exists but the deployment Master Key is missing: {}",
                path.display()
            )
            .into());
        }
        Err(error) => return Err(error.into()),
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let encoded = general_purpose::STANDARD.encode(bytes);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(encoded.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            tracing::warn!(
                path = %path.display(),
                "generated the stable deployment AI master key file; protect and back it up"
            );
            Ok(encoded)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let mut value = String::new();
            OpenOptions::new()
                .read(true)
                .open(path)?
                .read_to_string(&mut value)?;
            validated_master_key_file(value)
        }
        Err(error) => Err(error.into()),
    }
}

fn validated_master_key_file(value: String) -> Result<String, Box<dyn Error>> {
    let value = value.trim().to_owned();
    AiMasterKey::from_base64(&value, 1)?;
    Ok(value)
}

fn bootstrap_authenticator(
    root: &EnvironmentRootConfig,
) -> Result<StaticTokenAuthenticator, Box<dyn Error>> {
    let human_token = optional_secret_env("MURIARC_BOOTSTRAP_TOKEN")?;
    let mcp_token = optional_secret_env("MURIARC_BOOTSTRAP_MCP_TOKEN")?;
    if human_token.is_none() && mcp_token.is_none() {
        tracing::info!(
            "bootstrap bearer preview is disabled; persistent authentication remains available"
        );
        return Ok(StaticTokenAuthenticator::default());
    }

    if human_token.as_ref().is_some_and(|token| token.len() < 32) {
        return Err("MURIARC_BOOTSTRAP_TOKEN must contain at least 32 characters".into());
    }
    if mcp_token.as_ref().is_some_and(|token| token.len() < 32) {
        return Err("MURIARC_BOOTSTRAP_MCP_TOKEN must contain at least 32 characters".into());
    }
    if human_token.is_some() && human_token == mcp_token {
        return Err(
            "MURIARC_BOOTSTRAP_TOKEN and MURIARC_BOOTSTRAP_MCP_TOKEN must be different".into(),
        );
    }

    let lab_id = root.lab_id;
    let user_id = root.user_id;
    let display_name = root.user_display_name.clone();
    let mut entries = Vec::with_capacity(2);
    if let Some(token) = human_token {
        let principal = muriarc_server::AuthPrincipal::human(
            user_id,
            display_name.clone(),
            lab_id,
            [LabRole::LabAdmin],
        )
        .with_source(WriteSource::Api);
        entries.push((token, principal));
    }
    if let Some(token) = mcp_token {
        let principal = muriarc_server::AuthPrincipal::human(
            user_id,
            display_name,
            lab_id,
            [LabRole::LabAdmin],
        )
        .with_ai_scopes([AiScope::Read])
        .with_source(WriteSource::Mcp);
        entries.push((token, principal));
    }

    tracing::warn!(
        "bootstrap authentication is enabled for controlled preview only; replace it with persistent accounts and external tokens before production"
    );
    Ok(StaticTokenAuthenticator::new(entries)?)
}

fn environment_root_config(server_lab_id: Uuid) -> Result<EnvironmentRootConfig, Box<dyn Error>> {
    EnvironmentRootConfig::new(
        server_lab_id,
        required_env("MURIARC_LAB_NAME")?,
        required_env("MURIARC_ROOT_USER_ID")?.parse::<Uuid>()?,
        required_env("MURIARC_ROOT_USER_EMAIL")?,
        required_env("MURIARC_ROOT_USER_NAME")?,
        required_env("MURIARC_ROOT_PASSWORD")?,
    )
    .map_err(Into::into)
}

fn session_cookie_config() -> Result<SessionCookieConfig, Box<dyn Error>> {
    let secure = optional_bool_env("MURIARC_SESSION_COOKIE_SECURE", true)?;
    let ttl_hours = match env::var("MURIARC_SESSION_TTL_HOURS") {
        Ok(value) => value.parse::<i64>()?,
        Err(env::VarError::NotPresent) => 12,
        Err(error) => return Err(error.into()),
    };
    if !(1..=720).contains(&ttl_hours) {
        return Err("MURIARC_SESSION_TTL_HOURS must be between 1 and 720".into());
    }
    Ok(SessionCookieConfig::new(
        secure,
        Duration::hours(ttl_hours),
    )?)
}

fn optional_bool_env(name: &'static str, default: bool) -> Result<bool, Box<dyn Error>> {
    match env::var(name) {
        Err(env::VarError::NotPresent) => Ok(default),
        Ok(value) if matches!(value.as_str(), "true" | "1") => Ok(true),
        Ok(value) if matches!(value.as_str(), "false" | "0") => Ok(false),
        Err(error) => Err(error.into()),
        Ok(_) => Err(format!("{name} must be true, false, 1, or 0").into()),
    }
}

fn optional_positive_i32_env(name: &'static str, default: i32) -> Result<i32, Box<dyn Error>> {
    let value = match env::var(name) {
        Ok(value) => value.parse::<i32>()?,
        Err(env::VarError::NotPresent) => default,
        Err(error) => return Err(error.into()),
    };
    if value <= 0 {
        return Err(format!("{name} must be a positive integer").into());
    }
    Ok(value)
}

fn optional_secret_env(name: &'static str) -> Result<Option<String>, Box<dyn Error>> {
    match env::var(name) {
        Err(env::VarError::NotPresent) => Ok(None),
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(error) => Err(error.into()),
    }
}

fn required_env(name: &'static str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is not set").into())
}

#[cfg(test)]
mod ai_runtime_tests {
    use super::*;

    #[test]
    fn generated_master_key_is_stable_across_restarts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets/ai-master-key");
        let first = load_or_create_ai_master_key_file(&path, true).unwrap();
        let second = load_or_create_ai_master_key_file(&path, true).unwrap();
        assert_eq!(first, second);
        assert_eq!(general_purpose::STANDARD.decode(first).unwrap().len(), 32);
    }

    #[test]
    fn compose_starts_only_server_and_postgres_without_local_models() {
        let compose = include_str!("../../../docker-compose.yml").to_ascii_lowercase();
        assert!(compose.contains("  db:"));
        assert!(compose.contains("  server:"));
        assert!(!compose.contains("ollama"));
        assert!(!compose.contains("model pull"));
    }
}
