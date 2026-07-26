use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    ActivationVerificationEvidence, ActiveGeneration, BackupEvidence, CandidateEvidence,
    DrainEvidence, FreezeEvidence, HostUpgradeLock, MigrationEvidence, PreflightEvidence,
    ReadOnlyActivationEvidence, RecoveryPointCatalog, RestoreEvidence, SwitchEvidence,
    UpgradeError, UpgradeJournal, UpgradePhase, UpgradeSnapshot, UpgradeStatus,
    VerificationEvidence, VerifiedRelease, WriteLeaseEvidence,
};

pub trait BackendUpgradeLock: Send + Sync {}

impl<T: Send + Sync> BackendUpgradeLock for T {}

/// Deployment-specific side effects. Implementations must be idempotent for the
/// same operation/revision because `resume` may repeat the last unpersisted call.
#[async_trait]
pub trait UpgradeDriver: Send + Sync {
    fn profile(&self) -> crate::DeploymentProfile;

    async fn acquire_backend_lock(
        &self,
        operation_id: Uuid,
    ) -> Result<Box<dyn BackendUpgradeLock>, UpgradeError>;
    async fn current_generation(&self) -> Result<ActiveGeneration, UpgradeError>;
    async fn create_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError>;
    async fn save_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError>;
    async fn load_operation(&self, operation_id: Uuid) -> Result<UpgradeSnapshot, UpgradeError>;

