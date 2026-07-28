#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use muriarc_core::{
    Actor, ActorType, AiConversation, AiConversationMessage, AiConversationMessageRole,
    AiModelCredentialState, AiModelProfile, AiModelProfileBinding, AiModelProfileSecretRef,
    AiModelProfileSecretRefStore, AiModelProfileStore, AiModelProfileVersion, AiProviderProtocol,
    AiProviderTransport, Approval, ApprovalDecision, AuditContext, AuditFilter, BreedingMemberRole,
    Job, JobKind, JobStatus, LOCAL_LAB_ID, LOCAL_OPERATOR_NAME, LOCAL_USER_ID, MuriArcStore,
    ParentType, ParticipationFilter, ProvenanceFilter, RecordMeta, ToolRun, ToolRunStatus,
    WriteSource,
};
use muriarc_release_evidence::{
    AccountFact, AiApprovalFact, AiConversationFact, AiHistoryFact, AiJobFact, AiMessageFact,
    AiProfileFact, AiToolRunFact, AnimalFact, AttachmentFact, AuditFact, BreedingFact,
    ContinuationExpectation, CurrentReleaseFixtureProducer, ExpectedFacts, ExperimentFact,
    FIXTURE_FORMAT_VERSION, FixtureComponentKind, FixtureFile, FixtureManifest,
    FixtureProducerProvenance, MeasurementFact, ObservationFact, ProjectFact, ProvenanceFact,
    SampleFact, Sha256Digest, digest_bytes, expected_facts_digest, fixture_manifest_digest,
    load_and_verify_fixture,
};
use muriarc_standard_fixture::{SeedReceipt, seed_standard_v1};
use muriarc_store_sqlite::SqliteStore;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub type ReleaseFixtureResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const EXPECTED_FACTS_FILE: &str = "expected-facts.json";
const CONFIGURATION_FILE: &str = "configuration/recovery-config.json";
const KEYSET_FILE: &str = "keyset/recovery.json";
const AI_STATE_FILE: &str = "ai-state/summary.json";
const FIXTURE_MANIFEST_FILE: &str = "fixture-manifest.json";
const SYNTHETIC_KEYRING_ACCOUNT: &str = "muriarc-release-fixture-profile-v1-api-key";
#[cfg(feature = "postgres")]
const SYNTHETIC_MASTER_KEY_BASE64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";

trait FixtureStore: MuriArcStore + AiModelProfileStore {}
impl<T> FixtureStore for T where T: MuriArcStore + AiModelProfileStore {}

#[derive(Debug)]
struct AiSeed {
    profile: AiModelProfile,
    version: AiModelProfileVersion,
    conversation: AiConversation,
    messages: Vec<AiConversationMessage>,
    tool_run: ToolRun,
    approval: Approval,
    job: Job,
    secret_digests: BTreeSet<Sha256Digest>,
    key_versions: BTreeSet<i32>,
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    fn parse(value: &str) -> ReleaseFixtureResult<Self> {
        match value {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" => Ok(Self::Postgres),
            _ => Err(invalid("--backend must be sqlite or postgres").into()),
        }
    }

    fn core(self) -> muriarc_core::BackendKind {
        match self {
            Self::Sqlite => muriarc_core::BackendKind::Sqlite,
            Self::Postgres => muriarc_core::BackendKind::Postgres,
        }
    }
}

pub async fn run_cli<I>(args: I) -> ReleaseFixtureResult<bool>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| invalid("arguments must be valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if matches!(command, "-h" | "--help" | "help") {
        return Ok(false);
    }
    match command {
        "prepare-sqlite" => {
            let receipt = prepare_sqlite(
                required_path(&args, "--fixture")?,
                required_path(&args, "--output")?,
                required_value(&args, "--source-commit")?,
            )
            .await?;
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "ok": true,
                    "backend": "sqlite",
                    "generation_id": receipt.generation_id,
                }))?
            );
        }
        #[cfg(feature = "postgres")]
        "prepare-postgres" => {
            let database_url = std::env::var("MURIARC_FIXTURE_DATABASE_URL")
                .map_err(|_| invalid("MURIARC_FIXTURE_DATABASE_URL is required"))?;
            let receipt = prepare_postgres(
                required_path(&args, "--fixture")?,
                required_path(&args, "--output")?,
                required_value(&args, "--source-commit")?,
                &database_url,
            )
            .await?;
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "ok": true,
                    "backend": "postgres",
                    "generation_id": receipt.generation_id,
                }))?
            );
        }
        "finalize" => {
            let result = finalize(
                required_path(&args, "--root")?,
                Backend::parse(required_value(&args, "--backend")?)?,
                required_value(&args, "--source-artifact-digest")?.parse()?,
                required_value(&args, "--source-provenance-digest")?.parse()?,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        "verify" => {
            let root = required_path(&args, "--root")?;
            let expected = optional_value(&args, "--manifest-digest")
                .map(str::parse)
                .transpose()?;
            let (_, _, result) = load_and_verify_fixture(&root, expected.as_ref())?;
            println!("{}", serde_json::to_string(&result)?);
        }
        _ => return Err(invalid("unknown release fixture command").into()),
    }
    Ok(true)
}

pub async fn prepare_sqlite(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
) -> ReleaseFixtureResult<SeedReceipt> {
    let output = output_root.as_ref();
    require_new_output(output)?;
    let receipt = seed_standard_v1(fixture_root, output, source_commit).await?;
    let store =
        SqliteStore::connect_path(output.join(muriarc_standard_fixture::DATABASE_FILE)).await?;
    let result = async {
        let mut ai = seed_ai(&store, &receipt).await?;
        let secret_ref = AiModelProfileSecretRef {
            profile_id: ai.profile.id,
            profile_version: ai.version.version,
            keyring_account: SYNTHETIC_KEYRING_ACCOUNT.to_owned(),
            credential_state: AiModelCredentialState::Present,
            created_at: ai.version.created_at,
            updated_at: ai.version.created_at,
            revision: 1,
        };
        store
            .save_ai_model_profile_secret_ref(
                &secret_ref,
                None,
                &fixture_audit("desktop-keyring-reference"),
            )
            .await?;
        ai.secret_digests
            .insert(digest_bytes(secret_ref.keyring_account.as_bytes()));
        ai.key_versions.insert(1);
        write_release_state(output, &store, &receipt, &ai, Backend::Sqlite).await?;
        Ok(receipt)
    }
    .await;
    store.pool().close().await;
    result
}

