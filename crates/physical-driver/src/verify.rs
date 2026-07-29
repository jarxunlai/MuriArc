use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration as StdDuration,
};

use anyhow::{Context as _, Result};
use chrono::Utc;
use muriarc_core::{AuditContext, Cage, MuriArcStore, WriteSource};
use muriarc_store_postgres::PostgresStore;
use muriarc_upgrade::{
    ActivationVerificationEvidence, DeploymentProfile, UpgradeSnapshot, VerificationEvidence,
    VerificationLayer, VerificationLayerEvidence,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    backup::{
        database_dump_digest, digest_bytes, drop_isolated_database, recreate_database,
        verify_candidate_assets,
    },
    context::{
        DriverContext, require_executable, require_real_directory, safe_output, safe_status,
        set_mode,
    },
    model::DriverOperationState,
};

pub(crate) async fn verify_candidate(
    context: &DriverContext,
    state: &DriverOperationState,
    snapshot: &UpgradeSnapshot,
) -> Result<VerificationEvidence> {
    let generation_id = state
        .candidate_generation_id
        .context("Candidate generation is missing")?;
    let database = state
        .candidate_database
        .as_deref()
        .context("Candidate database is missing")?;
    let root = state
        .candidate_root
        .as_ref()
        .context("Candidate generation root is missing")?;
    require_real_directory(root, "Candidate generation root")?;
    anyhow::ensure!(
        snapshot.candidate_generation_id == Some(generation_id),
        "Candidate generation differs from upgrade snapshot"
    );

    let mut layers = BTreeMap::new();
    let now = Utc::now();
    let asset_digest = verify_candidate_assets(state, root)?;
    insert_layer(
        &mut layers,
        VerificationLayer::AssetRestore,
        asset_digest,
        now,
    );

    let store = PostgresStore::connect(&context.endpoint(database)?.connection_url()).await?;
    let compatibility = store.compatibility_report().await?;
    compatibility
        .require_read_only_compatible()
        .map_err(|message| anyhow::anyhow!(message))?;
    insert_layer(
        &mut layers,
        VerificationLayer::Storage,
        digest_value(&compatibility)?,
        Utc::now(),
    );
    store.health_check().await?;
    let persistent = store.persistent_recovery_inventory().await?;
    insert_layer(
        &mut layers,
        VerificationLayer::StoreApplication,
        digest_value(&persistent)?,
        Utc::now(),
    );
    store.pool().close().await;

    let verify_database = format!("muriarc_verify_{}", snapshot.operation_id.simple());
    recreate_database(context, &verify_database, Some(database)).await?;
    let continue_write =
        verify_continue_write(context, &verify_database, snapshot.operation_id).await;
    let cleanup = drop_isolated_database(context, &verify_database).await;
    let continue_write = match (continue_write, cleanup) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(cleanup)) => {
            anyhow::bail!("continue-write verification database cleanup failed: {cleanup}")
        }
        (Err(error), Err(cleanup)) => anyhow::bail!(
            "continue-write verification failed: {error}; database cleanup also failed: {cleanup}"
        ),
    };
    insert_layer(
        &mut layers,
        VerificationLayer::ContinueWrite,
        continue_write,
        Utc::now(),
    );

    let before = database_dump_digest(context, database, "candidate-before")?;
    let runtime = CandidateRuntime::start(context, state, database, root).await?;
    let probes = probe_server(runtime.origin()).await?;
    insert_layer(
        &mut layers,
        VerificationLayer::Api,
        digest_bytes(&serde_json::to_vec(&json!({
            "health": probes.health,
            "compatibility": probes.compatibility,
        }))?),
        Utc::now(),
    );
    insert_layer(
        &mut layers,
        VerificationLayer::RemoteUi,
        digest_bytes(probes.ui.as_bytes()),
        Utc::now(),
    );
    drop(runtime);
    let after = database_dump_digest(context, database, "candidate-after")?;
    anyhow::ensure!(
        before == after,
        "read-only Candidate verification changed database state"
    );
    insert_layer(
        &mut layers,
        VerificationLayer::ReadOnlyNoSideEffects,
        digest_bytes(format!("{before}\n{after}").as_bytes()),
        Utc::now(),
    );

    let evidence = VerificationEvidence {
        generation_id,
        layers,
    };
    evidence.validate()?;
    Ok(evidence)
}

pub(crate) async fn verify_activated_service(
    context: &DriverContext,
    state: &DriverOperationState,
) -> Result<ActivationVerificationEvidence> {
    let generation_id = state
        .candidate_generation_id
        .context("activated Candidate generation is missing")?;
    let database = state
        .candidate_database
        .as_deref()
        .context("activated Candidate database is missing")?;
    let before = database_dump_digest(context, database, "activated-before")?;
    let probes = probe_server("http://127.0.0.1:8787").await?;
    anyhow::ensure!(
        probes.health.get("status").is_some() || !probes.health.is_null(),
        "activated API health response is invalid"
    );
    let after = database_dump_digest(context, database, "activated-after")?;
    anyhow::ensure!(
        before == after,
        "read-only activation changed database state"
    );
    Ok(ActivationVerificationEvidence {
        generation_id,
        readiness_verified: true,
        compatibility_verified: !probes.compatibility.is_null(),
        no_write_side_effects: true,
        verified_at: Utc::now(),
    })
}

