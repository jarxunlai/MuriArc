use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use muriarc_core::{ReleaseIdentity, ReleaseManifest};
use muriarc_delivery::VerifiedServerBundle;
use muriarc_upgrade::{
    ActiveGeneration, BackupEvidence, CandidateEvidence, DeploymentProfile, RestoreEvidence,
    TrustedMetadataVersions, UpgradeSnapshot, VerificationEvidence, VerifiedRecoveryPoint,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub(crate) const DRIVER_PROTOCOL_FORMAT: u32 = 1;
pub(crate) const DRIVER_STATE_FORMAT: u32 = 1;
pub(crate) const RECOVERY_SET_FORMAT: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DriverRequest {
    pub(crate) format_version: u32,
    pub(crate) action: String,
    pub(crate) profile: DeploymentProfile,
    pub(crate) payload: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct DriverResponse<T: Serialize> {
    pub(crate) format_version: u32,
    pub(crate) action: String,
    pub(crate) status: &'static str,
    pub(crate) data: T,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetEnvelope {
    pub(crate) manifest: ReleaseManifest,
    pub(crate) target_name: String,
    pub(crate) target_length: u64,
    pub(crate) target_digest: String,
    pub(crate) metadata_versions: TrustedMetadataVersions,
    pub(crate) metadata_expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationPayload {
    pub(crate) operation_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotPayload {
    pub(crate) snapshot: UpgradeSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotTargetPayload {
    pub(crate) snapshot: UpgradeSnapshot,
    pub(crate) target: TargetEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotBackupPayload {
    pub(crate) snapshot: UpgradeSnapshot,
    pub(crate) backup: BackupEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotRestoreTargetPayload {
    pub(crate) snapshot: UpgradeSnapshot,
    pub(crate) restore: RestoreEvidence,
    pub(crate) target: TargetEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotCandidatePayload {
    pub(crate) snapshot: UpgradeSnapshot,
    pub(crate) candidate: CandidateEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotCandidateTargetPayload {
    pub(crate) snapshot: UpgradeSnapshot,
    pub(crate) candidate: CandidateEvidence,
    pub(crate) target: TargetEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerationPayload {
    pub(crate) generation_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DriverOperationState {
    pub(crate) format_version: u32,
    pub(crate) operation_id: Uuid,
    pub(crate) source_generation_id: Uuid,
    pub(crate) source_database: String,
    pub(crate) source_release_path: PathBuf,
    pub(crate) source_bundle: VerifiedServerBundle,
    pub(crate) target_release_path: Option<PathBuf>,
    pub(crate) target_bundle: Option<VerifiedServerBundle>,
    pub(crate) target_server_image: Option<String>,
    pub(crate) backup: Option<BackupEvidence>,
    pub(crate) recovery_set_digest: Option<String>,
    pub(crate) restore_database: Option<String>,
    pub(crate) restore_database_digest: Option<String>,
    pub(crate) restore_root: Option<PathBuf>,
    pub(crate) candidate_generation_id: Option<Uuid>,
    pub(crate) candidate_database: Option<String>,
    pub(crate) candidate_root: Option<PathBuf>,
    pub(crate) candidate_identity: Option<ReleaseIdentity>,
    pub(crate) candidate_verification: Option<VerificationEvidence>,
    pub(crate) cloudflared_was_active: bool,
    pub(crate) switched: bool,
    pub(crate) write_lease_opened: bool,
    pub(crate) recovered: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl DriverOperationState {
    pub(crate) fn new(
        snapshot: &UpgradeSnapshot,
        source_database: String,
        source_release_path: PathBuf,
        source_bundle: VerifiedServerBundle,
    ) -> Self {
        let now = Utc::now();
        Self {
            format_version: DRIVER_STATE_FORMAT,
            operation_id: snapshot.operation_id,
            source_generation_id: snapshot.source_generation_id,
            source_database,
            source_release_path,
            source_bundle,
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
        }
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.format_version == DRIVER_STATE_FORMAT
                && !self.operation_id.is_nil()
                && !self.source_generation_id.is_nil()
                && !self.source_database.is_empty()
                && self.source_release_path.is_absolute(),
            "driver operation state is invalid"
        );
        if self.switched {
            anyhow::ensure!(
                self.candidate_database.is_some()
                    && self.candidate_generation_id.is_some()
                    && self.target_release_path.is_some(),
                "switched operation state is incomplete"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoverySetEntry {
    pub(crate) path: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoverySetManifest {
    pub(crate) format_version: u32,
    pub(crate) backup_id: Uuid,
    pub(crate) source_generation: ActiveGeneration,
    pub(crate) entries: Vec<RecoverySetEntry>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedBackupResponse {
    pub(crate) backup: BackupEvidence,
    pub(crate) restore: RestoreEvidence,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadOnlyVerificationResponse {
    pub(crate) state_digest_before: String,
    pub(crate) state_digest_after: String,
    pub(crate) verification: VerificationEvidence,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationSelection {
    pub(crate) operation_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StandaloneBackupPayload {
    pub(crate) source_generation: ActiveGeneration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryRestorePayload {
    pub(crate) recovery_point: VerifiedRecoveryPoint,
    pub(crate) confirm_data_loss: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryPrunePayload {
    pub(crate) recovery_point: VerifiedRecoveryPoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreOperationResponse {
    pub(crate) backup_id: Uuid,
    pub(crate) backup_artifact_digest: String,
    pub(crate) restored_generation_id: Uuid,
    pub(crate) data_loss_confirmation_recorded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PruneResponse {
    pub(crate) backup_id: Uuid,
    pub(crate) artifact_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageLock {
    pub(crate) format_version: u32,
    pub(crate) source_commit: String,
    pub(crate) server_image: String,
    pub(crate) postgres_source_image: String,
    pub(crate) postgres_image: String,
    pub(crate) server_image_archive_digest: String,
    pub(crate) postgres_image_archive_digest: String,
    pub(crate) server_signature_bundle_digest: String,
    pub(crate) postgres_signature_bundle_digest: String,
}

pub(crate) type Environment = BTreeMap<String, String>;