#[cfg(feature = "postgres")]
pub async fn prepare_postgres(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
    database_url: &str,
) -> ReleaseFixtureResult<SeedReceipt> {
    use muriarc_server::{
        AiMasterKey, PostgresAiProviderStore, SaveAiProviderSettingsInput, UserAiProviderStore,
    };
    use muriarc_standard_fixture::seed_postgres_standard_v1;
    use muriarc_store_postgres::PostgresStore;
    use sqlx::Row;

    let output = output_root.as_ref();
    require_new_output(output)?;
    let receipt =
        seed_postgres_standard_v1(fixture_root, output, source_commit, database_url).await?;
    let store = PostgresStore::connect(database_url).await?;
    let result = async {
        let mut ai = seed_ai(&store, &receipt).await?;
        let provider = PostgresAiProviderStore::new(
            store.clone(),
            AiMasterKey::from_base64(SYNTHETIC_MASTER_KEY_BASE64, 1)?,
        );
        provider
            .save(
                LOCAL_USER_ID,
                SaveAiProviderSettingsInput {
                    enabled: true,
                    provider_kind: muriarc_ai::ProviderKind::OpenAiCompatible,
                    provider_preset_id: "deepseek".to_owned(),
                    model: "muriarc-release-fixture-model".to_owned(),
                    base_url: "https://api.deepseek.com".to_owned(),
                    supports_vision: false,
                    vision_model: None,
                    context_window_tokens: 131_072,
                    max_input_tokens: 65_536,
                    max_output_tokens: 8_192,
                    history_token_budget: 32_768,
                    history_turns: 20,
                    temperature: 0.0,
                    timeout_ms: 30_000,
                    api_key: Some("muriarc-synthetic-fixture-credential-not-valid".to_owned()),
                },
                &fixture_audit("postgres-encrypted-provider"),
            )
            .await?;
        let row = sqlx::query(
            "SELECT secret_key_version, secret_nonce, secret_ciphertext
               FROM ai_provider_settings
              WHERE user_id = $1",
        )
        .bind(LOCAL_USER_ID)
        .fetch_one(store.pool())
        .await?;
        let version: i32 = row.try_get("secret_key_version")?;
        let nonce: Vec<u8> = row.try_get("secret_nonce")?;
        let ciphertext: Vec<u8> = row.try_get("secret_ciphertext")?;
        let mut envelope = nonce;
        envelope.extend_from_slice(&ciphertext);
        ai.secret_digests.insert(digest_bytes(&envelope));
        ai.key_versions.insert(version);
        write_release_state(output, &store, &receipt, &ai, Backend::Postgres).await?;
        Ok(receipt)
    }
    .await;
    store.pool().close().await;
    result
}

