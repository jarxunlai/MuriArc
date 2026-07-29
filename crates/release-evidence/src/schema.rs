use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use chrono::{DateTime, Utc};
use muriarc_core::{
    ApplicationVersion, BackendKind, BackendStateDigest, DataEpoch, GatewayContractRevision,
    ReleaseIdentity,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const FIXTURE_FORMAT_VERSION: u32 = 1;
pub const EXPECTED_FACTS_FORMAT_VERSION: u32 = 1;
pub const VERIFICATION_REPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Sha256Digest {
    type Err = EvidenceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(EvidenceError::InvalidDigest);
        };
        if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
        } else {
            Err(EvidenceError::InvalidDigest)
        }
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = EvidenceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureComponentKind {
    Database,
    Attachments,
    DataArtifacts,
    Configuration,
    Keyset,
    AiState,
    GenerationManifest,
    ExpectedFacts,
}

impl FixtureComponentKind {
    pub fn required() -> BTreeSet<Self> {
        BTreeSet::from([
            Self::Database,
            Self::Attachments,
            Self::DataArtifacts,
            Self::Configuration,
            Self::Keyset,
            Self::AiState,
            Self::GenerationManifest,
            Self::ExpectedFacts,
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureFile {
    pub path: String,
    pub kind: FixtureComponentKind,
    pub size_bytes: u64,
    pub sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureProducerProvenance {
    pub generator_application_version: ApplicationVersion,
    pub generator_data_epoch: DataEpoch,
    pub generator_backend_state_digest: BackendStateDigest,
    pub source_release_artifact_digest: Sha256Digest,
    pub source_release_provenance_digest: Sha256Digest,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub format_version: u32,
    pub fixture_id: Uuid,
    pub backend: BackendKind,
    pub release_identity: ReleaseIdentity,
    pub generation_id: Uuid,
    pub producer: FixtureProducerProvenance,
    pub files: Vec<FixtureFile>,
    pub expected_facts_digest: Sha256Digest,
}

impl FixtureManifest {
    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.format_version != FIXTURE_FORMAT_VERSION || self.fixture_id.is_nil() {
            return Err(EvidenceError::InvalidFixture {
                message: "fixture header is invalid".to_owned(),
            });
        }
        if self.producer.generator_application_version != self.release_identity.application_version
            || self.producer.generator_data_epoch != self.release_identity.data_epoch
            || self.producer.generator_backend_state_digest
                != self.release_identity.backend_state_digest
        {
            return Err(EvidenceError::WrongProducerRelease);
        }
        let mut paths = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        for file in &self.files {
            validate_fixture_path(&file.path)?;
            if file.size_bytes == 0 || !paths.insert(file.path.as_str()) {
                return Err(EvidenceError::InvalidFixture {
                    message: "fixture files must be non-empty and path-unique".to_owned(),
                });
            }
            kinds.insert(file.kind);
        }
        if kinds != FixtureComponentKind::required() {
            return Err(EvidenceError::IncompleteRecoverySet);
        }
        let expected = self
            .files
            .iter()
            .find(|file| file.kind == FixtureComponentKind::ExpectedFacts)
            .ok_or(EvidenceError::IncompleteRecoverySet)?;
        if expected.sha256 != self.expected_facts_digest {
            return Err(EvidenceError::ExpectedFactsMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountFact {
    pub user_id: Uuid,
    pub normalized_email_digest: Sha256Digest,
    pub lab_roles: BTreeSet<String>,
    pub project_ids: BTreeSet<Uuid>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFact {
    pub project_id: Uuid,
    pub name_digest: Sha256Digest,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimalFact {
    pub animal_id: Uuid,
    pub display_id: String,
    pub status: String,
    pub sire_id: Option<Uuid>,
    pub dam_id: Option<Uuid>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreedingFact {
    pub breeding_id: Uuid,
    pub male_id: Uuid,
    pub female_ids: BTreeSet<Uuid>,
    pub offspring_ids: BTreeSet<Uuid>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentFact {
    pub experiment_id: Uuid,
    pub project_id: Uuid,
    pub animal_ids: BTreeSet<Uuid>,
    pub status: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFact {
    pub observation_id: Uuid,
    pub experiment_id: Uuid,
    pub animal_id: Uuid,
    pub value_digest: Sha256Digest,
    pub signed: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementFact {
    pub measurement_id: Uuid,
    pub experiment_id: Uuid,
    pub animal_id: Uuid,
    pub value_digest: Sha256Digest,
    pub status: String,
    pub signed: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleFact {
    pub sample_id: Uuid,
    pub experiment_id: Uuid,
    pub animal_id: Uuid,
    pub status: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentFact {
    pub attachment_id: Uuid,
    pub owner_entity_id: Uuid,
    pub size_bytes: u64,
    pub content_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiProfileFact {
    pub profile_id: Uuid,
    pub current_version: i64,
    pub version_digests: BTreeMap<i64, Sha256Digest>,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiConversationFact {
    pub conversation_id: Uuid,
    pub project_id: Option<Uuid>,
    pub profile_id: Option<Uuid>,
    pub profile_version: Option<i64>,
    pub message_ids: BTreeSet<Uuid>,
    pub legacy_read_only: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiMessageFact {
    pub message_id: Uuid,
    pub conversation_id: Uuid,
    pub sequence: i64,
    pub role: String,
    pub content_digest: Sha256Digest,
    pub response_digest: Option<Sha256Digest>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiToolRunFact {
    pub tool_run_id: Uuid,
    pub conversation_id: Option<Uuid>,
    pub status: String,
    pub input_digest: Sha256Digest,
    pub output_digest: Option<Sha256Digest>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiApprovalFact {
    pub approval_id: Uuid,
    pub tool_run_id: Uuid,
    pub decision: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiJobFact {
    pub job_id: Uuid,
    pub kind: String,
    pub status: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiHistoryFact {
    pub profiles: Vec<AiProfileFact>,
    pub conversations: Vec<AiConversationFact>,
    pub messages: Vec<AiMessageFact>,
    pub tool_runs: Vec<AiToolRunFact>,
    pub approvals: Vec<AiApprovalFact>,
    pub jobs: Vec<AiJobFact>,
    pub conversation_ids: BTreeSet<Uuid>,
    pub encrypted_envelope_count: u64,
    pub ciphertext_digests: BTreeSet<Sha256Digest>,
    pub key_versions: BTreeSet<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditFact {
    pub minimum_entry_count: u64,
    pub entity_ids: BTreeSet<Uuid>,
    pub action_counts: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceFact {
    pub minimum_record_count: u64,
    pub entity_ids: BTreeSet<Uuid>,
    pub source_kinds: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationExpectation {
    pub actor_user_id: Uuid,
    pub animal_id: Uuid,
    pub expected_previous_revision: i64,
    pub write_kind: String,
    pub expected_audit_delta: u64,
    pub expected_provenance_delta: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedFacts {
    pub format_version: u32,
    pub fixture_id: Uuid,
    pub release_identity: ReleaseIdentity,
    pub accounts: Vec<AccountFact>,
    pub projects: Vec<ProjectFact>,
    pub animals: Vec<AnimalFact>,
    pub breeding: Vec<BreedingFact>,
    pub experiments: Vec<ExperimentFact>,
    pub observations: Vec<ObservationFact>,
    pub measurements: Vec<MeasurementFact>,
    pub samples: Vec<SampleFact>,
    pub attachments: Vec<AttachmentFact>,
    pub ai_history: AiHistoryFact,
    pub audit: AuditFact,
    pub provenance: ProvenanceFact,
    pub continuation: ContinuationExpectation,
}

impl ExpectedFacts {
    pub fn validate(&self, manifest: &FixtureManifest) -> Result<(), EvidenceError> {
        if self.format_version != EXPECTED_FACTS_FORMAT_VERSION
            || self.fixture_id != manifest.fixture_id
            || self.release_identity != manifest.release_identity
        {
            return Err(EvidenceError::ExpectedFactsMismatch);
        }
        if self.accounts.is_empty()
            || self.projects.is_empty()
            || self.animals.is_empty()
            || self.breeding.is_empty()
            || self.experiments.is_empty()
            || self.observations.is_empty()
            || self.measurements.is_empty()
            || self.samples.is_empty()
            || self.attachments.is_empty()
            || self.animals.iter().any(|fact| {
                fact.display_id.trim().is_empty()
                    || fact.status.trim().is_empty()
                    || fact.revision < 1
            })
            || self.breeding.iter().any(|fact| {
                fact.female_ids.is_empty()
                    || fact.offspring_ids.is_empty()
                    || fact.status.trim().is_empty()
            })
            || self.experiments.iter().any(|fact| {
                fact.animal_ids.is_empty() || fact.status.trim().is_empty() || fact.revision < 1
            })
            || self.observations.iter().any(|fact| fact.revision < 1)
            || self
                .measurements
                .iter()
                .any(|fact| fact.status.trim().is_empty() || fact.revision < 1)
            || self
                .samples
                .iter()
                .any(|fact| fact.status.trim().is_empty() || fact.revision < 1)
            || self.attachments.iter().any(|fact| fact.size_bytes == 0)
            || self.ai_history.profiles.is_empty()
            || self.ai_history.conversations.is_empty()
            || self.ai_history.messages.is_empty()
            || self.ai_history.tool_runs.is_empty()
            || self.ai_history.approvals.is_empty()
            || self.ai_history.jobs.is_empty()
            || self.ai_history.conversation_ids.is_empty()
            || self.ai_history.encrypted_envelope_count == 0
            || self.ai_history.ciphertext_digests.len()
                != usize::try_from(self.ai_history.encrypted_envelope_count).unwrap_or(usize::MAX)
            || self.ai_history.key_versions.is_empty()
            || self
                .ai_history
                .key_versions
                .iter()
                .any(|version| *version < 1)
            || self.audit.minimum_entry_count == 0
            || self.provenance.minimum_record_count == 0
            || self.continuation.actor_user_id.is_nil()
            || self.continuation.animal_id.is_nil()
            || self.continuation.expected_previous_revision < 1
            || self.continuation.write_kind.trim().is_empty()
            || self.continuation.expected_audit_delta == 0
            || self.continuation.expected_provenance_delta == 0
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }
        require_unique_ids(self.accounts.iter().map(|fact| fact.user_id), "accounts")?;
        require_unique_ids(self.projects.iter().map(|fact| fact.project_id), "projects")?;
        require_unique_ids(self.animals.iter().map(|fact| fact.animal_id), "animals")?;
        require_unique_ids(
            self.breeding.iter().map(|fact| fact.breeding_id),
            "breeding",
        )?;
        require_unique_ids(
            self.experiments.iter().map(|fact| fact.experiment_id),
            "experiments",
        )?;
        require_unique_ids(
            self.observations.iter().map(|fact| fact.observation_id),
            "observations",
        )?;
        require_unique_ids(
            self.measurements.iter().map(|fact| fact.measurement_id),
            "measurements",
        )?;
        require_unique_ids(self.samples.iter().map(|fact| fact.sample_id), "samples")?;
        require_unique_ids(
            self.attachments.iter().map(|fact| fact.attachment_id),
            "attachments",
        )?;
        require_unique_ids(
            self.ai_history.profiles.iter().map(|fact| fact.profile_id),
            "ai_profiles",
        )?;
        require_unique_ids(
            self.ai_history
                .conversations
                .iter()
                .map(|fact| fact.conversation_id),
            "ai_conversations",
        )?;
        require_unique_ids(
            self.ai_history.messages.iter().map(|fact| fact.message_id),
            "ai_messages",
        )?;
        require_unique_ids(
            self.ai_history
                .tool_runs
                .iter()
                .map(|fact| fact.tool_run_id),
            "ai_tool_runs",
        )?;
        require_unique_ids(
            self.ai_history
                .approvals
                .iter()
                .map(|fact| fact.approval_id),
            "ai_approvals",
        )?;
        require_unique_ids(
            self.ai_history.jobs.iter().map(|fact| fact.job_id),
            "ai_jobs",
        )?;
        self.validate_ai_history()?;
        let animals = self
            .animals
            .iter()
            .map(|animal| animal.animal_id)
            .collect::<BTreeSet<_>>();
        let projects = self
            .projects
            .iter()
            .map(|project| project.project_id)
            .collect::<BTreeSet<_>>();
        let experiments = self
            .experiments
            .iter()
            .map(|experiment| experiment.experiment_id)
            .collect::<BTreeSet<_>>();
        if self
            .accounts
            .iter()
            .any(|account| !account.project_ids.is_subset(&projects))
            || self.animals.iter().any(|animal| {
                animal.sire_id.is_some_and(|id| !animals.contains(&id))
                    || animal.dam_id.is_some_and(|id| !animals.contains(&id))
            })
            || self.breeding.iter().any(|breeding| {
                !animals.contains(&breeding.male_id)
                    || !breeding.female_ids.is_subset(&animals)
                    || !breeding.offspring_ids.is_subset(&animals)
            })
            || self.experiments.iter().any(|experiment| {
                !projects.contains(&experiment.project_id)
                    || !experiment.animal_ids.is_subset(&animals)
            })
            || self.observations.iter().any(|observation| {
                !experiments.contains(&observation.experiment_id)
                    || !animals.contains(&observation.animal_id)
            })
            || self.measurements.iter().any(|measurement| {
                !experiments.contains(&measurement.experiment_id)
                    || !animals.contains(&measurement.animal_id)
            })
            || self.samples.iter().any(|sample| {
                !experiments.contains(&sample.experiment_id) || !animals.contains(&sample.animal_id)
            })
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }
        let attachment_owners = projects
            .iter()
            .chain(animals.iter())
            .chain(self.breeding.iter().map(|fact| &fact.breeding_id))
            .chain(experiments.iter())
            .chain(self.observations.iter().map(|fact| &fact.observation_id))
            .chain(self.measurements.iter().map(|fact| &fact.measurement_id))
            .chain(self.samples.iter().map(|fact| &fact.sample_id))
            .copied()
            .collect::<BTreeSet<_>>();
        if self
            .attachments
            .iter()
            .any(|attachment| !attachment_owners.contains(&attachment.owner_entity_id))
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }
        if !animals.contains(&self.continuation.animal_id)
            || !self
                .accounts
                .iter()
                .any(|account| account.user_id == self.continuation.actor_user_id)
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }
        Ok(())
    }

    fn validate_ai_history(&self) -> Result<(), EvidenceError> {
        let profiles = self
            .ai_history
            .profiles
            .iter()
            .map(|profile| (profile.profile_id, profile))
            .collect::<BTreeMap<_, _>>();
        if profiles.values().any(|profile| {
            profile.current_version < 1
                || profile.version_digests.is_empty()
                || !profile
                    .version_digests
                    .contains_key(&profile.current_version)
                || profile.version_digests.keys().any(|version| *version < 1)
        }) {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }

        let conversations = self
            .ai_history
            .conversations
            .iter()
            .map(|conversation| (conversation.conversation_id, conversation))
            .collect::<BTreeMap<_, _>>();
        if conversations.keys().copied().collect::<BTreeSet<_>>()
            != self.ai_history.conversation_ids
            || conversations.values().any(|conversation| {
                conversation.revision < 1
                    || conversation.message_ids.is_empty()
                    || match (conversation.profile_id, conversation.profile_version) {
                        (Some(profile_id), Some(version)) => profiles
                            .get(&profile_id)
                            .is_none_or(|profile| !profile.version_digests.contains_key(&version)),
                        (None, None) => !conversation.legacy_read_only,
                        _ => true,
                    }
            })
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }

        let message_ids = self
            .ai_history
            .messages
            .iter()
            .map(|message| message.message_id)
            .collect::<BTreeSet<_>>();
        let referenced_message_ids = conversations
            .values()
            .flat_map(|conversation| conversation.message_ids.iter().copied())
            .collect::<BTreeSet<_>>();
        if message_ids != referenced_message_ids
            || self.ai_history.messages.iter().any(|message| {
                message.sequence < 1
                    || message.revision < 1
                    || message.role.trim().is_empty()
                    || conversations
                        .get(&message.conversation_id)
                        .is_none_or(|conversation| {
                            !conversation.message_ids.contains(&message.message_id)
                        })
            })
        {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }

        let tool_runs = self
            .ai_history
            .tool_runs
            .iter()
            .map(|tool_run| tool_run.tool_run_id)
            .collect::<BTreeSet<_>>();
        if self.ai_history.tool_runs.iter().any(|tool_run| {
            tool_run.revision < 1
                || tool_run.status.trim().is_empty()
                || tool_run
                    .conversation_id
                    .is_some_and(|id| !conversations.contains_key(&id))
        }) || self.ai_history.approvals.iter().any(|approval| {
            approval.revision < 1
                || approval.decision.trim().is_empty()
                || !tool_runs.contains(&approval.tool_run_id)
        }) || self.ai_history.jobs.iter().any(|job| {
            job.revision < 1 || job.kind.trim().is_empty() || job.status.trim().is_empty()
        }) {
            return Err(EvidenceError::ExpectedFactsIncomplete);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureCatalogEntry {
    pub fixture_id: Uuid,
    pub application_version: ApplicationVersion,
    pub data_epoch: DataEpoch,
    pub gateway_contract_revision: GatewayContractRevision,
    pub backend: BackendKind,
    pub backend_state_digest: BackendStateDigest,
    pub source_release_artifact_digest: Sha256Digest,
    pub source_release_provenance_digest: Sha256Digest,
    pub fixture_artifact_digest: Sha256Digest,
    pub fixture_manifest_digest: Sha256Digest,
    pub expected_facts_digest: Sha256Digest,
    pub oci_reference: String,
    pub created_at: DateTime<Utc>,
    pub immutable_entry_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureCatalog {
    pub format_version: u32,
    pub entries: Vec<FixtureCatalogEntry>,
}

impl Default for FixtureCatalog {
    fn default() -> Self {
        Self {
            format_version: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("SHA-256 digest is invalid")]
    InvalidDigest,
    #[error("fixture path is unsafe: {path}")]
    UnsafePath { path: String },
    #[error("fixture is invalid: {message}")]
    InvalidFixture { message: String },
    #[error("fixture recovery set is incomplete")]
    IncompleteRecoverySet,
    #[error("fixture was not generated by its declared release identity")]
    WrongProducerRelease,
    #[error("expected facts digest or identity differs")]
    ExpectedFactsMismatch,
    #[error("expected facts do not cover every required data domain")]
    ExpectedFactsIncomplete,
    #[error("duplicate identifier in expected facts: {domain}")]
    DuplicateFactId { domain: &'static str },
    #[error("catalog is not append-only")]
    CatalogNotAppendOnly,
    #[error("catalog entry is invalid: {message}")]
    InvalidCatalogEntry { message: String },
    #[error("fixture asset verification failed: {message}")]
    AssetVerification { message: String },
    #[error("verification layer {layer} failed: {message}")]
    LayerFailed { layer: String, message: String },
    #[error("verification report is invalid: {message}")]
    InvalidReport { message: String },
    #[error("I/O error: {message}")]
    Io { message: String },
    #[error("serialization error: {message}")]
    Serialization { message: String },
}

pub(crate) fn validate_fixture_path(path: &str) -> Result<(), EvidenceError> {
    let path_value = std::path::Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || path_value.is_absolute()
        || path_value
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(EvidenceError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn require_unique_ids(
    ids: impl IntoIterator<Item = Uuid>,
    domain: &'static str,
) -> Result<(), EvidenceError> {
    let mut seen = BTreeSet::new();
    if ids.into_iter().any(|id| id.is_nil() || !seen.insert(id)) {
        Err(EvidenceError::DuplicateFactId { domain })
    } else {
        Ok(())
    }
}
