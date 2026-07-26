use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_core::ReleaseIdentity;
use muriarc_upgrade::{
    VerificationEvidence as UpgradeVerificationEvidence,
    VerificationLayer as UpgradeVerificationLayer, VerificationLayerEvidence,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AssetVerificationResult, EvidenceError, ExpectedFacts, FixtureCatalog, FixtureManifest,
    Sha256Digest, VERIFICATION_REPORT_FORMAT_VERSION, digest_bytes, load_and_verify_fixture,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    Pr,
    Nightly,
    Rc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryProfile {
    NativeSystem,
    ManagedCompose,
    DesktopWindows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactExecutionKind {
    FinalPackage,
    SourceRun,
    DemoGateway,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterLayerEvidence {
    pub status: LayerStatus,
    pub evidence_digest: Option<Sha256Digest>,
    pub observed_expected_facts_digest: Option<Sha256Digest>,
    pub state_digest_before: Option<Sha256Digest>,
    pub state_digest_after: Option<Sha256Digest>,
    pub continuation_write_verified: bool,
    pub detail_codes: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationLayerRecord {
    pub layer: VerificationLayer,
    pub status: LayerStatus,
    pub evidence_digest: Option<Sha256Digest>,
    pub detail_codes: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    pub format_version: u32,
    pub run_id: Uuid,
    pub fixture_id: Uuid,
    pub source_identity: ReleaseIdentity,
    pub target_identity: ReleaseIdentity,
    pub mode: VerificationMode,
    pub profile: DeliveryProfile,
    pub target_artifact_digest: Sha256Digest,
    pub execution_kind: ArtifactExecutionKind,
    pub expected_facts_digest: Sha256Digest,
    pub layers: BTreeMap<VerificationLayer, VerificationLayerRecord>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

impl VerificationReport {
    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.format_version != VERIFICATION_REPORT_FORMAT_VERSION
            || self.run_id.is_nil()
            || self.fixture_id.is_nil()
            || self.layers.keys().copied().collect::<BTreeSet<_>>() != VerificationLayer::required()
        {
            return Err(EvidenceError::InvalidReport {
                message: "report header or seven-layer set is invalid".to_owned(),
            });
        }
        for (layer, record) in &self.layers {
            if record.layer != *layer
                || record.status != LayerStatus::Pass
                || record.evidence_digest.is_none()
                || record.completed_at < record.started_at
                || record
                    .detail_codes
                    .iter()
                    .any(|code| !valid_detail_code(code))
            {
                return Err(EvidenceError::LayerFailed {
                    layer: format!("{layer:?}"),
                    message: "FAIL, SKIP, missing digest, or invalid timestamps".to_owned(),
                });
            }
        }
        if self.completed_at < self.started_at {
            return Err(EvidenceError::InvalidReport {
                message: "report completion precedes start".to_owned(),
            });
        }
        if self.mode == VerificationMode::Rc
            && self.execution_kind != ArtifactExecutionKind::FinalPackage
        {
            return Err(EvidenceError::InvalidReport {
                message:
                    "RC verification must execute final packages/images, not source or DemoGateway"
                        .to_owned(),
            });
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<Sha256Digest, EvidenceError> {
        let bytes = serde_json::to_vec(self).map_err(|error| EvidenceError::Serialization {
            message: error.to_string(),
        })?;
        Ok(digest_bytes(&bytes))
    }

    pub fn to_upgrade_evidence(
        &self,
        candidate_generation_id: Uuid,
    ) -> Result<UpgradeVerificationEvidence, EvidenceError> {
        self.validate()?;
        if candidate_generation_id.is_nil() {
            return Err(EvidenceError::InvalidReport {
                message: "Candidate generation ID must be non-nil".to_owned(),
            });
        }
        let layers = self
            .layers
            .iter()
            .map(|(layer, record)| {
                let upgrade_layer = match layer {
                    VerificationLayer::AssetRestore => UpgradeVerificationLayer::AssetRestore,
                    VerificationLayer::Storage => UpgradeVerificationLayer::Storage,
                    VerificationLayer::StoreApplication => {
                        UpgradeVerificationLayer::StoreApplication
                    }
                    VerificationLayer::Api => UpgradeVerificationLayer::Api,
                    VerificationLayer::RemoteUi => UpgradeVerificationLayer::RemoteUi,
                    VerificationLayer::ContinueWrite => UpgradeVerificationLayer::ContinueWrite,
                    VerificationLayer::ReadOnlyNoSideEffects => {
                        UpgradeVerificationLayer::ReadOnlyNoSideEffects
                    }
                };
                let digest = record
                    .evidence_digest
                    .as_ref()
                    .expect("validated report has an evidence digest");
                (
                    upgrade_layer,
                    VerificationLayerEvidence {
                        evidence_digest: digest.to_string(),
                        verified_at: record.completed_at,
                    },
                )
            })
            .collect();
        Ok(UpgradeVerificationEvidence {
            generation_id: candidate_generation_id,
            layers,
        })
    }
}

pub struct VerificationContext<'a> {
    pub fixture_root: &'a Path,
    pub manifest: &'a FixtureManifest,
    pub expected_facts: &'a ExpectedFacts,
    pub target_identity: &'a ReleaseIdentity,
    pub mode: VerificationMode,
    pub profile: DeliveryProfile,
}

#[async_trait]
pub trait VerificationAdapter: Send + Sync {
    async fn verify_layer(
        &self,
        layer: VerificationLayer,
        context: &VerificationContext<'_>,
    ) -> Result<AdapterLayerEvidence, EvidenceError>;
}

pub struct VerifierRunner<A> {
    fixture_root: PathBuf,
    expected_manifest_digest: Option<Sha256Digest>,
    target_identity: ReleaseIdentity,
    target_artifact_digest: Sha256Digest,
    mode: VerificationMode,
    profile: DeliveryProfile,
    execution_kind: ArtifactExecutionKind,
    adapter: A,
}

impl<A: VerificationAdapter> VerifierRunner<A> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fixture_root: PathBuf,
        expected_manifest_digest: Option<Sha256Digest>,
        target_identity: ReleaseIdentity,
        target_artifact_digest: Sha256Digest,
        mode: VerificationMode,
        profile: DeliveryProfile,
        execution_kind: ArtifactExecutionKind,
        adapter: A,
    ) -> Self {
        Self {
            fixture_root,
            expected_manifest_digest,
            target_identity,
            target_artifact_digest,
            mode,
            profile,
            execution_kind,
            adapter,
        }
    }

    pub async fn run(self) -> Result<VerificationReport, EvidenceError> {
        let started_at = Utc::now();
        let (manifest, expected_facts, asset_result) =
            load_and_verify_fixture(&self.fixture_root, self.expected_manifest_digest.as_ref())?;
        let mut layers = BTreeMap::new();
        layers.insert(
            VerificationLayer::AssetRestore,
            asset_layer_record(&asset_result, started_at)?,
        );
        let context = VerificationContext {
            fixture_root: &self.fixture_root,
            manifest: &manifest,
            expected_facts: &expected_facts,
            target_identity: &self.target_identity,
            mode: self.mode,
            profile: self.profile,
        };
        for layer in [
            VerificationLayer::Storage,
            VerificationLayer::StoreApplication,
            VerificationLayer::Api,
            VerificationLayer::RemoteUi,
            VerificationLayer::ContinueWrite,
            VerificationLayer::ReadOnlyNoSideEffects,
        ] {
            let evidence = self.adapter.verify_layer(layer, &context).await?;
            validate_adapter_evidence(layer, &evidence, &manifest.expected_facts_digest)?;
            layers.insert(
                layer,
                VerificationLayerRecord {
                    layer,
                    status: evidence.status,
                    evidence_digest: evidence.evidence_digest,
                    detail_codes: evidence.detail_codes,
                    started_at: evidence.started_at,
                    completed_at: evidence.completed_at,
                },
            );
        }
        let report = VerificationReport {
            format_version: VERIFICATION_REPORT_FORMAT_VERSION,
            run_id: Uuid::new_v4(),
            fixture_id: manifest.fixture_id,
            source_identity: manifest.release_identity,
            target_identity: self.target_identity,
            mode: self.mode,
            profile: self.profile,
            target_artifact_digest: self.target_artifact_digest,
            execution_kind: self.execution_kind,
            expected_facts_digest: manifest.expected_facts_digest,
            layers,
            started_at,
            completed_at: Utc::now(),
        };
        report.validate()?;
        Ok(report)
    }
}

fn asset_layer_record(
    result: &AssetVerificationResult,
    started_at: DateTime<Utc>,
) -> Result<VerificationLayerRecord, EvidenceError> {
    let bytes = serde_json::to_vec(result).map_err(|error| EvidenceError::Serialization {
        message: error.to_string(),
    })?;
    Ok(VerificationLayerRecord {
        layer: VerificationLayer::AssetRestore,
        status: LayerStatus::Pass,
        evidence_digest: Some(digest_bytes(&bytes)),
        detail_codes: Vec::new(),
        started_at,
        completed_at: Utc::now(),
    })
}

fn validate_adapter_evidence(
    layer: VerificationLayer,
    evidence: &AdapterLayerEvidence,
    expected_facts_digest: &Sha256Digest,
) -> Result<(), EvidenceError> {
    if evidence.status != LayerStatus::Pass
        || evidence.evidence_digest.is_none()
        || evidence.observed_expected_facts_digest.as_ref() != Some(expected_facts_digest)
        || evidence.completed_at < evidence.started_at
        || evidence
            .detail_codes
            .iter()
            .any(|code| !valid_detail_code(code))
    {
        return Err(EvidenceError::LayerFailed {
            layer: format!("{layer:?}"),
            message: "layer did not pass against the exact Expected Facts".to_owned(),
        });
    }
    if layer == VerificationLayer::ContinueWrite && !evidence.continuation_write_verified {
        return Err(EvidenceError::LayerFailed {
            layer: format!("{layer:?}"),
            message: "continued write contract was not verified".to_owned(),
        });
    }
    if layer == VerificationLayer::ReadOnlyNoSideEffects
        && (evidence.state_digest_before.is_none()
            || evidence.state_digest_before != evidence.state_digest_after)
    {
        return Err(EvidenceError::LayerFailed {
            layer: format!("{layer:?}"),
            message: "read-only verification changed persistent state".to_owned(),
        });
    }
    Ok(())
}

fn valid_detail_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityMatrixDefinition {
    pub format_version: u32,
    pub pr_profiles: BTreeSet<DeliveryProfile>,
    pub nightly_profiles: BTreeSet<DeliveryProfile>,
    pub rc_profiles: BTreeSet<DeliveryProfile>,
    pub rc_requires_all_catalog_entries: bool,
    pub rc_requires_final_artifacts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixRun {
    pub fixture_id: Uuid,
    pub profile: DeliveryProfile,
    pub report_digest: Option<Sha256Digest>,
    pub status: LayerStatus,
    pub execution_kind: ArtifactExecutionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityMatrixReport {
    pub format_version: u32,
    pub mode: VerificationMode,
    pub selected_fixture_ids: BTreeSet<Uuid>,
    pub runs: Vec<MatrixRun>,
}

impl CompatibilityMatrixReport {
    pub fn validate(
        &self,
        definition: &CompatibilityMatrixDefinition,
        catalog: &FixtureCatalog,
    ) -> Result<(), EvidenceError> {
        if self.format_version != 1 || definition.format_version != 1 {
            return Err(EvidenceError::InvalidReport {
                message: "matrix format is unsupported".to_owned(),
            });
        }
        let all_profiles = BTreeSet::from([
            DeliveryProfile::NativeSystem,
            DeliveryProfile::ManagedCompose,
            DeliveryProfile::DesktopWindows,
        ]);
        if definition.pr_profiles.is_empty()
            || definition.nightly_profiles.is_empty()
            || definition.rc_profiles != all_profiles
            || !definition.rc_requires_all_catalog_entries
            || !definition.rc_requires_final_artifacts
        {
            return Err(EvidenceError::InvalidReport {
                message: "matrix definition weakens the mandatory RC compatibility policy"
                    .to_owned(),
            });
        }
        catalog.validate()?;
        let catalog_ids = catalog
            .entries
            .iter()
            .map(|entry| entry.fixture_id)
            .collect::<BTreeSet<_>>();
        if !self.selected_fixture_ids.is_subset(&catalog_ids) {
            return Err(EvidenceError::InvalidReport {
                message: "matrix selects a fixture outside the immutable Catalog".to_owned(),
            });
        }
        let required_profiles = match self.mode {
            VerificationMode::Pr => &definition.pr_profiles,
            VerificationMode::Nightly => &definition.nightly_profiles,
            VerificationMode::Rc => &definition.rc_profiles,
        };
        if self.mode == VerificationMode::Nightly && self.selected_fixture_ids != catalog_ids {
            return Err(EvidenceError::InvalidReport {
                message: "Nightly must select the complete historical Catalog".to_owned(),
            });
        }
        if self.mode == VerificationMode::Rc {
            catalog.require_non_empty_for_rc()?;
            if self.selected_fixture_ids != catalog_ids {
                return Err(EvidenceError::InvalidReport {
                    message: "RC must select the complete historical Catalog".to_owned(),
                });
            }
        }
        let expected_pairs = self
            .selected_fixture_ids
            .iter()
            .flat_map(|fixture| {
                required_profiles
                    .iter()
                    .map(move |profile| (*fixture, *profile))
            })
            .collect::<BTreeSet<_>>();
        let observed_pairs = self
            .runs
            .iter()
            .map(|run| (run.fixture_id, run.profile))
            .collect::<BTreeSet<_>>();
        if expected_pairs != observed_pairs || self.runs.len() != expected_pairs.len() {
            return Err(EvidenceError::InvalidReport {
                message: "matrix is missing or duplicates a fixture/profile run".to_owned(),
            });
        }
        if self.runs.iter().any(|run| {
            run.status != LayerStatus::Pass
                || run.report_digest.is_none()
                || (self.mode == VerificationMode::Rc
                    && run.execution_kind != ArtifactExecutionKind::FinalPackage)
        }) {
            return Err(EvidenceError::InvalidReport {
                message: "matrix contains FAIL/SKIP, missing evidence, or non-final RC execution"
                    .to_owned(),
            });
        }
        Ok(())
    }
}