async fn seed_ai<S>(store: &S, receipt: &SeedReceipt) -> ReleaseFixtureResult<AiSeed>
where
    S: FixtureStore,
{
    let now = DateTime::parse_from_rfc3339("2025-03-01T00:00:00Z")?.with_timezone(&Utc);
    let project_id = receipt
        .ids
        .projects
        .get("intervention")
        .copied()
        .ok_or_else(|| invalid("standard-v1 intervention project is missing"))?;
    let profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: LOCAL_LAB_ID,
        user_id: LOCAL_USER_ID,
        name: "MuriArc release fixture model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    let version = AiModelProfileVersion {
        profile_id: profile.id,
        version: 1,
        protocol: AiProviderProtocol::OpenaiChatCompletions,
        transport: AiProviderTransport::LocalHttp,
        base_url: "http://127.0.0.1:11434".to_owned(),
        normalized_base_url: "http://127.0.0.1:11434".to_owned(),
        model_id: "muriarc-release-fixture-local".to_owned(),
        supports_vision: false,
        context_window_tokens: 32_768,
        max_input_tokens: 16_384,
        max_output_tokens: 2_048,
        history_token_budget: 8_192,
        history_turns: 20,
        temperature: 0.0,
        timeout_ms: 30_000,
        created_at: now,
    };
    store
        .create_ai_model_profile(&profile, &version, &fixture_audit("ai-profile"))
        .await?;

    let conversation = AiConversation {
        id: Uuid::new_v4(),
        lab_id: LOCAL_LAB_ID,
        project_id: Some(project_id),
        user_id: LOCAL_USER_ID,
        title: "standard-v1 synthetic animal review".to_owned(),
        model_profile: Some(AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: version.version,
        }),
        legacy_read_only: false,
        pinned_at: None,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_conversation(&conversation, &fixture_audit("ai-conversation"))
        .await?;

    let user_message = AiConversationMessage::new(
        conversation.id,
        LOCAL_LAB_ID,
        Some(project_id),
        LOCAL_USER_ID,
        1,
        AiConversationMessageRole::User,
        "Summarize the synthetic cohort without making any write.",
        None,
        now,
    )?;
    let assistant_message = AiConversationMessage::new(
        conversation.id,
        LOCAL_LAB_ID,
        Some(project_id),
        LOCAL_USER_ID,
        2,
        AiConversationMessageRole::Assistant,
        "The standard-v1 cohort contains only synthetic animal records.",
        Some(json!({
            "content": "synthetic summary",
            "citations": [],
            "trace": {"provider": "fixture", "external_request": false}
        })),
        now,
    )?;
    let conversation = store
        .append_ai_turn_messages(
            &user_message,
            &assistant_message,
            0,
            &fixture_ai_audit("ai-messages"),
        )
        .await?;

    let mut tool_run = ToolRun {
        id: Uuid::new_v4(),
        conversation_id: Some(conversation.id),
        lab_id: LOCAL_LAB_ID,
        project_id: Some(project_id),
        user_id: LOCAL_USER_ID,
        tool_name: "animal_context".to_owned(),
        input: json!({"animal_id": receipt.ids.animals["animal_005"]}),
        output: Some(json!({"synthetic": true, "status": "pending_approval"})),
        status: ToolRunStatus::AwaitingApproval,
        source: WriteSource::Ai,
        started_at: Some(now),
        completed_at: None,
        error: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_tool_run(&tool_run, &fixture_ai_audit("ai-tool-run"))
        .await?;

    let mut approval = Approval {
        id: Uuid::new_v4(),
        tool_run_id: tool_run.id,
        requested_diff: json!({
            "kind": "synthetic_review",
            "writes": [],
        }),
        decision: ApprovalDecision::Pending,
        decided_by: None,
        decided_at: None,
        reason: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_approval(&approval, &fixture_ai_audit("ai-approval"))
        .await?;

    let expected_tool_revision = tool_run.meta.revision;
    let expected_approval_revision = approval.meta.revision;
    let decided_at = now + chrono::Duration::seconds(1);
    tool_run.status = ToolRunStatus::Completed;
    tool_run.completed_at = Some(decided_at);
    tool_run.meta.touch(decided_at);
    approval.decision = ApprovalDecision::Approved;
    approval.decided_by = Some(LOCAL_USER_ID);
    approval.decided_at = Some(decided_at);
    approval.reason = Some("Synthetic release fixture approval".to_owned());
    approval.meta.touch(decided_at);
    store
        .finalize_ai_draft(
            &tool_run,
            expected_tool_revision,
            &approval,
            expected_approval_revision,
            &fixture_audit("ai-approval-decision"),
        )
        .await?;

    let job = Job {
        id: Uuid::new_v4(),
        lab_id: LOCAL_LAB_ID,
        project_id: Some(project_id),
        created_by: LOCAL_USER_ID,
        kind: JobKind::Snapshot,
        status: JobStatus::Completed,
        idempotency_key: format!("release-fixture-{}", receipt.generation_id),
        progress_current: 1,
        progress_total: Some(1),
        result: Some(json!({"synthetic": true, "fixture": "standard-v1"})),
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(now),
    };
    store.create_job(&job, &fixture_audit("ai-job")).await?;

    Ok(AiSeed {
        profile,
        version,
        conversation,
        messages: vec![user_message, assistant_message],
        tool_run,
        approval,
        job,
        secret_digests: BTreeSet::new(),
        key_versions: BTreeSet::new(),
    })
}

async fn write_release_state<S>(
    root: &Path,
    store: &S,
    receipt: &SeedReceipt,
    ai: &AiSeed,
    backend: Backend,
) -> ReleaseFixtureResult<()>
where
    S: FixtureStore,
{
    ensure(
        !ai.secret_digests.is_empty() && !ai.key_versions.is_empty(),
        "release fixture secret recovery evidence is missing",
    )?;
    let fixture_id = Uuid::new_v4();
    let facts = expected_facts(store, receipt, ai, backend, fixture_id).await?;
    write_json_canonical(&root.join(EXPECTED_FACTS_FILE), &facts)?;
    write_json_pretty_new(
        &root.join(CONFIGURATION_FILE),
        &json!({
            "format_version": 1,
            "backend": match backend { Backend::Sqlite => "sqlite", Backend::Postgres => "postgres" },
            "synthetic": true,
            "external_ai_requests_enabled": false,
            "standard_dataset": "standard-v1",
        }),
    )?;
    let keyset = match backend {
        Backend::Sqlite => json!({
            "format_version": 1,
            "kind": "desktop_keyring_recovery_reference",
            "synthetic": true,
            "key_version": 1,
            "keyring_account": SYNTHETIC_KEYRING_ACCOUNT,
        }),
        #[cfg(feature = "postgres")]
        Backend::Postgres => json!({
            "format_version": 1,
            "kind": "server_master_key",
            "synthetic": true,
            "key_version": 1,
            "master_key_base64": SYNTHETIC_MASTER_KEY_BASE64,
        }),
        #[cfg(not(feature = "postgres"))]
        Backend::Postgres => return Err(invalid("PostgreSQL support is not compiled").into()),
    };
    write_json_pretty_new(&root.join(KEYSET_FILE), &keyset)?;
    write_json_pretty_new(
        &root.join(AI_STATE_FILE),
        &json!({
            "format_version": 1,
            "synthetic": true,
            "conversation_ids": [ai.conversation.id],
            "secret_recovery_record_count": ai.secret_digests.len(),
            "secret_recovery_digests": ai.secret_digests,
            "key_versions": ai.key_versions,
        }),
    )?;
    Ok(())
}

async fn expected_facts<S>(
    store: &S,
    receipt: &SeedReceipt,
    ai: &AiSeed,
    backend: Backend,
    fixture_id: Uuid,
) -> ReleaseFixtureResult<ExpectedFacts>
where
    S: FixtureStore,
{
    let identity = match backend {
        Backend::Sqlite => SqliteStore::compiled_release_identity(),
        #[cfg(feature = "postgres")]
        Backend::Postgres => muriarc_store_postgres::PostgresStore::compiled_release_identity(),
        #[cfg(not(feature = "postgres"))]
        Backend::Postgres => return Err(invalid("PostgreSQL support is not compiled").into()),
    };
    let user = store.get_user(LOCAL_USER_ID).await?;
    let project_ids = receipt
        .ids
        .projects
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    let accounts = vec![AccountFact {
        user_id: user.id,
        normalized_email_digest: digest_bytes(user.email.trim().to_ascii_lowercase().as_bytes()),
        lab_roles: BTreeSet::new(),
        project_ids: project_ids.clone(),
        active: enum_string(&user.status)? == "active",
    }];

    let mut projects = Vec::new();
    for id in receipt.ids.projects.values() {
        let project = store.get_project(*id).await?;
        projects.push(ProjectFact {
            project_id: project.id,
            name_digest: digest_bytes(project.name.as_bytes()),
            active: project.meta.deleted_at.is_none(),
        });
    }

    let mut animals = Vec::new();
    for id in receipt.ids.animals.values() {
        let animal = store.get_animal(*id).await?;
        let pedigrees = store.list_pedigrees(animal.id).await?;
        let sire_id = pedigrees
            .iter()
            .find(|value| value.parent_type == ParentType::Father)
            .map(|value| value.parent_id);
        let dam_id = pedigrees
            .iter()
            .find(|value| value.parent_type == ParentType::Mother)
            .map(|value| value.parent_id);
        animals.push(AnimalFact {
            animal_id: animal.id,
            display_id: animal.display_id,
            status: enum_string(&animal.current_status)?,
            sire_id,
            dam_id,
            revision: animal.meta.revision,
        });
    }
    let animal_ids = animals
        .iter()
        .map(|value| value.animal_id)
        .collect::<Vec<_>>();

    let mut breeding = Vec::new();
    for id in receipt.ids.breeding_pairs.values() {
        let pair = store.get_breeding_pair(*id).await?;
        let male_id = pair
            .members
            .iter()
            .find(|member| member.role == BreedingMemberRole::Male)
            .map(|member| member.animal_id)
            .ok_or_else(|| invalid("breeding pair has no male"))?;
        let female_ids = pair
            .members
            .iter()
            .filter(|member| member.role == BreedingMemberRole::Female)
            .map(|member| member.animal_id)
            .collect::<BTreeSet<_>>();
        let parent_ids = pair
            .members
            .iter()
            .map(|member| member.animal_id)
            .collect::<BTreeSet<_>>();
        let mut offspring_ids = BTreeSet::new();
        for animal_id in &animal_ids {
            let parents = store.list_pedigrees(*animal_id).await?;
            if parents
                .iter()
                .any(|edge| parent_ids.contains(&edge.parent_id))
            {
                offspring_ids.insert(*animal_id);
            }
        }
        if !offspring_ids.is_empty() {
            breeding.push(BreedingFact {
                breeding_id: pair.id,
                male_id,
                female_ids,
                offspring_ids,
                status: enum_string(&pair.status)?,
            });
        }
    }

    let mut experiments = Vec::new();
    for id in receipt.ids.experiments.values() {
        let experiment = store.get_experiment(*id).await?;
        let participations = store
            .list_participations(&ParticipationFilter {
                project_id: experiment.project_id,
                experiment_id: Some(experiment.id),
                animal_id: None,
                cohort_id: None,
            })
            .await?;
        if !participations.is_empty() {
            experiments.push(ExperimentFact {
                experiment_id: experiment.id,
                project_id: experiment.project_id,
                animal_ids: participations.iter().map(|value| value.animal_id).collect(),
                status: enum_string(&experiment.status)?,
                revision: experiment.meta.revision,
            });
        }
    }

    let mut observations = Vec::new();
    for id in receipt.ids.observations.values() {
        let observation = store.get_observation(*id).await?;
        let values = store.list_observation_values(observation.id).await?;
        let latest = values
            .iter()
            .max_by_key(|value| value.version)
            .ok_or_else(|| invalid("observation has no value"))?;
        observations.push(ObservationFact {
            observation_id: observation.id,
            experiment_id: observation.experiment_id,
            animal_id: observation.subject_id,
            value_digest: digest_serialized(&latest.value)?,
            signed: false,
            revision: observation.meta.revision,
        });
    }

    let mut measurements = Vec::new();
    for id in receipt.ids.measurements.values() {
        let measurement = store.get_measurement(*id).await?;
        measurements.push(MeasurementFact {
            measurement_id: measurement.id,
            experiment_id: measurement
                .experiment_id
                .ok_or_else(|| invalid("fixture measurement has no experiment"))?,
            animal_id: measurement.animal_id,
            value_digest: digest_serialized(&measurement.value)?,
            status: enum_string(&measurement.status)?,
            signed: measurement.signed_by.is_some() && measurement.signed_at.is_some(),
            revision: measurement.meta.revision,
        });
    }

    let mut samples = Vec::new();
    for id in receipt.ids.samples.values() {
        let sample = store.get_sample(*id).await?;
        samples.push(SampleFact {
            sample_id: sample.id,
            experiment_id: sample
                .experiment_id
                .ok_or_else(|| invalid("fixture sample has no experiment"))?,
            animal_id: sample.animal_id,
            status: if sample.meta.deleted_at.is_none() {
                "active".to_owned()
            } else {
                "deleted".to_owned()
            },
            revision: sample.meta.revision,
        });
    }

    let mut attachments = Vec::new();
    for id in receipt.ids.attachments.values() {
        let attachment = store.get_attachment(*id).await?;
        attachments.push(AttachmentFact {
            attachment_id: attachment.id,
            owner_entity_id: attachment.entity_id,
            size_bytes: u64::try_from(attachment.size_bytes)
                .map_err(|_| invalid("attachment size is negative"))?,
            content_sha256: format!("sha256:{}", attachment.sha256).parse()?,
        });
    }

    let audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_id: None,
        })
        .await?;
    let mut action_counts = BTreeMap::new();
    for entry in &audits {
        *action_counts
            .entry(entry.action.as_str().to_owned())
            .or_insert(0) += 1;
    }
    let provenance = store
        .list_provenance(&ProvenanceFilter {
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_type: None,
            entity_id: None,
            source: None,
        })
        .await?;

    let profile_digest = digest_serialized(&ai.version)?;
    let expected = ExpectedFacts {
        format_version: 1,
        fixture_id,
        release_identity: identity,
        accounts,
        projects,
        animals,
        breeding,
        experiments,
        observations,
        measurements,
        samples,
        attachments,
        ai_history: AiHistoryFact {
            profiles: vec![AiProfileFact {
                profile_id: ai.profile.id,
                current_version: ai.profile.current_version,
                version_digests: BTreeMap::from([(ai.version.version, profile_digest)]),
                archived: ai.profile.archived_at.is_some(),
            }],
            conversations: vec![AiConversationFact {
                conversation_id: ai.conversation.id,
                project_id: ai.conversation.project_id,
                profile_id: ai.conversation.model_profile.map(|value| value.profile_id),
                profile_version: ai
                    .conversation
                    .model_profile
                    .map(|value| value.profile_version),
                message_ids: ai.messages.iter().map(|value| value.id).collect(),
                legacy_read_only: ai.conversation.legacy_read_only,
                revision: ai.conversation.meta.revision,
            }],
            messages: ai
                .messages
                .iter()
                .map(|message| {
                    Ok(AiMessageFact {
                        message_id: message.id,
                        conversation_id: message.conversation_id,
                        sequence: message.sequence,
                        role: enum_string(&message.role)?,
                        content_digest: digest_bytes(message.content.as_bytes()),
                        response_digest: message
                            .response
                            .as_ref()
                            .map(digest_serialized)
                            .transpose()?,
                        revision: message.meta.revision,
                    })
                })
                .collect::<ReleaseFixtureResult<Vec<_>>>()?,
            tool_runs: vec![AiToolRunFact {
                tool_run_id: ai.tool_run.id,
                conversation_id: ai.tool_run.conversation_id,
                status: enum_string(&ai.tool_run.status)?,
                input_digest: digest_serialized(&ai.tool_run.input)?,
                output_digest: ai
                    .tool_run
                    .output
                    .as_ref()
                    .map(digest_serialized)
                    .transpose()?,
                revision: ai.tool_run.meta.revision,
            }],
            approvals: vec![AiApprovalFact {
                approval_id: ai.approval.id,
                tool_run_id: ai.approval.tool_run_id,
                decision: enum_string(&ai.approval.decision)?,
                revision: ai.approval.meta.revision,
            }],
            jobs: vec![AiJobFact {
                job_id: ai.job.id,
                kind: enum_string(&ai.job.kind)?,
                status: enum_string(&ai.job.status)?,
                revision: ai.job.meta.revision,
            }],
            conversation_ids: BTreeSet::from([ai.conversation.id]),
            encrypted_envelope_count: u64::try_from(ai.secret_digests.len())?,
            ciphertext_digests: ai.secret_digests.clone(),
            key_versions: ai.key_versions.clone(),
        },
        audit: AuditFact {
            minimum_entry_count: u64::try_from(audits.len())?,
            entity_ids: audits.iter().map(|entry| entry.entity_id).collect(),
            action_counts,
        },
        provenance: ProvenanceFact {
            minimum_record_count: u64::try_from(provenance.len())?,
            entity_ids: provenance.iter().map(|entry| entry.entity_id).collect(),
            source_kinds: provenance
                .iter()
                .map(|entry| enum_string(&entry.source))
                .collect::<ReleaseFixtureResult<BTreeSet<_>>>()?,
        },
        continuation: ContinuationExpectation {
            actor_user_id: LOCAL_USER_ID,
            animal_id: receipt.ids.animals["animal_005"],
            expected_previous_revision: store
                .get_animal(receipt.ids.animals["animal_005"])
                .await?
                .meta
                .revision,
            write_kind: "animal_update".to_owned(),
            expected_audit_delta: 1,
            expected_provenance_delta: 1,
        },
    };
    let placeholder = FixtureManifest {
        format_version: FIXTURE_FORMAT_VERSION,
        fixture_id,
        backend: backend.core(),
        release_identity: expected.release_identity.clone(),
        generation_id: receipt.generation_id,
        producer: FixtureProducerProvenance {
            generator_application_version: expected.release_identity.application_version.clone(),
            generator_data_epoch: expected.release_identity.data_epoch.clone(),
            generator_backend_state_digest: expected.release_identity.backend_state_digest.clone(),
            source_release_artifact_digest: zero_digest(),
            source_release_provenance_digest: zero_digest(),
            generated_at: Utc::now(),
        },
        files: Vec::new(),
        expected_facts_digest: zero_digest(),
    };
    // ExpectedFacts::validate also checks cross-domain and AI-history closure.
    // A temporary complete file set is used only for this structural check.
    let mut validation_manifest = placeholder;
    validation_manifest.files = placeholder_files();
    validation_manifest.expected_facts_digest = expected_facts_digest(&expected)?;
    ensure(
        !expected.accounts.is_empty()
            && !expected.projects.is_empty()
            && !expected.animals.is_empty()
            && !expected.breeding.is_empty()
            && !expected.experiments.is_empty()
            && !expected.observations.is_empty()
            && !expected.measurements.is_empty()
            && !expected.samples.is_empty()
            && !expected.attachments.is_empty()
            && !expected.ai_history.profiles.is_empty()
            && !expected.ai_history.conversations.is_empty()
            && !expected.ai_history.messages.is_empty()
            && !expected.ai_history.tool_runs.is_empty()
            && !expected.ai_history.approvals.is_empty()
            && !expected.ai_history.jobs.is_empty()
            && !expected.ai_history.conversation_ids.is_empty()
            && expected.ai_history.encrypted_envelope_count > 0
            && !expected.ai_history.key_versions.is_empty()
            && expected.audit.minimum_entry_count > 0
            && expected.provenance.minimum_record_count > 0,
        format!(
            "release expected facts are incomplete: accounts={}, projects={}, animals={}, breeding={}, experiments={}, observations={}, measurements={}, samples={}, attachments={}, ai_profiles={}, ai_conversations={}, ai_messages={}, ai_tools={}, ai_approvals={}, ai_jobs={}, ai_envelopes={}, audits={}, provenance={}",
            expected.accounts.len(),
            expected.projects.len(),
            expected.animals.len(),
            expected.breeding.len(),
            expected.experiments.len(),
            expected.observations.len(),
            expected.measurements.len(),
            expected.samples.len(),
            expected.attachments.len(),
            expected.ai_history.profiles.len(),
            expected.ai_history.conversations.len(),
            expected.ai_history.messages.len(),
            expected.ai_history.tool_runs.len(),
            expected.ai_history.approvals.len(),
            expected.ai_history.jobs.len(),
            expected.ai_history.encrypted_envelope_count,
            expected.audit.minimum_entry_count,
            expected.provenance.minimum_record_count,
        ),
    )?;
    ensure(
        !expected.animals.iter().any(|fact| {
            fact.display_id.trim().is_empty() || fact.status.trim().is_empty() || fact.revision < 1
        }),
        "release animal facts contain invalid display/status/revision values",
    )?;
    ensure(
        !expected.breeding.iter().any(|fact| {
            fact.female_ids.is_empty()
                || fact.offspring_ids.is_empty()
                || fact.status.trim().is_empty()
        }),
        "release breeding facts contain invalid female/offspring/status values",
    )?;
    ensure(
        !expected.experiments.iter().any(|fact| {
            fact.animal_ids.is_empty() || fact.status.trim().is_empty() || fact.revision < 1
        }),
        "release experiment facts contain invalid animal/status/revision values",
    )?;
    ensure(
        !expected.observations.iter().any(|fact| fact.revision < 1),
        "release observation facts contain an invalid revision",
    )?;
    ensure(
        !expected
            .measurements
            .iter()
            .any(|fact| fact.status.trim().is_empty() || fact.revision < 1),
        "release measurement facts contain invalid status/revision values",
    )?;
    ensure(
        !expected
            .samples
            .iter()
            .any(|fact| fact.status.trim().is_empty() || fact.revision < 1),
        "release sample facts contain invalid status/revision values",
    )?;
    ensure(
        !expected.attachments.iter().any(|fact| fact.size_bytes == 0),
        "release attachment facts contain an empty payload",
    )?;
    ensure(
        expected.ai_history.ciphertext_digests.len()
            == usize::try_from(expected.ai_history.encrypted_envelope_count)?,
        "release AI ciphertext count differs from its digest set",
    )?;
    ensure(
        expected
            .ai_history
            .key_versions
            .iter()
            .all(|version| *version >= 1),
        "release AI key versions contain an invalid value",
    )?;
    ensure(
        !expected.continuation.actor_user_id.is_nil()
            && !expected.continuation.animal_id.is_nil()
            && expected.continuation.expected_previous_revision >= 1
            && !expected.continuation.write_kind.trim().is_empty()
            && expected.continuation.expected_audit_delta > 0
            && expected.continuation.expected_provenance_delta > 0,
        "release continuation expectation is incomplete",
    )?;
    validate_expected_fact_closure(&expected)?;
    expected.validate(&validation_manifest)?;
    Ok(expected)
}

