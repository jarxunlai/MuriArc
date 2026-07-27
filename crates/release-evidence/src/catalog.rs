use std::collections::BTreeSet;

use muriarc_core::{
    BackendKind, BackendStateDigest, CURRENT_APPLICATION_VERSION, CURRENT_DATA_EPOCH,
    CURRENT_GATEWAY_CONTRACT_REVISION, ReleaseIdentity,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{EvidenceError, FixtureCatalog, FixtureCatalogEntry, FixtureManifest, Sha256Digest};

impl FixtureCatalogEntry {
    pub fn verify_self_digest(&self) -> Result<(), EvidenceError> {
        let expected = catalog_entry_digest(self)?;
        if expected != self.immutable_entry_digest {
            return Err(EvidenceError::InvalidCatalogEntry {
                message: format!("fixture {} self-digest differs", self.fixture_id),
            });
        }
        validate_oci_reference(self)?;
        Ok(())
    }
}

impl FixtureCatalog {
    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.format_version != 1 {
            return Err(EvidenceError::InvalidCatalogEntry {
                message: "catalog format must be 1".to_owned(),
            });
        }
        let mut fixture_ids = BTreeSet::new();
        let mut backend_states = BTreeSet::new();
        for entry in &self.entries {
            entry.verify_self_digest()?;
            if entry.fixture_id.is_nil() || !fixture_ids.insert(entry.fixture_id) {
                return Err(EvidenceError::InvalidCatalogEntry {
                    message: "fixture identifiers must be unique and non-nil".to_owned(),
                });
            }
            if !backend_states.insert((entry.backend, entry.backend_state_digest.clone())) {
                return Err(EvidenceError::InvalidCatalogEntry {
                    message: "a backend state may have only one immutable fixture".to_owned(),
                });
            }
        }
        Ok(())
    }

    pub fn append(
        &mut self,
        manifest: &FixtureManifest,
        fixture_artifact_digest: Sha256Digest,
        fixture_manifest_digest: Sha256Digest,
        oci_reference: String,
    ) -> Result<&FixtureCatalogEntry, EvidenceError> {
        manifest.validate()?;
        if self.entries.iter().any(|entry| {
            entry.fixture_id == manifest.fixture_id
                || (entry.backend == manifest.backend
                    && entry.backend_state_digest == manifest.release_identity.backend_state_digest)
        }) {
            return Err(EvidenceError::InvalidCatalogEntry {
                message: "fixture or backend state is already cataloged".to_owned(),
            });
        }
        let mut entry = FixtureCatalogEntry {
            fixture_id: manifest.fixture_id,
            application_version: manifest.release_identity.application_version.clone(),
            data_epoch: manifest.release_identity.data_epoch.clone(),
            gateway_contract_revision: manifest.release_identity.gateway_contract_revision.clone(),
            backend: manifest.backend,
            backend_state_digest: manifest.release_identity.backend_state_digest.clone(),
            source_release_artifact_digest: manifest
                .producer
                .source_release_artifact_digest
                .clone(),
            source_release_provenance_digest: manifest
                .producer
                .source_release_provenance_digest
                .clone(),
            fixture_artifact_digest,
            fixture_manifest_digest,
            expected_facts_digest: manifest.expected_facts_digest.clone(),
            oci_reference,
            created_at: manifest.producer.generated_at,
            immutable_entry_digest: zero_digest(),
        };
        entry.immutable_entry_digest = catalog_entry_digest(&entry)?;
        entry.verify_self_digest()?;
        self.entries.push(entry);
        self.validate()?;
        self.entries
            .last()
            .ok_or_else(|| EvidenceError::InvalidCatalogEntry {
                message: "catalog append produced no entry".to_owned(),
            })
    }

    pub fn assert_append_only_from(&self, previous: &Self) -> Result<(), EvidenceError> {
        previous.validate()?;
        self.validate()?;
        if self.entries.len() < previous.entries.len()
            || self.entries[..previous.entries.len()] != previous.entries
        {
            return Err(EvidenceError::CatalogNotAppendOnly);
        }
        Ok(())
    }

    pub fn require_non_empty_for_rc(&self) -> Result<(), EvidenceError> {
        self.validate()?;
        if self.entries.is_empty() {
            Err(EvidenceError::InvalidReport {
                message: "RC compatibility matrix cannot use an empty fixture catalog".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct CurrentReleaseFixtureProducer {
    backend: BackendKind,
    identity: ReleaseIdentity,
}

impl CurrentReleaseFixtureProducer {
    pub fn new(
        backend: BackendKind,
        compiled_backend_state_digest: BackendStateDigest,
    ) -> Result<Self, EvidenceError> {
        let identity = ReleaseIdentity::parse(
            CURRENT_APPLICATION_VERSION.to_owned(),
            CURRENT_DATA_EPOCH.to_owned(),
            compiled_backend_state_digest.to_string(),
            CURRENT_GATEWAY_CONTRACT_REVISION.to_owned(),
        )
        .map_err(|message| EvidenceError::InvalidFixture { message })?;
        Ok(Self { backend, identity })
    }

    pub fn identity(&self) -> &ReleaseIdentity {
        &self.identity
    }

    pub fn validate_manifest(&self, manifest: &FixtureManifest) -> Result<(), EvidenceError> {
        manifest.validate()?;
        if manifest.backend != self.backend || manifest.release_identity != self.identity {
            return Err(EvidenceError::WrongProducerRelease);
        }
        Ok(())
    }
}

pub fn fixture_manifest_digest(manifest: &FixtureManifest) -> Result<Sha256Digest, EvidenceError> {
    digest_serializable(manifest)
}

pub fn expected_facts_digest(facts: &crate::ExpectedFacts) -> Result<Sha256Digest, EvidenceError> {
    digest_serializable(facts)
}

pub fn digest_bytes(bytes: &[u8]) -> Sha256Digest {
    format!("sha256:{:x}", Sha256::digest(bytes))
        .parse()
        .expect("SHA-256 formatting is valid")
}

fn digest_serializable(value: &impl Serialize) -> Result<Sha256Digest, EvidenceError> {
    let bytes = serde_json::to_vec(value).map_err(|error| EvidenceError::Serialization {
        message: error.to_string(),
    })?;
    Ok(digest_bytes(&bytes))
}

fn catalog_entry_digest(entry: &FixtureCatalogEntry) -> Result<Sha256Digest, EvidenceError> {
    #[derive(Serialize)]
    struct DigestView<'a> {
        fixture_id: uuid::Uuid,
        application_version: &'a muriarc_core::ApplicationVersion,
        data_epoch: &'a muriarc_core::DataEpoch,
        gateway_contract_revision: &'a muriarc_core::GatewayContractRevision,
        backend: BackendKind,
        backend_state_digest: &'a BackendStateDigest,
        source_release_artifact_digest: &'a Sha256Digest,
        source_release_provenance_digest: &'a Sha256Digest,
        fixture_artifact_digest: &'a Sha256Digest,
        fixture_manifest_digest: &'a Sha256Digest,
        expected_facts_digest: &'a Sha256Digest,
        oci_reference: &'a str,
        created_at: chrono::DateTime<chrono::Utc>,
    }
    digest_serializable(&DigestView {
        fixture_id: entry.fixture_id,
        application_version: &entry.application_version,
        data_epoch: &entry.data_epoch,
        gateway_contract_revision: &entry.gateway_contract_revision,
        backend: entry.backend,
        backend_state_digest: &entry.backend_state_digest,
        source_release_artifact_digest: &entry.source_release_artifact_digest,
        source_release_provenance_digest: &entry.source_release_provenance_digest,
        fixture_artifact_digest: &entry.fixture_artifact_digest,
        fixture_manifest_digest: &entry.fixture_manifest_digest,
        expected_facts_digest: &entry.expected_facts_digest,
        oci_reference: &entry.oci_reference,
        created_at: entry.created_at,
    })
}

fn validate_oci_reference(entry: &FixtureCatalogEntry) -> Result<(), EvidenceError> {
    let expected_suffix = entry
        .fixture_artifact_digest
        .as_str()
        .strip_prefix("sha256:")
        .expect("validated digest has SHA-256 prefix");
    if !entry.oci_reference.starts_with("ghcr.io/")
        || !entry.oci_reference.contains("@sha256:")
        || !entry.oci_reference.ends_with(expected_suffix)
        || entry.oci_reference.contains(":latest")
    {
        return Err(EvidenceError::InvalidCatalogEntry {
            message: "fixture OCI reference must be GHCR and pinned by its artifact digest"
                .to_owned(),
        });
    }
    Ok(())
}

fn zero_digest() -> Sha256Digest {
    format!("sha256:{}", "0".repeat(64))
        .parse()
        .expect("zero SHA-256 digest is valid")
}
