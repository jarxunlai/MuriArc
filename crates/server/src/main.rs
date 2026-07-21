use std::{env, error::Error, net::SocketAddr, path::PathBuf, sync::Arc};

use chrono::Duration;
use muriarc_core::{AiOperationStore, AiScope, LabRole, MuriArcStore, WriteSource};
use muriarc_data::DataFiles;
use muriarc_server::{
    AiMasterKey, AppState, Authenticator, ChainedAuthenticator, EnvironmentRootConfig,
    LiveBootstrapAuthenticator, PostgresAiProviderStore, PostgresAuthBackend,
    PostgresUserGovernance, SessionCookieConfig, StaticTokenAuthenticator, StoreJobRepository,
    application_router, sync_postgres_environment_root,
};
use muriarc_store_postgres::PostgresStore;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zeroize::Zeroizing;

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

    let store = Arc::new(PostgresStore::connect(&database_url).await?);
    store.migrate().await?;

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
    ));
    let state = configure_ai(state, store.clone())?;
    let data_root = PathBuf::from(required_env("MURIARC_DATA_ROOT")?);
    let attachment_root = PathBuf::from(required_env("MURIARC_ATTACHMENT_ROOT")?);
    tokio::fs::create_dir_all(&data_root).await?;
    tokio::fs::create_dir_all(&attachment_root).await?;
    let state = state.with_data_storage(DataFiles::new(data_root), attachment_root);
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    let ui_dir = env::var_os("MURIARC_UI_DIR").map(PathBuf::from);
    if ui_dir.is_none() {
        tracing::warn!("MURIARC_UI_DIR is not set; shared Web UI will not be served");
    }

    tracing::info!(address = %bind_address, "MuriArc server listening");
    axum::serve(listener, application_router(state, ui_dir)).await?;
    Ok(())
}

fn configure_ai(state: AppState, store: Arc<PostgresStore>) -> Result<AppState, Box<dyn Error>> {
    let Some(encoded_key) = optional_secret_env("MURIARC_AI_MASTER_KEY")? else {
        tracing::warn!(
            "MURIARC_AI_MASTER_KEY is not set; shared AI settings, turns and approvals are disabled"
        );
        return Ok(state);
    };
    let encoded_key = Zeroizing::new(encoded_key);
    let key_version = optional_positive_i32_env("MURIARC_AI_MASTER_KEY_VERSION", 1)?;
    let master_key = AiMasterKey::from_base64(encoded_key.as_str(), key_version)?;
    let providers = Arc::new(PostgresAiProviderStore::new(store.as_ref().clone(), master_key));
    let operations: Arc<dyn AiOperationStore> = store;
    tracing::info!(
        key_version,
        "shared AI transport is enabled with encrypted per-user credentials"
    );
    Ok(state.with_ai(operations, providers))
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