fn validate_expected_fact_closure(expected: &ExpectedFacts) -> ReleaseFixtureResult<()> {
    let animals = expected
        .animals
        .iter()
        .map(|fact| fact.animal_id)
        .collect::<BTreeSet<_>>();
    let projects = expected
        .projects
        .iter()
        .map(|fact| fact.project_id)
        .collect::<BTreeSet<_>>();
    let experiments = expected
        .experiments
        .iter()
        .map(|fact| fact.experiment_id)
        .collect::<BTreeSet<_>>();
    ensure(
        expected
            .accounts
            .iter()
            .all(|fact| fact.project_ids.is_subset(&projects)),
        "release account facts reference an unknown project",
    )?;
    ensure(
        expected.animals.iter().all(|fact| {
            fact.sire_id.is_none_or(|id| animals.contains(&id))
                && fact.dam_id.is_none_or(|id| animals.contains(&id))
        }),
        "release animal facts reference an unknown parent",
    )?;
    ensure(
        expected.breeding.iter().all(|fact| {
            animals.contains(&fact.male_id)
                && fact.female_ids.is_subset(&animals)
                && fact.offspring_ids.is_subset(&animals)
        }),
        "release breeding facts reference an unknown animal",
    )?;
    ensure(
        expected
            .experiments
            .iter()
            .all(|fact| projects.contains(&fact.project_id) && fact.animal_ids.is_subset(&animals)),
        "release experiment facts reference an unknown project or animal",
    )?;
    ensure(
        expected.observations.iter().all(|fact| {
            experiments.contains(&fact.experiment_id) && animals.contains(&fact.animal_id)
        }),
        "release observation facts reference an unknown experiment or animal",
    )?;
    ensure(
        expected.measurements.iter().all(|fact| {
            experiments.contains(&fact.experiment_id) && animals.contains(&fact.animal_id)
        }),
        "release measurement facts reference an unknown experiment or animal",
    )?;
    ensure(
        expected.samples.iter().all(|fact| {
            experiments.contains(&fact.experiment_id) && animals.contains(&fact.animal_id)
        }),
        "release sample facts reference an unknown experiment or animal",
    )?;
    let attachment_owners = projects
        .iter()
        .chain(animals.iter())
        .chain(expected.breeding.iter().map(|fact| &fact.breeding_id))
        .chain(experiments.iter())
        .chain(
            expected
                .observations
                .iter()
                .map(|fact| &fact.observation_id),
        )
        .chain(
            expected
                .measurements
                .iter()
                .map(|fact| &fact.measurement_id),
        )
        .chain(expected.samples.iter().map(|fact| &fact.sample_id))
        .copied()
        .collect::<BTreeSet<_>>();
    ensure(
        expected
            .attachments
            .iter()
            .all(|fact| attachment_owners.contains(&fact.owner_entity_id)),
        "release attachment facts reference an unsupported owner domain",
    )?;
    ensure(
        animals.contains(&expected.continuation.animal_id)
            && expected
                .accounts
                .iter()
                .any(|fact| fact.user_id == expected.continuation.actor_user_id),
        "release continuation expectation references an unknown actor or animal",
    )?;

    let profiles = expected
        .ai_history
        .profiles
        .iter()
        .map(|fact| (fact.profile_id, fact))
        .collect::<BTreeMap<_, _>>();
    ensure(
        profiles.values().all(|fact| {
            fact.current_version >= 1
                && !fact.version_digests.is_empty()
                && fact.version_digests.contains_key(&fact.current_version)
                && fact.version_digests.keys().all(|version| *version >= 1)
        }),
        "release AI profile facts do not close over their current version",
    )?;
    let conversations = expected
        .ai_history
        .conversations
        .iter()
        .map(|fact| (fact.conversation_id, fact))
        .collect::<BTreeMap<_, _>>();
    ensure(
        conversations.keys().copied().collect::<BTreeSet<_>>()
            == expected.ai_history.conversation_ids
            && conversations.values().all(|fact| {
                fact.revision >= 1
                    && !fact.message_ids.is_empty()
                    && match (fact.profile_id, fact.profile_version) {
                        (Some(profile_id), Some(version)) => profiles
                            .get(&profile_id)
                            .is_some_and(|profile| profile.version_digests.contains_key(&version)),
                        (None, None) => fact.legacy_read_only,
                        _ => false,
                    }
            }),
        "release AI conversation facts do not close over profile versions/messages",
    )?;
    let messages = expected
        .ai_history
        .messages
        .iter()
        .map(|fact| fact.message_id)
        .collect::<BTreeSet<_>>();
    let referenced_messages = conversations
        .values()
        .flat_map(|fact| fact.message_ids.iter().copied())
        .collect::<BTreeSet<_>>();
    ensure(
        messages == referenced_messages
            && expected.ai_history.messages.iter().all(|fact| {
                fact.sequence >= 1
                    && fact.revision >= 1
                    && !fact.role.trim().is_empty()
                    && conversations
                        .get(&fact.conversation_id)
                        .is_some_and(|conversation| {
                            conversation.message_ids.contains(&fact.message_id)
                        })
            }),
        "release AI message facts do not close over their conversation",
    )?;
    let tool_runs = expected
        .ai_history
        .tool_runs
        .iter()
        .map(|fact| fact.tool_run_id)
        .collect::<BTreeSet<_>>();
    ensure(
        expected.ai_history.tool_runs.iter().all(|fact| {
            fact.revision >= 1
                && !fact.status.trim().is_empty()
                && fact
                    .conversation_id
                    .is_none_or(|id| conversations.contains_key(&id))
        }),
        "release AI tool-run facts are incomplete",
    )?;
    ensure(
        expected.ai_history.approvals.iter().all(|fact| {
            fact.revision >= 1
                && !fact.decision.trim().is_empty()
                && tool_runs.contains(&fact.tool_run_id)
        }),
        "release AI approval facts reference an invalid tool run",
    )?;
    ensure(
        expected.ai_history.jobs.iter().all(|fact| {
            fact.revision >= 1 && !fact.kind.trim().is_empty() && !fact.status.trim().is_empty()
        }),
        "release AI job facts are incomplete",
    )?;
    Ok(())
}

