use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use muriarc_core::{BackendKind, MigrationClass, ReleaseIdentity, ReleaseManifest};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const UPGRADE_JOURNAL_VERSION: u32 = 1;
pub const CONTROLLER_PROTOCOL_REVISION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentProfile {
    NativeSystem,
    ManagedCompose,
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradePhase {
    Initialized,
    LocksAcquired,
    PreflightPassed,
    Drained,
    WritesFrozen,
    BackupCreated,
    BackupRestored,
    CandidatePrepared,
    CandidateMigrated,
    CandidateVerified,
    Switched,
    ReadOnlyActivated,
    ActivationVerified,
    WriteLeaseOpened,
    Completed,
}

impl UpgradePhase {
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Initialized => Some(Self::LocksAcquired),
            Self::LocksAcquired => Some(Self::PreflightPassed),
            Self::PreflightPassed => Some(Self::Drained),
            Self::Drained => Some(Self::WritesFrozen),
            Self::WritesFrozen => Some(Self::BackupCreated),
            Self::BackupCreated => Some(Self::BackupRestored),
            Self::BackupRestored => Some(Self::CandidatePrepared),
            Self::CandidatePrepared => Some(Self::CandidateMigrated),
            Self::CandidateMigrated => Some(Self::CandidateVerified),
            Self::CandidateVerified => Some(Self::Switched),
            Self::Switched => Some(Self::ReadOnlyActivated),
            Self::ReadOnlyActivated => Some(Self::ActivationVerified),
            Self::ActivationVerified => Some(Self::WriteLeaseOpened),
            Self::WriteLeaseOpened => Some(Self::Completed),
            Self::Completed => None,
        }
    }

    pub const fn has_switched(self) -> bool {
        matches!(
            self,
            Self::Switched
                | Self::ReadOnlyActivated
                | Self::ActivationVerified
                | Self::WriteLeaseOpened
                | Self::Completed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeStatus {
    Running,
    Succeeded,
    Failed,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedMetadataVersions {
    pub root: u64,
    pub timestamp: u64,
    pub snapshot: u64,
    pub targets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct VerifiedRelease {
    pub manifest: ReleaseManifest,
    pub target_name: String,
    pub target_length: u64,
    pub target_digest: String,
    pub metadata_versions: TrustedMetadataVersions,
    pub metadata_expires_at: DateTime<Utc>,
}

impl VerifiedRelease {
    /// Builds the common controller target after a platform-specific updater
    /// has authenticated both the release manifest and the exact artifact.
    ///
    /// TUF-backed Native/Compose updates are constructed by the TUF client.
    /// Tauri Desktop uses its independent Minisign trust root and has no TUF
    /// timestamp role, so the caller supplies the already verified artifact
    /// identity and the short-lived time until which this check result may be
    /// consumed. The controller still revalidates the manifest, digest, size,
    /// protocol range, and this expiry before touching user data.
    pub fn from_verified_platform_artifact(
        manifest: ReleaseManifest,
        target_name: impl Into<String>,
        target_length: u64,
        target_digest: impl Into<String>,
        verification_expires_at: DateTime<Utc>,
    ) -> Result<Self, UpgradeError> {
        let release = Self {
            manifest,
            target_name: target_name.into(),
            target_length,
            target_digest: target_digest.into(),
            metadata_versions: TrustedMetadataVersions {
                root: 0,
                timestamp: 0,
                snapshot: 0,
                targets: 0,
            },
            metadata_expires_at: verification_expires_at,
        };
        release.validate_for_controller()?;
        Ok(release)
    }

    pub fn validate_for_controller(&self) -> Result<(), UpgradeError> {
        self.manifest
            .validate()
            .map_err(|message| UpgradeError::TargetInvalid { message })?;
        if self.target_name.trim().is_empty()
            || self.target_length == 0
            || !valid_sha256_digest(&self.target_digest)
        {
            return Err(UpgradeError::TargetInvalid {
                message: "verified target identity is incomplete".to_owned(),
            });
        }
        if self.metadata_expires_at <= Utc::now() {
            return Err(UpgradeError::MetadataExpired);
        }
        if CONTROLLER_PROTOCOL_REVISION < self.manifest.controller_protocol_min
            || CONTROLLER_PROTOCOL_REVISION > self.manifest.controller_protocol_max
        {
            return Err(UpgradeError::ControllerProtocolMismatch {
                controller: CONTROLLER_PROTOCOL_REVISION,
                minimum: self.manifest.controller_protocol_min,
                maximum: self.manifest.controller_protocol_max,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_test(manifest: ReleaseManifest) -> Self {
        Self {
            manifest,
            target_name: "muriarc-test-bundle".to_owned(),
            target_length: 1,
            target_digest: format!("sha256:{}", "a".repeat(64)),
            metadata_versions: TrustedMetadataVersions {
                root: 1,
                timestamp: 1,
                snapshot: 1,
                targets: 1,
            },
            metadata_expires_at: Utc::now() + chrono::Duration::hours(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveGeneration {
    pub generation_id: Uuid,
    pub identity: ReleaseIdentity,
    pub backend: BackendKind,
    pub first_write_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightEvidence {
    pub source_generation_id: Uuid,
    pub target_application_version: String,
    pub free_bytes: u64,
    pub required_bytes: u64,
    pub maintenance_class: MigrationClass,
    pub recovery_prerequisites_satisfied: bool,
    pub checked_at: DateTime<Utc>,
}

impl PreflightEvidence {
    pub fn validate(&self, source: Uuid, release: &VerifiedRelease) -> Result<(), UpgradeError> {
        if self.source_generation_id != source
            || self.target_application_version != release.manifest.application_version.as_str()
            || self.maintenance_class != release.manifest.migration_class
            || self.free_bytes < self.required_bytes
            || !self.recovery_prerequisites_satisfied
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::PreflightPassed,
                message: "preflight evidence does not satisfy target prerequisites".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainEvidence {
    pub inflight_requests: u64,
    pub running_jobs: u64,
    pub pending_attachment_writes: u64,
    pub provider_requests: u64,
    pub drained_at: DateTime<Utc>,
}

impl DrainEvidence {
    pub fn validate(&self) -> Result<(), UpgradeError> {
        if self.inflight_requests != 0
            || self.running_jobs != 0
            || self.pending_attachment_writes != 0
            || self.provider_requests != 0
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::Drained,
                message: "drain completed with live writers or provider requests".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeEvidence {
    pub source_generation_id: Uuid,
    pub revoked_lease_id: Uuid,
    pub fencing_token: i64,
    pub frozen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryComponent {
    Database,
    Attachments,
    DataArtifacts,
    Configuration,
    Keyset,
    AiState,
    GenerationManifest,
}

impl RecoveryComponent {
    pub fn required() -> BTreeSet<Self> {
        BTreeSet::from([
            Self::Database,
            Self::Attachments,
            Self::DataArtifacts,
            Self::Configuration,
            Self::Keyset,
            Self::AiState,
            Self::GenerationManifest,
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEvidence {
    pub backup_id: Uuid,
    pub source_generation_id: Uuid,
    pub artifact_digest: String,
    pub recovery_set_digest: String,
    pub components: BTreeSet<RecoveryComponent>,
    pub created_at: DateTime<Utc>,
}

impl BackupEvidence {
    pub fn validate(&self, source_generation_id: Uuid) -> Result<(), UpgradeError> {
        if self.source_generation_id != source_generation_id
            || !valid_sha256_digest(&self.artifact_digest)
            || !valid_sha256_digest(&self.recovery_set_digest)
            || self.components != RecoveryComponent::required()
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::BackupCreated,
                message: "backup does not cover the complete recovery set".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreEvidence {
    pub backup_id: Uuid,
    pub backup_artifact_digest: String,
    pub restored_generation_id: Uuid,
    pub isolated_restore: bool,
    pub verified_at: DateTime<Utc>,
}

impl RestoreEvidence {
    pub fn validate(&self, backup: &BackupEvidence) -> Result<(), UpgradeError> {
        if self.backup_id != backup.backup_id
            || self.backup_artifact_digest != backup.artifact_digest
            || !self.isolated_restore
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::BackupRestored,
                message: "backup was not verified through an isolated restore".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub generation_id: Uuid,
    pub isolated: bool,
    pub private_endpoint: bool,
    pub external_providers_disabled: bool,
    pub background_jobs_disabled: bool,
    pub real_user_writes_disabled: bool,
    pub prepared_at: DateTime<Utc>,
}

impl CandidateEvidence {
    pub fn validate(&self) -> Result<(), UpgradeError> {
        if !self.isolated
            || !self.private_endpoint
            || !self.external_providers_disabled
            || !self.background_jobs_disabled
            || !self.real_user_writes_disabled
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::CandidatePrepared,
                message: "candidate isolation controls are incomplete".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationEvidence {
    pub generation_id: Uuid,
    pub identity: ReleaseIdentity,
    pub migration_path: Vec<String>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationLayer {
    AssetRestore,
    Storage,
    StoreApplication,
    Api,
    RemoteUi,
    ContinueWrite,
    ReadOnlyNoSideEffects,
}

impl VerificationLayer {
    pub fn required() -> BTreeSet<Self> {
        BTreeSet::from([
            Self::AssetRestore,
            Self::Storage,
            Self::StoreApplication,
            Self::Api,
            Self::RemoteUi,
            Self::ContinueWrite,
            Self::ReadOnlyNoSideEffects,
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationLayerEvidence {
    pub evidence_digest: String,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub generation_id: Uuid,
    pub layers: BTreeMap<VerificationLayer, VerificationLayerEvidence>,
}

impl VerificationEvidence {
    pub fn validate(&self) -> Result<(), UpgradeError> {
        if self.layers.keys().copied().collect::<BTreeSet<_>>() != VerificationLayer::required()
            || self
                .layers
                .values()
                .any(|layer| !valid_sha256_digest(&layer.evidence_digest))
        {
            return Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::CandidateVerified,
                message: "all seven verifier layers must pass with pinned evidence".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchEvidence {
    pub source_generation_id: Uuid,
    pub candidate_generation_id: Uuid,
    pub atomic: bool,
    pub switched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOnlyActivationEvidence {
    pub generation_id: Uuid,
    pub write_lease_absent: bool,
    pub external_traffic_blocked: bool,
    pub activated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationVerificationEvidence {
    pub generation_id: Uuid,
    pub readiness_verified: bool,
    pub compatibility_verified: bool,
    pub no_write_side_effects: bool,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLeaseEvidence {
    pub generation_id: Uuid,
    pub lease_id: Uuid,
    pub fencing_token: i64,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeEvidence {
    pub preflight: Option<PreflightEvidence>,
    pub drain: Option<DrainEvidence>,
    pub freeze: Option<FreezeEvidence>,
    pub backup: Option<BackupEvidence>,
    pub restore: Option<RestoreEvidence>,
    pub candidate: Option<CandidateEvidence>,
    pub migration: Option<MigrationEvidence>,
    pub candidate_verification: Option<VerificationEvidence>,
    pub switch: Option<SwitchEvidence>,
    pub read_only_activation: Option<ReadOnlyActivationEvidence>,
    pub activation_verification: Option<ActivationVerificationEvidence>,
    pub write_lease: Option<WriteLeaseEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeSnapshot {
    pub journal_version: u32,
    pub revision: u64,
    pub operation_id: Uuid,
    pub profile: DeploymentProfile,
    pub source_generation_id: Uuid,
    pub source_identity: ReleaseIdentity,
    pub candidate_generation_id: Option<Uuid>,
    pub target_application_version: String,
    pub target_data_epoch: String,
    pub target_backend_state_digest: String,
    pub target_gateway_contract_revision: String,
    pub maintenance_class: MigrationClass,
    pub phase: UpgradePhase,
    pub status: UpgradeStatus,
    pub evidence: UpgradeEvidence,
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl UpgradeSnapshot {
    pub fn new(
        operation_id: Uuid,
        profile: DeploymentProfile,
        source: &ActiveGeneration,
        target: &VerifiedRelease,
    ) -> Result<Self, UpgradeError> {
        let target_digest = target
            .manifest
            .backend_states
            .get(&source.backend)
            .ok_or_else(|| UpgradeError::TargetInvalid {
                message: format!("target has no {} backend state", source.backend.as_str()),
            })?;
        let now = Utc::now();
        Ok(Self {
            journal_version: UPGRADE_JOURNAL_VERSION,
            revision: 0,
            operation_id,
            profile,
            source_generation_id: source.generation_id,
            source_identity: source.identity.clone(),
            candidate_generation_id: None,
            target_application_version: target.manifest.application_version.to_string(),
            target_data_epoch: target.manifest.data_epoch.to_string(),
            target_backend_state_digest: target_digest.to_string(),
            target_gateway_contract_revision: target.manifest.gateway_contract_revision.to_string(),
            maintenance_class: target.manifest.migration_class,
            phase: UpgradePhase::Initialized,
            status: UpgradeStatus::Running,
            evidence: UpgradeEvidence::default(),
            failure_code: None,
            failure_detail: None,
            started_at: now,
            updated_at: now,
            completed_at: None,
        })
    }

    pub fn advance(&mut self, next: UpgradePhase) -> Result<(), UpgradeError> {
        if self.status != UpgradeStatus::Running || self.phase.next() != Some(next) {
            return Err(UpgradeError::InvalidTransition {
                from: self.phase,
                to: next,
            });
        }
        self.phase = next;
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
        if next == UpgradePhase::Completed {
            self.status = UpgradeStatus::Succeeded;
            self.completed_at = Some(self.updated_at);
        }
        Ok(())
    }

    pub fn mark_failed(&mut self, error: &UpgradeError, recovery_required: bool) {
        self.status = if recovery_required {
            UpgradeStatus::RecoveryRequired
        } else {
            UpgradeStatus::Failed
        };
        self.failure_code = Some(error.code().to_owned());
        self.failure_detail = Some(error.safe_detail());
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
        self.completed_at = Some(self.updated_at);
    }
}

#[derive(Debug, Error)]
pub enum UpgradeError {
    #[error("upgrade lock is already held")]
    LockBusy,
    #[error("upgrade target is invalid: {message}")]
    TargetInvalid { message: String },
    #[error("trusted metadata is expired")]
    MetadataExpired,
    #[error("trusted metadata rollback was detected for {role}: {observed} < {trusted}")]
    MetadataRollback {
        role: String,
        observed: u64,
        trusted: u64,
    },
    #[error("trusted metadata signature threshold was not met for {role}")]
    SignatureThreshold { role: String },
    #[error("controller protocol {controller} is outside target range {minimum}..={maximum}")]
    ControllerProtocolMismatch {
        controller: u32,
        minimum: u32,
        maximum: u32,
    },
    #[error("invalid state transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: UpgradePhase,
        to: UpgradePhase,
    },
    #[error("invalid evidence for {phase:?}: {message}")]
    EvidenceInvalid {
        phase: UpgradePhase,
        message: String,
    },
    #[error("operation {operation_id} was not found")]
    OperationNotFound { operation_id: Uuid },
    #[error("operation persistence failed: {message}")]
    Persistence { message: String },
    #[error("driver prerequisite failed: {message}")]
    Prerequisite { message: String },
    #[error("driver phase failed at {phase:?}: {message}")]
    Driver {
        phase: UpgradePhase,
        message: String,
    },
    #[error("automatic downgrade is forbidden after first write at {first_write_at}")]
    FirstWriteBlocksRollback { first_write_at: DateTime<Utc> },
    #[error("journal integrity verification failed: {message}")]
    JournalIntegrity { message: String },
    #[error("artifact verification failed: {message}")]
    ArtifactVerification { message: String },
    #[error("invalid command: {message}")]
    InvalidCommand { message: String },
}

impl UpgradeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::LockBusy => "upgrade_lock_busy",
            Self::TargetInvalid { .. } => "target_invalid",
            Self::MetadataExpired => "metadata_expired",
            Self::MetadataRollback { .. } => "metadata_rollback",
            Self::SignatureThreshold { .. } => "signature_threshold",
            Self::ControllerProtocolMismatch { .. } => "controller_protocol_mismatch",
            Self::InvalidTransition { .. } => "invalid_transition",
            Self::EvidenceInvalid { .. } => "evidence_invalid",
            Self::OperationNotFound { .. } => "operation_not_found",
            Self::Persistence { .. } => "persistence_failed",
            Self::Prerequisite { .. } => "prerequisite_failed",
            Self::Driver { .. } => "driver_failed",
            Self::FirstWriteBlocksRollback { .. } => "first_write_blocks_rollback",
            Self::JournalIntegrity { .. } => "journal_integrity_failed",
            Self::ArtifactVerification { .. } => "artifact_verification_failed",
            Self::InvalidCommand { .. } => "invalid_command",
        }
    }

    pub fn safe_detail(&self) -> String {
        self.to_string()
    }
}

pub(crate) fn valid_sha256_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