pub(crate) async fn probe_current_read_only(
    context: &DriverContext,
    database: &str,
) -> Result<(String, String, String)> {
    let before = database_dump_digest(context, database, "readonly-before")?;
    let probes = probe_server("http://127.0.0.1:8787").await?;
    let after = database_dump_digest(context, database, "readonly-after")?;
    anyhow::ensure!(before == after, "read-only command changed database state");
    let evidence = digest_bytes(&serde_json::to_vec(&json!({
        "health": probes.health,
        "compatibility": probes.compatibility,
        "ui": digest_bytes(probes.ui.as_bytes()),
        "state_before": before,
        "state_after": after,
    }))?);
    Ok((before, after, evidence))
}

async fn verify_continue_write(
    context: &DriverContext,
    database: &str,
    operation_id: Uuid,
) -> Result<String> {
    let store = PostgresStore::connect(&context.endpoint(database)?.connection_url()).await?;
    let lab_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM labs WHERE deleted_at IS NULL ORDER BY created_at, id LIMIT 1",
    )
    .fetch_one(store.pool())
    .await?;
    let now = Utc::now();
    let cage = Cage::new(
        lab_id,
        "MuriArc upgrade verification",
        format!("VERIFY-{}", &operation_id.simple().to_string()[..12]),
        now,
    )?;
    let mut audit = AuditContext::system(WriteSource::Migration);
    audit.request_id = Some(format!("upgrade-verification-{operation_id}"));
    audit.reason = Some("isolated Candidate continue-write verification".to_owned());
    store.create_cage(&cage, &audit).await?;
    let observed = store.get_cage(cage.id).await?;
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_entries WHERE entity_type = 'cage' AND entity_id = $1 AND request_id = $2",
    )
    .bind(cage.id)
    .bind(audit.request_id.as_deref())
    .fetch_one(store.pool())
    .await?;
    let provenance_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provenance WHERE entity_type = 'cage' AND entity_id = $1 AND request_id = $2",
    )
    .bind(cage.id)
    .bind(audit.request_id.as_deref())
    .fetch_one(store.pool())
    .await?;
    anyhow::ensure!(
        observed == cage && audit_count == 1 && provenance_count == 1,
        "isolated Store/Application write lacked exact Audit or Provenance"
    );
    let digest = digest_value(&json!({
        "cage_id": cage.id,
        "revision": cage.meta.revision,
        "audit_count": audit_count,
        "provenance_count": provenance_count,
    }))?;
    store.pool().close().await;
    Ok(digest)
}

struct CandidateRuntime {
    child: Child,
    origin: String,
}

impl CandidateRuntime {
    async fn start(
        context: &DriverContext,
        state: &DriverOperationState,
        database: &str,
        root: &Path,
    ) -> Result<Self> {
        let (server, ui) = final_runtime(context, state)?;
        require_executable(&server, "final Candidate Server")?;
        require_real_directory(&ui, "final Candidate UI")?;
        let port = available_loopback_port()?;
        let origin = format!("http://127.0.0.1:{port}");
        let generation_id = state
            .candidate_generation_id
            .context("Candidate generation is missing")?;
        let mut command = Command::new(server);
        command
            .env_clear()
            .envs(context.environment.iter())
            .env(
                "MURIARC_DATABASE_URL",
                context.endpoint(database)?.connection_url(),
            )
            .env("MURIARC_BIND_ADDR", format!("127.0.0.1:{port}"))
            .env("MURIARC_UI_DIR", &ui)
            .env("MURIARC_ACTIVE_GENERATION", generation_id.to_string())
            .env("MURIARC_DATA_ROOT", root.join("data"))
            .env("MURIARC_ATTACHMENT_ROOT", root.join("attachments"))
            .env(
                "MURIARC_AI_MASTER_KEY_FILE",
                root.join("secrets/ai-master-key"),
            )
            .env("MURIARC_ACTIVATION_MODE", "read-only")
            .env("MURIARC_PREVIEW_BOOTSTRAP", "false")
            .env("MURIARC_EXTERNAL_API_ENABLED", "false")
            .env("MURIARC_CANDIDATE_EXTERNAL_PROVIDERS_DISABLED", "true")
            .env("MURIARC_CANDIDATE_BACKGROUND_JOBS_DISABLED", "true")
            .env("MURIARC_CANDIDATE_REAL_USER_WRITES_DISABLED", "true")
            .env("RUST_LOG", "off")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .context("final Candidate Server could not start")?;
        let runtime = Self { child, origin };
        wait_until_ready(runtime.origin()).await?;
        Ok(runtime)
    }