fn finalize(
    root: impl AsRef<Path>,
    backend: Backend,
    source_artifact_digest: Sha256Digest,
    source_provenance_digest: Sha256Digest,
) -> ReleaseFixtureResult<Value> {
    let root = canonical_real_directory(root.as_ref())?;
    let facts: ExpectedFacts = load_json(&root.join(EXPECTED_FACTS_FILE))?;
    let generation: muriarc_core::DeploymentGenerationManifest =
        load_json(&root.join(muriarc_standard_fixture::GENERATION_MANIFEST_FILE))?;
    ensure(
        facts.release_identity.backend_state_digest == generation.backend_state_digest
            && facts.release_identity.data_epoch == generation.data_epoch,
        "Expected Facts and generation manifest differ",
    )?;
    let expected_identity = match backend {
        Backend::Sqlite => SqliteStore::compiled_release_identity(),
        #[cfg(feature = "postgres")]
        Backend::Postgres => muriarc_store_postgres::PostgresStore::compiled_release_identity(),
        #[cfg(not(feature = "postgres"))]
        Backend::Postgres => return Err(invalid("PostgreSQL support is not compiled").into()),
    };
    ensure(
        facts.release_identity == expected_identity,
        "fixture identity differs from the compiled producer",
    )?;
    let files = inventory_files(&root)?;
    let facts_digest = expected_facts_digest(&facts)?;
    let generated_at = Utc::now();
    let manifest = FixtureManifest {
        format_version: FIXTURE_FORMAT_VERSION,
        fixture_id: facts.fixture_id,
        backend: backend.core(),
        release_identity: facts.release_identity.clone(),
        generation_id: generation.generation_id,
        producer: FixtureProducerProvenance {
            generator_application_version: facts.release_identity.application_version.clone(),
            generator_data_epoch: facts.release_identity.data_epoch.clone(),
            generator_backend_state_digest: facts.release_identity.backend_state_digest.clone(),
            source_release_artifact_digest: source_artifact_digest,
            source_release_provenance_digest: source_provenance_digest,
            generated_at,
        },
        files,
        expected_facts_digest: facts_digest,
    };
    CurrentReleaseFixtureProducer::new(
        backend.core(),
        facts.release_identity.backend_state_digest.clone(),
    )?
    .validate_manifest(&manifest)?;
    manifest.validate()?;
    write_json_pretty_new(&root.join(FIXTURE_MANIFEST_FILE), &manifest)?;
    let digest = fixture_manifest_digest(&manifest)?;
    load_and_verify_fixture(&root, Some(&digest))?;
    Ok(json!({
        "ok": true,
        "fixture_id": manifest.fixture_id,
        "backend": manifest.backend,
        "fixture_manifest_digest": digest,
        "generated_at": generated_at,
    }))
}