    async fn preflight(
        &self,
        snapshot: &UpgradeSnapshot,
        target: &VerifiedRelease,
    ) -> Result<PreflightEvidence, UpgradeError>;
    async fn drain(&self, snapshot: &UpgradeSnapshot) -> Result<DrainEvidence, UpgradeError>;
    async fn freeze_writes(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<FreezeEvidence, UpgradeError>;
    async fn create_backup(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<BackupEvidence, UpgradeError>;
    async fn verify_backup_restore(
        &self,
        snapshot: &UpgradeSnapshot,
        backup: &BackupEvidence,
    ) -> Result<RestoreEvidence, UpgradeError>;
    async fn prepare_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        restore: &RestoreEvidence,
        target: &VerifiedRelease,
    ) -> Result<CandidateEvidence, UpgradeError>;
    async fn migrate_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
        target: &VerifiedRelease,
    ) -> Result<MigrationEvidence, UpgradeError>;
    async fn verify_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<VerificationEvidence, UpgradeError>;
    async fn switch_generation(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<SwitchEvidence, UpgradeError>;
    async fn activate_read_only(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ReadOnlyActivationEvidence, UpgradeError>;
    async fn verify_activated(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ActivationVerificationEvidence, UpgradeError>;
    async fn open_write_lease(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<WriteLeaseEvidence, UpgradeError>;

    async fn first_write_at(
        &self,
        generation_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, UpgradeError>;
    async fn recover_before_first_write(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<(), UpgradeError>;
}

pub struct UpgradeEngine<D> {
    driver: Arc<D>,
    state_root: PathBuf,
}

impl<D> UpgradeEngine<D>
where
    D: UpgradeDriver + 'static,
{
    pub fn new(driver: Arc<D>, state_root: impl Into<PathBuf>) -> Self {
        Self {
            driver,
            state_root: state_root.into(),
        }
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub async fn run(&self, target: VerifiedRelease) -> Result<UpgradeSnapshot, UpgradeError> {
        target.validate_for_controller()?;
        let operation_id = Uuid::new_v4();
        let _host_lock = HostUpgradeLock::acquire(&self.state_root, operation_id)?;
        let _backend_lock = self.driver.acquire_backend_lock(operation_id).await?;
        let source = self.driver.current_generation().await?;
        let mut snapshot =
            UpgradeSnapshot::new(operation_id, self.driver.profile(), &source, &target)?;
        snapshot.advance(UpgradePhase::LocksAcquired)?;
        self.driver.create_operation(&snapshot).await?;
        let journal = UpgradeJournal::new(&self.state_root, operation_id)?;
        journal.append(&snapshot)?;
        self.continue_operation(snapshot, &target, &journal).await
    }

    pub async fn resume(
        &self,
        operation_id: Uuid,
        target: VerifiedRelease,
    ) -> Result<UpgradeSnapshot, UpgradeError> {
        target.validate_for_controller()?;
        let _host_lock = HostUpgradeLock::acquire_for_resume(&self.state_root, operation_id)?;
        let _backend_lock = self.driver.acquire_backend_lock(operation_id).await?;
        let snapshot = self.driver.load_operation(operation_id).await?;
        if snapshot.status != UpgradeStatus::Running {
            return Err(UpgradeError::Prerequisite {
                message: format!("operation status is {:?}, not running", snapshot.status),
            });
        }
        ensure_target_matches(&snapshot, &target)?;
        let journal = UpgradeJournal::new(&self.state_root, operation_id)?;
        if let Some(local) = journal.latest()? {
            if local.revision > snapshot.revision
                || (local.revision == snapshot.revision && local != snapshot)
            {
                return Err(UpgradeError::JournalIntegrity {
                    message: "local Journal is ahead of or conflicts with persistent state"
                        .to_owned(),
                });
            }
        }
        if journal.latest()?.as_ref() != Some(&snapshot) {
            journal.append(&snapshot)?;
        }
        self.continue_operation(snapshot, &target, &journal).await
    }

    async fn continue_operation(
        &self,
        mut snapshot: UpgradeSnapshot,
        target: &VerifiedRelease,
        journal: &UpgradeJournal,
    ) -> Result<UpgradeSnapshot, UpgradeError> {
        let result = self
            .run_remaining_steps(&mut snapshot, target, journal)
            .await;
        match result {
            Ok(()) => Ok(snapshot),
            Err(error) => {
                self.handle_failure(&mut snapshot, &error, journal).await;
                Err(error)
            }
        }
    }

    async fn run_remaining_steps(
        &self,
        snapshot: &mut UpgradeSnapshot,
        target: &VerifiedRelease,
        journal: &UpgradeJournal,
    ) -> Result<(), UpgradeError> {
        loop {
            match snapshot.phase {
                UpgradePhase::Initialized => {
                    return Err(UpgradeError::InvalidTransition {
                        from: UpgradePhase::Initialized,
                        to: UpgradePhase::PreflightPassed,
                    });
                }
                UpgradePhase::LocksAcquired => {
                    let evidence = self.driver.preflight(snapshot, target).await?;
                    evidence.validate(snapshot.source_generation_id, target)?;
                    snapshot.evidence.preflight = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::PreflightPassed, journal)
                        .await?;
                }
                UpgradePhase::PreflightPassed => {
                    let evidence = self.driver.drain(snapshot).await?;
                    evidence.validate()?;
                    snapshot.evidence.drain = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::Drained, journal)
                        .await?;
                }
                UpgradePhase::Drained => {
                    let evidence = self.driver.freeze_writes(snapshot).await?;
                    if evidence.source_generation_id != snapshot.source_generation_id
                        || evidence.fencing_token <= 0
                    {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::WritesFrozen,
                            message: "write freeze did not revoke the source fencing lease"
                                .to_owned(),
                        });
                    }
                    snapshot.evidence.freeze = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::WritesFrozen, journal)
                        .await?;
                }
                UpgradePhase::WritesFrozen => {
                    let evidence = self.driver.create_backup(snapshot).await?;
                    evidence.validate(snapshot.source_generation_id)?;
                    snapshot.evidence.backup = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::BackupCreated, journal)
                        .await?;
                }
                UpgradePhase::BackupCreated => {
                    let backup = snapshot.evidence.backup.as_ref().ok_or_else(|| {
                        UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::BackupCreated,
                            message: "backup evidence is missing from persisted operation"
                                .to_owned(),
                        }
                    })?;
                    let evidence = self.driver.verify_backup_restore(snapshot, backup).await?;
                    evidence.validate(backup)?;
                    let mut recovery_catalog = RecoveryPointCatalog::load(&self.state_root)?;
                    recovery_catalog.register_verified(backup.clone(), evidence.clone())?;
                    recovery_catalog.save_atomic(&self.state_root)?;
                    snapshot.evidence.restore = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::BackupRestored, journal)
                        .await?;
                }
                UpgradePhase::BackupRestored => {
                    let restore = snapshot.evidence.restore.as_ref().ok_or_else(|| {
                        UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::BackupRestored,
                            message: "restore evidence is missing from persisted operation"
                                .to_owned(),
                        }
                    })?;
                    let evidence = self
                        .driver
                        .prepare_candidate(snapshot, restore, target)
                        .await?;
                    evidence.validate()?;
                    snapshot.candidate_generation_id = Some(evidence.generation_id);
                    snapshot.evidence.candidate = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::CandidatePrepared, journal)
                        .await?;
                }
                UpgradePhase::CandidatePrepared => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self
                        .driver
                        .migrate_candidate(snapshot, candidate, target)
                        .await?;
                    ensure_migration_matches(snapshot, candidate, &evidence)?;
                    snapshot.evidence.migration = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::CandidateMigrated, journal)
                        .await?;
                }
                UpgradePhase::CandidateMigrated => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self.driver.verify_candidate(snapshot, candidate).await?;
                    evidence.validate()?;
                    if evidence.generation_id != candidate.generation_id {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::CandidateVerified,
                            message: "verification belongs to another generation".to_owned(),
                        });
                    }
                    snapshot.evidence.candidate_verification = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::CandidateVerified, journal)
                        .await?;
                }
                UpgradePhase::CandidateVerified => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self.driver.switch_generation(snapshot, candidate).await?;
                    if !evidence.atomic
                        || evidence.source_generation_id != snapshot.source_generation_id
                        || evidence.candidate_generation_id != candidate.generation_id
                    {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::Switched,
                            message: "generation switch was not atomic or identities differ"
                                .to_owned(),
                        });
                    }
                    snapshot.evidence.switch = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::Switched, journal)
                        .await?;
                }
                UpgradePhase::Switched => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self.driver.activate_read_only(snapshot, candidate).await?;
                    if evidence.generation_id != candidate.generation_id
                        || !evidence.write_lease_absent
                        || !evidence.external_traffic_blocked
                    {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::ReadOnlyActivated,
                            message: "target was not activated behind a read-only traffic gate"
                                .to_owned(),
                        });
                    }
                    snapshot.evidence.read_only_activation = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::ReadOnlyActivated, journal)
                        .await?;
                }
                UpgradePhase::ReadOnlyActivated => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self.driver.verify_activated(snapshot, candidate).await?;
                    if evidence.generation_id != candidate.generation_id
                        || !evidence.readiness_verified
                        || !evidence.compatibility_verified
                        || !evidence.no_write_side_effects
                    {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::ActivationVerified,
                            message: "read-only activation verification is incomplete".to_owned(),
                        });
                    }
                    snapshot.evidence.activation_verification = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::ActivationVerified, journal)
                        .await?;
                }
                UpgradePhase::ActivationVerified => {
                    let candidate = candidate_evidence(snapshot)?;
                    let evidence = self.driver.open_write_lease(snapshot, candidate).await?;
                    if evidence.generation_id != candidate.generation_id
                        || evidence.fencing_token <= 0
                        || evidence.expires_at <= Utc::now()
                    {
                        return Err(UpgradeError::EvidenceInvalid {
                            phase: UpgradePhase::WriteLeaseOpened,
                            message: "new write lease is invalid".to_owned(),
                        });
                    }
                    snapshot.evidence.write_lease = Some(evidence);
                    self.advance_and_persist(snapshot, UpgradePhase::WriteLeaseOpened, journal)
                        .await?;
                }
                UpgradePhase::WriteLeaseOpened => {
                    self.advance_and_persist(snapshot, UpgradePhase::Completed, journal)
                        .await?;
                }
                UpgradePhase::Completed => return Ok(()),
            }
        }
    }

    async fn advance_and_persist(
        &self,
        snapshot: &mut UpgradeSnapshot,
        phase: UpgradePhase,
        journal: &UpgradeJournal,
    ) -> Result<(), UpgradeError> {
        snapshot.advance(phase)?;
        self.driver.save_operation(snapshot).await?;
        journal.append(snapshot)?;
        Ok(())
    }

    async fn handle_failure(
        &self,
        snapshot: &mut UpgradeSnapshot,
        error: &UpgradeError,
        journal: &UpgradeJournal,
    ) {
        let first_write = if snapshot.phase.has_switched() {
            match snapshot.candidate_generation_id {
                Some(generation_id) => self
                    .driver
                    .first_write_at(generation_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        } else {
            None
        };
        let mut recovery_required = first_write.is_some();
        let recorded_error = first_write.map_or_else(
            || error.safe_detail(),
            |first_write_at| {
                UpgradeError::FirstWriteBlocksRollback { first_write_at }.safe_detail()
            },
        );
        if !recovery_required
            && snapshot.phase >= UpgradePhase::WritesFrozen
            && self
                .driver
                .recover_before_first_write(snapshot)
                .await
                .is_err()
        {
            recovery_required = true;
        }
        snapshot.mark_failed(error, recovery_required);
        snapshot.failure_detail = Some(recorded_error);
        let _ = self.driver.save_operation(snapshot).await;
        let _ = journal.append(snapshot);
    }
}

fn candidate_evidence(snapshot: &UpgradeSnapshot) -> Result<&CandidateEvidence, UpgradeError> {
    snapshot
        .evidence
        .candidate
        .as_ref()
        .ok_or_else(|| UpgradeError::EvidenceInvalid {
            phase: UpgradePhase::CandidatePrepared,
            message: "candidate evidence is missing from persisted operation".to_owned(),
        })
}

fn ensure_target_matches(
    snapshot: &UpgradeSnapshot,
    target: &VerifiedRelease,
) -> Result<(), UpgradeError> {
    let matches = snapshot.target_application_version
        == target.manifest.application_version.as_str()
        && snapshot.target_data_epoch == target.manifest.data_epoch.as_str()
        && snapshot.target_gateway_contract_revision
            == target.manifest.gateway_contract_revision.as_str()
        && target
            .manifest
            .backend_states
            .values()
            .any(|digest| digest.as_str() == snapshot.target_backend_state_digest);
    if matches {
        Ok(())
    } else {
        Err(UpgradeError::TargetInvalid {
            message: "resume target differs from persisted operation".to_owned(),
        })
    }
}

fn ensure_migration_matches(
    snapshot: &UpgradeSnapshot,
    candidate: &CandidateEvidence,
    migration: &MigrationEvidence,
) -> Result<(), UpgradeError> {
    if migration.generation_id != candidate.generation_id
        || migration.identity.application_version.as_str() != snapshot.target_application_version
        || migration.identity.data_epoch.as_str() != snapshot.target_data_epoch
        || migration.identity.backend_state_digest.as_str() != snapshot.target_backend_state_digest
        || migration.identity.gateway_contract_revision.as_str()
            != snapshot.target_gateway_contract_revision
        || migration.migration_path.is_empty()
    {
        return Err(UpgradeError::EvidenceInvalid {
            phase: UpgradePhase::CandidateMigrated,
            message: "candidate migration identity does not match the signed target".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::Duration;
    use muriarc_core::{
        BackendKind, BackendStateDigest, MigrationClass, ReleaseArtifact, ReleaseIdentity,
        ReleaseManifest,
    };
    use tokio::sync::Mutex;

    use super::*;
    use crate::{
        DeploymentProfile, RecoveryComponent, VerificationLayer, VerificationLayerEvidence,
    };

    #[derive(Default)]
    struct FakeState {
        operations: BTreeMap<Uuid, UpgradeSnapshot>,
        fail_phase: Option<UpgradePhase>,
        incomplete_backup: bool,
        first_write_at: Option<DateTime<Utc>>,
        recovered: bool,
    }

    struct FakeDriver {
        source: ActiveGeneration,
        state: Mutex<FakeState>,
    }

    impl FakeDriver {
        fn new() -> Self {
            Self {
                source: ActiveGeneration {
                    generation_id: Uuid::new_v4(),
                    identity: ReleaseIdentity::parse(
                        "0.1.0".to_owned(),
                        "preview_epoch_0".to_owned(),
                        format!("sha256:{}", "c".repeat(64)),
                        "gateway-v1".to_owned(),
                    )
                    .unwrap(),
                    backend: BackendKind::Postgres,
                    first_write_at: None,
                },
                state: Mutex::new(FakeState::default()),
            }
        }

        async fn fail_if_requested(&self, phase: UpgradePhase) -> Result<(), UpgradeError> {
            if self.state.lock().await.fail_phase == Some(phase) {
                Err(UpgradeError::Driver {
                    phase,
                    message: "injected failure".to_owned(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl UpgradeDriver for FakeDriver {
        fn profile(&self) -> DeploymentProfile {
            DeploymentProfile::NativeSystem
        }

        async fn acquire_backend_lock(
            &self,
            _operation_id: Uuid,
        ) -> Result<Box<dyn BackendUpgradeLock>, UpgradeError> {
            Ok(Box::new(()))
        }

        async fn current_generation(&self) -> Result<ActiveGeneration, UpgradeError> {
            Ok(self.source.clone())
        }

        async fn create_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
            self.state
                .lock()
                .await
                .operations
                .insert(snapshot.operation_id, snapshot.clone());
            Ok(())
        }

        async fn save_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
            self.state
                .lock()
                .await
                .operations
                .insert(snapshot.operation_id, snapshot.clone());
            Ok(())
        }

        async fn load_operation(
            &self,
            operation_id: Uuid,
        ) -> Result<UpgradeSnapshot, UpgradeError> {
            self.state
                .lock()
                .await
                .operations
                .get(&operation_id)
                .cloned()
                .ok_or(UpgradeError::OperationNotFound { operation_id })
        }

        async fn preflight(
            &self,
            snapshot: &UpgradeSnapshot,
            _target: &VerifiedRelease,
        ) -> Result<PreflightEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::PreflightPassed)
                .await?;
            Ok(PreflightEvidence {
                source_generation_id: snapshot.source_generation_id,
                target_application_version: snapshot.target_application_version.clone(),
                free_bytes: 100,
                required_bytes: 10,
                maintenance_class: snapshot.maintenance_class,
                recovery_prerequisites_satisfied: true,
                checked_at: Utc::now(),
            })
        }

        async fn drain(&self, _snapshot: &UpgradeSnapshot) -> Result<DrainEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::Drained).await?;
            Ok(DrainEvidence {
                inflight_requests: 0,
                running_jobs: 0,
                pending_attachment_writes: 0,
                provider_requests: 0,
                drained_at: Utc::now(),
            })
        }

        async fn freeze_writes(
            &self,
            snapshot: &UpgradeSnapshot,
        ) -> Result<FreezeEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::WritesFrozen).await?;
            Ok(FreezeEvidence {
                source_generation_id: snapshot.source_generation_id,
                revoked_lease_id: Uuid::new_v4(),
                fencing_token: 1,
                frozen_at: Utc::now(),
            })
        }

        async fn create_backup(
            &self,
            snapshot: &UpgradeSnapshot,
        ) -> Result<BackupEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::BackupCreated).await?;
            let components = if self.state.lock().await.incomplete_backup {
                BTreeSet::from([RecoveryComponent::Database])
            } else {
                RecoveryComponent::required()
            };
            Ok(BackupEvidence {
                backup_id: Uuid::new_v4(),
                source_generation_id: snapshot.source_generation_id,
                artifact_digest: format!("sha256:{}", "d".repeat(64)),
                recovery_set_digest: format!("sha256:{}", "e".repeat(64)),
                components,
                created_at: Utc::now(),
            })
        }

        async fn verify_backup_restore(
            &self,
            _snapshot: &UpgradeSnapshot,
            backup: &BackupEvidence,
        ) -> Result<RestoreEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::BackupRestored).await?;
            Ok(RestoreEvidence {
                backup_id: backup.backup_id,
                backup_artifact_digest: backup.artifact_digest.clone(),
                restored_generation_id: Uuid::new_v4(),
                isolated_restore: true,
                verified_at: Utc::now(),
            })
        }

        async fn prepare_candidate(
            &self,
            _snapshot: &UpgradeSnapshot,
            restore: &RestoreEvidence,
            _target: &VerifiedRelease,
        ) -> Result<CandidateEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::CandidatePrepared)
                .await?;
            Ok(CandidateEvidence {
                generation_id: restore.restored_generation_id,
                isolated: true,
                private_endpoint: true,
                external_providers_disabled: true,
                background_jobs_disabled: true,
                real_user_writes_disabled: true,
                prepared_at: Utc::now(),
            })
        }

        async fn migrate_candidate(
            &self,
            snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
            _target: &VerifiedRelease,
        ) -> Result<MigrationEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::CandidateMigrated)
                .await?;
            Ok(MigrationEvidence {
                generation_id: candidate.generation_id,
                identity: ReleaseIdentity::parse(
                    snapshot.target_application_version.clone(),
                    snapshot.target_data_epoch.clone(),
                    snapshot.target_backend_state_digest.clone(),
                    snapshot.target_gateway_contract_revision.clone(),
                )
                .unwrap(),
                migration_path: vec!["preview_epoch_0->E0001".to_owned()],
                completed_at: Utc::now(),
            })
        }

        async fn verify_candidate(
            &self,
            _snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
        ) -> Result<VerificationEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::CandidateVerified)
                .await?;
            Ok(VerificationEvidence {
                generation_id: candidate.generation_id,
                layers: VerificationLayer::required()
                    .into_iter()
                    .map(|layer| {
                        (
                            layer,
                            VerificationLayerEvidence {
                                evidence_digest: format!("sha256:{}", "f".repeat(64)),
                                verified_at: Utc::now(),
                            },
                        )
                    })
                    .collect(),
            })
        }

        async fn switch_generation(
            &self,
            snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
        ) -> Result<SwitchEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::Switched).await?;
            Ok(SwitchEvidence {
                source_generation_id: snapshot.source_generation_id,
                candidate_generation_id: candidate.generation_id,
                atomic: true,
                switched_at: Utc::now(),
            })
        }

        async fn activate_read_only(
            &self,
            _snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
        ) -> Result<ReadOnlyActivationEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::ReadOnlyActivated)
                .await?;
            Ok(ReadOnlyActivationEvidence {
                generation_id: candidate.generation_id,
                write_lease_absent: true,
                external_traffic_blocked: true,
                activated_at: Utc::now(),
            })
        }

        async fn verify_activated(
            &self,
            _snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
        ) -> Result<ActivationVerificationEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::ActivationVerified)
                .await?;
            Ok(ActivationVerificationEvidence {
                generation_id: candidate.generation_id,
                readiness_verified: true,
                compatibility_verified: true,
                no_write_side_effects: true,
                verified_at: Utc::now(),
            })
        }

        async fn open_write_lease(
            &self,
            _snapshot: &UpgradeSnapshot,
            candidate: &CandidateEvidence,
        ) -> Result<WriteLeaseEvidence, UpgradeError> {
            self.fail_if_requested(UpgradePhase::WriteLeaseOpened)
                .await?;
            Ok(WriteLeaseEvidence {
                generation_id: candidate.generation_id,
                lease_id: Uuid::new_v4(),
                fencing_token: 2,
                expires_at: Utc::now() + Duration::hours(1),
            })
        }

        async fn first_write_at(
            &self,
            _generation_id: Uuid,
        ) -> Result<Option<DateTime<Utc>>, UpgradeError> {
            Ok(self.state.lock().await.first_write_at)
        }

        async fn recover_before_first_write(
            &self,
            _snapshot: &UpgradeSnapshot,
        ) -> Result<(), UpgradeError> {
            self.state.lock().await.recovered = true;
            Ok(())
        }
    }

    fn target() -> VerifiedRelease {
        let digest: BackendStateDigest = format!("sha256:{}", "a".repeat(64)).parse().unwrap();
        VerifiedRelease::for_test(ReleaseManifest {
            format_version: 1,
            application_version: "1.0.0".parse().unwrap(),
            data_epoch: "E0001".parse().unwrap(),
            gateway_contract_revision: "gateway-v1".parse().unwrap(),
            backend_states: BTreeMap::from([
                (BackendKind::Sqlite, digest.clone()),
                (BackendKind::Postgres, digest.clone()),
            ]),
            postgres_major: 17,
            bootstrap_protocol_revision: 1,
            controller_protocol_min: 1,
            controller_protocol_max: 1,
            migration_class: MigrationClass::M3,
            artifacts: BTreeMap::from([(
                "test".to_owned(),
                ReleaseArtifact {
                    media_type: "application/octet-stream".to_owned(),
                    digest,
                    size_bytes: 1,
                },
            )]),
        })
    }

    #[tokio::test]
    async fn complete_upgrade_requires_every_evidence_gate() {
        let driver = Arc::new(FakeDriver::new());
        let root = tempfile::tempdir().unwrap();
        let engine = UpgradeEngine::new(driver.clone(), root.path());
        let completed = engine.run(target()).await.unwrap();
        assert_eq!(completed.phase, UpgradePhase::Completed);
        assert_eq!(completed.status, UpgradeStatus::Succeeded);
        assert!(completed.evidence.write_lease.is_some());
        assert!(!driver.state.lock().await.recovered);
        assert!(HostUpgradeLock::inspect(root.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn incomplete_recovery_set_fails_before_candidate_and_recovers_source() {
        let driver = Arc::new(FakeDriver::new());
        driver.state.lock().await.incomplete_backup = true;
        let root = tempfile::tempdir().unwrap();
        let engine = UpgradeEngine::new(driver.clone(), root.path());
        assert!(matches!(
            engine.run(target()).await,
            Err(UpgradeError::EvidenceInvalid {
                phase: UpgradePhase::BackupCreated,
                ..
            })
        ));
        let state = driver.state.lock().await;
        let saved = state.operations.values().next().unwrap();
        assert_eq!(saved.status, UpgradeStatus::Failed);
        assert!(state.recovered);
        assert!(saved.candidate_generation_id.is_none());
    }

    #[tokio::test]
    async fn first_target_write_forbids_automatic_rollback() {
        let driver = Arc::new(FakeDriver::new());
        {
            let mut state = driver.state.lock().await;
            state.fail_phase = Some(UpgradePhase::ActivationVerified);
            state.first_write_at = Some(Utc::now());
        }
        let root = tempfile::tempdir().unwrap();
        let engine = UpgradeEngine::new(driver.clone(), root.path());
        assert!(engine.run(target()).await.is_err());
        let state = driver.state.lock().await;
        let saved = state.operations.values().next().unwrap();
        assert_eq!(saved.status, UpgradeStatus::RecoveryRequired);
        assert!(!state.recovered);
        assert_eq!(
            saved.failure_code.as_deref(),
            Some("driver_failed"),
            "the initiating failure remains machine-readable"
        );
        assert!(
            saved
                .failure_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("automatic downgrade is forbidden"))
        );
    }

    #[tokio::test]
    async fn resume_continues_from_persisted_verified_boundary() {
        let driver = Arc::new(FakeDriver::new());
        let target = target();
        let mut snapshot =
            UpgradeSnapshot::new(Uuid::new_v4(), driver.profile(), &driver.source, &target)
                .unwrap();
        snapshot.advance(UpgradePhase::LocksAcquired).unwrap();
        driver.create_operation(&snapshot).await.unwrap();

        let preflight = driver.preflight(&snapshot, &target).await.unwrap();
        preflight
            .validate(snapshot.source_generation_id, &target)
            .unwrap();
        snapshot.evidence.preflight = Some(preflight);
        snapshot.advance(UpgradePhase::PreflightPassed).unwrap();
        let drain = driver.drain(&snapshot).await.unwrap();
        drain.validate().unwrap();
        snapshot.evidence.drain = Some(drain);
        snapshot.advance(UpgradePhase::Drained).unwrap();
        let freeze = driver.freeze_writes(&snapshot).await.unwrap();
        snapshot.evidence.freeze = Some(freeze);
        snapshot.advance(UpgradePhase::WritesFrozen).unwrap();
        let backup = driver.create_backup(&snapshot).await.unwrap();
        backup.validate(snapshot.source_generation_id).unwrap();
        snapshot.evidence.backup = Some(backup);
        snapshot.advance(UpgradePhase::BackupCreated).unwrap();
        driver.save_operation(&snapshot).await.unwrap();

        let root = tempfile::tempdir().unwrap();
        let engine = UpgradeEngine::new(driver, root.path());
        let completed = engine.resume(snapshot.operation_id, target).await.unwrap();
        assert_eq!(completed.status, UpgradeStatus::Succeeded);
        assert_eq!(completed.phase, UpgradePhase::Completed);
        assert!(
            UpgradeJournal::new(root.path(), snapshot.operation_id)
                .unwrap()
                .read_verified()
                .unwrap()
                .len()
                >= 2
        );
    }
}