    fn origin(&self) -> &str {
        &self.origin
    }
}

impl Drop for CandidateRuntime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ProbeBodies {
    health: serde_json::Value,
    compatibility: serde_json::Value,
    ui: String,
}

async fn probe_server(origin: &str) -> Result<ProbeBodies> {
    let client = reqwest::Client::builder()
        .timeout(StdDuration::from_secs(10))
        .build()?;
    let ready = client.get(format!("{origin}/readyz")).send().await?;
    anyhow::ensure!(ready.status().is_success(), "Server readiness probe failed");
    let health = json_body(&client, format!("{origin}/api/v1/health")).await?;
    let compatibility = json_body(&client, format!("{origin}/api/v1/compatibility")).await?;
    let response = client.get(format!("{origin}/")).send().await?;
    anyhow::ensure!(response.status().is_success(), "Remote UI probe failed");
    let ui = response.text().await?;
    anyhow::ensure!(
        ui.len() <= 2 * 1024 * 1024 && ui.contains("<!doctype html"),
        "Remote UI response is invalid"
    );
    Ok(ProbeBodies {
        health,
        compatibility,
        ui,
    })
}

async fn json_body(client: &reqwest::Client, url: String) -> Result<serde_json::Value> {
    let response = client.get(url).send().await?;
    anyhow::ensure!(
        response.status().is_success(),
        "API verification probe failed"
    );
    let bytes = response.bytes().await?;
    anyhow::ensure!(
        bytes.len() <= 1024 * 1024,
        "API verification response is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

async fn wait_until_ready(origin: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(StdDuration::from_secs(2))
        .build()?;
    for _ in 0..90 {
        if client
            .get(format!("{origin}/readyz"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(StdDuration::from_secs(1)).await;
    }
    anyhow::bail!("final Candidate Server did not become ready")
}

fn final_runtime(
    context: &DriverContext,
    state: &DriverOperationState,
) -> Result<(PathBuf, PathBuf)> {
    match context.profile() {
        DeploymentProfile::NativeSystem => {
            let release = state
                .target_release_path
                .as_ref()
                .context("target release path is missing")?;
            Ok((release.join("bin/muriarc-server"), release.join("ui")))
        }
        DeploymentProfile::ManagedCompose => extract_compose_runtime(context, state),
        DeploymentProfile::Desktop => unreachable!(),
    }
}

fn extract_compose_runtime(
    context: &DriverContext,
    state: &DriverOperationState,
) -> Result<(PathBuf, PathBuf)> {
    let root = context
        .operation_root(state.operation_id)
        .join("final-image-runtime");
    let server = root.join("bin/muriarc-server");
    let ui = root.join("ui");
    if server.exists() && ui.is_dir() {
        require_executable(&server, "extracted final Server")?;
        return Ok((server, ui));
    }
    anyhow::ensure!(
        !root.exists() && !root.is_symlink(),
        "final image runtime is partial"
    );
    fs::create_dir(&root)?;
    set_mode(&root, 0o700)?;
    fs::create_dir(root.join("bin"))?;
    fs::create_dir(&ui)?;
    let image = state
        .target_server_image
        .as_deref()
        .context("target Server image is missing")?;
    let output = safe_output(Command::new("/usr/bin/docker").args(["create", image]))?;
    let container = String::from_utf8(output.stdout)?;
    let container = container.trim().to_owned();
    anyhow::ensure!(
        !container.is_empty(),
        "final image extraction container is missing"
    );
    let result = (|| {
        safe_status(
            Command::new("/usr/bin/docker")
                .args(["cp"])
                .arg(format!("{container}:/usr/local/bin/muriarc-server"))
                .arg(&server),
        )?;
        safe_status(
            Command::new("/usr/bin/docker")
                .args(["cp"])
                .arg(format!("{container}:/opt/muriarc/ui/."))
                .arg(&ui),
        )?;
        require_executable(&server, "extracted final Server")?;
        require_real_directory(&ui, "extracted final UI")?;
        Ok((server, ui))
    })();
    let _ = Command::new("/usr/bin/docker")
        .args(["rm", "--force", &container])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if result.is_err() {
        let _ = fs::remove_dir_all(root);
    }
    result
}

fn available_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn insert_layer(
    layers: &mut BTreeMap<VerificationLayer, VerificationLayerEvidence>,
    layer: VerificationLayer,
    evidence_digest: String,
    verified_at: chrono::DateTime<Utc>,
) {
    layers.insert(
        layer,
        VerificationLayerEvidence {
            evidence_digest,
            verified_at,
        },
    );
}

fn digest_value(value: &impl serde::Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}