fn inventory_files(root: &Path) -> ReleaseFixtureResult<Vec<FixtureFile>> {
    let mut paths = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure(
                !metadata.file_type().is_symlink(),
                "fixture contains a symlink",
            )?;
            if metadata.is_dir() {
                pending.push(path);
            } else {
                ensure(metadata.is_file(), "fixture contains a special file")?;
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or_else(|| invalid("fixture paths must be UTF-8"))?
                    .replace('\\', "/");
                if relative != FIXTURE_MANIFEST_FILE {
                    paths.push((relative, path, metadata.len()));
                }
            }
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    let mut files = Vec::new();
    for (relative, path, size_bytes) in paths {
        ensure(size_bytes > 0, format!("fixture file is empty: {relative}"))?;
        let kind = classify_file(&relative)?;
        files.push(FixtureFile {
            path: relative,
            kind,
            size_bytes,
            sha256: format!("sha256:{:x}", Sha256::digest(fs::read(path)?)).parse()?,
        });
    }
    Ok(files)
}

fn classify_file(path: &str) -> ReleaseFixtureResult<FixtureComponentKind> {
    let kind =
        if path == muriarc_standard_fixture::DATABASE_FILE || path == "database/postgres.dump" {
            FixtureComponentKind::Database
        } else if path.starts_with("attachments/") {
            FixtureComponentKind::Attachments
        } else if path == muriarc_standard_fixture::RECEIPT_FILE || path.starts_with("data/") {
            FixtureComponentKind::DataArtifacts
        } else if path == CONFIGURATION_FILE {
            FixtureComponentKind::Configuration
        } else if path == KEYSET_FILE {
            FixtureComponentKind::Keyset
        } else if path == AI_STATE_FILE {
            FixtureComponentKind::AiState
        } else if path == muriarc_standard_fixture::GENERATION_MANIFEST_FILE {
            FixtureComponentKind::GenerationManifest
        } else if path == EXPECTED_FACTS_FILE {
            FixtureComponentKind::ExpectedFacts
        } else {
            return Err(invalid(format!("unclassified fixture file: {path}")).into());
        };
    Ok(kind)
}

fn placeholder_files() -> Vec<FixtureFile> {
    let digest = zero_digest();
    [
        ("database", FixtureComponentKind::Database),
        ("attachments", FixtureComponentKind::Attachments),
        ("data", FixtureComponentKind::DataArtifacts),
        ("configuration", FixtureComponentKind::Configuration),
        ("keyset", FixtureComponentKind::Keyset),
        ("ai-state", FixtureComponentKind::AiState),
        ("generation", FixtureComponentKind::GenerationManifest),
        ("expected-facts", FixtureComponentKind::ExpectedFacts),
    ]
    .into_iter()
    .map(|(path, kind)| FixtureFile {
        path: path.to_owned(),
        kind,
        size_bytes: 1,
        sha256: digest.clone(),
    })
    .collect()
}

fn fixture_audit(reason: &str) -> AuditContext {
    AuditContext {
        actor: Actor::human(LOCAL_USER_ID, LOCAL_OPERATOR_NAME),
        source: WriteSource::Migration,
        request_id: Some(format!("release-fixture-{reason}")),
        reason: Some("synthetic release fixture".to_owned()),
    }
}

fn fixture_ai_audit(reason: &str) -> AuditContext {
    AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(LOCAL_USER_ID),
            display_name: "MuriArc release fixture AI".to_owned(),
        },
        source: WriteSource::Ai,
        request_id: Some(format!("release-fixture-{reason}")),
        reason: Some("synthetic release fixture".to_owned()),
    }
}

fn enum_string(value: &impl Serialize) -> ReleaseFixtureResult<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("enum did not serialize to a string").into())
}

fn digest_serialized(value: &impl Serialize) -> ReleaseFixtureResult<Sha256Digest> {
    Ok(digest_bytes(&serde_json::to_vec(value)?))
}

fn zero_digest() -> Sha256Digest {
    format!("sha256:{}", "0".repeat(64))
        .parse()
        .expect("zero digest is valid")
}

fn required_path(args: &[String], name: &str) -> ReleaseFixtureResult<PathBuf> {
    optional_value(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| invalid(format!("{name} is required")).into())
}

fn required_value<'a>(args: &'a [String], name: &str) -> ReleaseFixtureResult<&'a str> {
    optional_value(args, name).ok_or_else(|| invalid(format!("{name} is required")).into())
}

fn optional_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn require_new_output(path: &Path) -> ReleaseFixtureResult<()> {
    ensure(
        !path.exists() && !path.is_symlink(),
        "release fixture output must be a new path",
    )
}

fn canonical_real_directory(path: &Path) -> ReleaseFixtureResult<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    ensure(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "fixture root must be a real directory",
    )?;
    Ok(fs::canonicalize(path)?)
}

fn load_json<T: DeserializeOwned>(path: &Path) -> ReleaseFixtureResult<T> {
    let metadata = fs::symlink_metadata(path)?;
    ensure(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "JSON input must be a regular file",
    )?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_json_canonical(path: &Path, value: &impl Serialize) -> ReleaseFixtureResult<()> {
    write_bytes_new(path, &serde_json::to_vec(value)?)
}

fn write_json_pretty_new(path: &Path, value: &impl Serialize) -> ReleaseFixtureResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes_new(path, &bytes)
}

fn write_bytes_new(path: &Path, bytes: &[u8]) -> ReleaseFixtureResult<()> {
    ensure(!bytes.is_empty(), "fixture output may not be empty")?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("fixture output has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn ensure(condition: bool, message: impl Into<String>) -> ReleaseFixtureResult<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message).into())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_classification_is_closed() {
        assert_eq!(
            classify_file("database/postgres.dump").unwrap(),
            FixtureComponentKind::Database
        );
        assert_eq!(
            classify_file("attachments/one/file.bin").unwrap(),
            FixtureComponentKind::Attachments
        );
        assert!(classify_file("unregistered.txt").is_err());
    }

    #[test]
    fn command_requires_known_backend() {
        assert!(Backend::parse("sqlite").is_ok());
        assert!(Backend::parse("postgres").is_ok());
        assert!(Backend::parse("demo").is_err());
    }

    #[tokio::test]
    async fn sqlite_fixture_lifecycle_is_closed_and_verifiable() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("release-fixture");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/standard-v1");
        let source_commit = "1111111111111111111111111111111111111111";
        prepare_sqlite(&fixture, &output, source_commit)
            .await
            .unwrap();
        finalize(
            &output,
            Backend::Sqlite,
            format!("sha256:{}", "2".repeat(64)).parse().unwrap(),
            format!("sha256:{}", "3".repeat(64)).parse().unwrap(),
        )
        .unwrap();
        let (manifest, facts, verification) = load_and_verify_fixture(&output, None).unwrap();
        assert_eq!(manifest.backend, muriarc_core::BackendKind::Sqlite);
        assert!(!facts.measurements.is_empty());
        assert!(!facts.ai_history.conversations.is_empty());
        assert_eq!(facts.ai_history.encrypted_envelope_count, 1);
        assert!(verification.verified_file_count >= 8);
    }
}
