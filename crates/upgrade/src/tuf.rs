use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use muriarc_core::ReleaseManifest;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{TrustedMetadataVersions, UpgradeError, VerifiedRelease, valid_sha256_digest};

const SUPPORTED_SPEC_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSignature {
    pub keyid: String,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMetadata<T> {
    pub signatures: Vec<MetadataSignature>,
    pub signed: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TufKeyValue {
    pub public: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TufKey {
    pub keytype: String,
    pub scheme: String,
    pub keyval: TufKeyValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TufRole {
    pub keyids: Vec<String>,
    pub threshold: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootMetadata {
    #[serde(rename = "_type")]
    pub metadata_type: String,
    pub spec_version: String,
    pub version: u64,
    pub expires: DateTime<Utc>,
    pub consistent_snapshot: bool,
    pub keys: BTreeMap<String, TufKey>,
    pub roles: BTreeMap<String, TufRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataFile {
    pub version: u64,
    pub length: u64,
    pub hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampMetadata {
    #[serde(rename = "_type")]
    pub metadata_type: String,
    pub spec_version: String,
    pub version: u64,
    pub expires: DateTime<Utc>,
    pub meta: BTreeMap<String, MetadataFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    #[serde(rename = "_type")]
    pub metadata_type: String,
    pub spec_version: String,
    pub version: u64,
    pub expires: DateTime<Utc>,
    pub meta: BTreeMap<String, MetadataFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescription {
    pub length: u64,
    pub hashes: BTreeMap<String, String>,
    pub custom: ReleaseManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetsMetadata {
    #[serde(rename = "_type")]
    pub metadata_type: String,
    pub spec_version: String,
    pub version: u64,
    pub expires: DateTime<Utc>,
    pub targets: BTreeMap<String, TargetDescription>,
}

#[derive(Debug, Clone)]
pub struct TufVerifier {
    root: SignedMetadata<RootMetadata>,
    trusted_versions: TrustedMetadataVersions,
}

impl TufVerifier {
    /// The first root is pinned out of band. It still must be internally
    /// threshold-signed so a truncated or accidentally replaced root fails.
    pub fn from_trusted_root(bytes: &[u8], now: DateTime<Utc>) -> Result<Self, UpgradeError> {
        let root: SignedMetadata<RootMetadata> = parse(bytes, "root")?;
        validate_common(
            &root.signed.metadata_type,
            "root",
            &root.signed.spec_version,
            root.signed.version,
            root.signed.expires,
            now,
        )?;
        verify_role(&root, &root.signed, "root")?;
        Ok(Self {
            trusted_versions: TrustedMetadataVersions {
                root: root.signed.version,
                timestamp: 0,
                snapshot: 0,
                targets: 0,
            },
            root,
        })
    }

    pub fn trusted_versions(&self) -> &TrustedMetadataVersions {
        &self.trusted_versions
    }

    pub fn rotate_root(&mut self, bytes: &[u8], now: DateTime<Utc>) -> Result<(), UpgradeError> {
        let next: SignedMetadata<RootMetadata> = parse(bytes, "root")?;
        validate_common(
            &next.signed.metadata_type,
            "root",
            &next.signed.spec_version,
            next.signed.version,
            next.signed.expires,
            now,
        )?;
        if next.signed.version != self.root.signed.version + 1 {
            return Err(UpgradeError::MetadataRollback {
                role: "root".to_owned(),
                observed: next.signed.version,
                trusted: self.root.signed.version + 1,
            });
        }
        verify_role(&next, &self.root.signed, "root")?;
        verify_role(&next, &next.signed, "root")?;
        self.trusted_versions.root = next.signed.version;
        self.root = next;
        Ok(())
    }

    pub fn verify_release(
        &mut self,
        timestamp_bytes: &[u8],
        snapshot_bytes: &[u8],
        targets_bytes: &[u8],
        target_name: &str,
        now: DateTime<Utc>,
    ) -> Result<VerifiedRelease, UpgradeError> {
        let timestamp: SignedMetadata<TimestampMetadata> = parse(timestamp_bytes, "timestamp")?;
        validate_common(
            &timestamp.signed.metadata_type,
            "timestamp",
            &timestamp.signed.spec_version,
            timestamp.signed.version,
            timestamp.signed.expires,
            now,
        )?;
        verify_role(&timestamp, &self.root.signed, "timestamp")?;
        reject_rollback(
            "timestamp",
            timestamp.signed.version,
            self.trusted_versions.timestamp,
        )?;

        let snapshot_meta = timestamp.signed.meta.get("snapshot.json").ok_or_else(|| {
            UpgradeError::TargetInvalid {
                message: "timestamp metadata does not pin snapshot.json".to_owned(),
            }
        })?;
        verify_metadata_file(snapshot_bytes, snapshot_meta, "snapshot.json")?;
        let snapshot: SignedMetadata<SnapshotMetadata> = parse(snapshot_bytes, "snapshot")?;
        validate_common(
            &snapshot.signed.metadata_type,
            "snapshot",
            &snapshot.signed.spec_version,
            snapshot.signed.version,
            snapshot.signed.expires,
            now,
        )?;
        verify_role(&snapshot, &self.root.signed, "snapshot")?;
        if snapshot.signed.version != snapshot_meta.version {
            return Err(UpgradeError::TargetInvalid {
                message: "snapshot version differs from timestamp metadata".to_owned(),
            });
        }
        reject_rollback(
            "snapshot",
            snapshot.signed.version,
            self.trusted_versions.snapshot,
        )?;

        let targets_meta = snapshot.signed.meta.get("targets.json").ok_or_else(|| {
            UpgradeError::TargetInvalid {
                message: "snapshot metadata does not pin targets.json".to_owned(),
            }
        })?;
        verify_metadata_file(targets_bytes, targets_meta, "targets.json")?;
        let targets: SignedMetadata<TargetsMetadata> = parse(targets_bytes, "targets")?;
        validate_common(
            &targets.signed.metadata_type,
            "targets",
            &targets.signed.spec_version,
            targets.signed.version,
            targets.signed.expires,
            now,
        )?;
        verify_role(&targets, &self.root.signed, "targets")?;
        if targets.signed.version != targets_meta.version {
            return Err(UpgradeError::TargetInvalid {
                message: "targets version differs from snapshot metadata".to_owned(),
            });
        }
        reject_rollback(
            "targets",
            targets.signed.version,
            self.trusted_versions.targets,
        )?;

        let target =
            targets
                .signed
                .targets
                .get(target_name)
                .ok_or_else(|| UpgradeError::TargetInvalid {
                    message: format!("target {target_name} is not present in signed metadata"),
                })?;
        let digest = target
            .hashes
            .get("sha256")
            .map(|value| format!("sha256:{value}"))
            .filter(|value| valid_sha256_digest(value))
            .ok_or_else(|| UpgradeError::TargetInvalid {
                message: "target lacks a valid SHA-256 digest".to_owned(),
            })?;
        target
            .custom
            .validate()
            .map_err(|message| UpgradeError::TargetInvalid { message })?;

        let versions = TrustedMetadataVersions {
            root: self.root.signed.version,
            timestamp: timestamp.signed.version,
            snapshot: snapshot.signed.version,
            targets: targets.signed.version,
        };
        self.trusted_versions = versions.clone();
        let metadata_expires_at = [
            timestamp.signed.expires,
            snapshot.signed.expires,
            targets.signed.expires,
        ]
        .into_iter()
        .min()
        .ok_or(UpgradeError::MetadataExpired)?;
        Ok(VerifiedRelease {
            manifest: target.custom.clone(),
            target_name: target_name.to_owned(),
            target_length: target.length,
            target_digest: digest,
            metadata_versions: versions,
            metadata_expires_at,
        })
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8], role: &str) -> Result<T, UpgradeError> {
    serde_json::from_slice(bytes).map_err(|error| UpgradeError::TargetInvalid {
        message: format!("{role} metadata is malformed: {error}"),
    })
}

fn validate_common(
    metadata_type: &str,
    expected_type: &str,
    spec_version: &str,
    version: u64,
    expires: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), UpgradeError> {
    if metadata_type != expected_type || spec_version != SUPPORTED_SPEC_VERSION || version == 0 {
        return Err(UpgradeError::TargetInvalid {
            message: format!("{expected_type} metadata header is unsupported"),
        });
    }
    if expires <= now {
        return Err(UpgradeError::MetadataExpired);
    }
    Ok(())
}

fn reject_rollback(role: &str, observed: u64, trusted: u64) -> Result<(), UpgradeError> {
    if observed < trusted {
        Err(UpgradeError::MetadataRollback {
            role: role.to_owned(),
            observed,
            trusted,
        })
    } else {
        Ok(())
    }
}

fn verify_role<T: Serialize>(
    envelope: &SignedMetadata<T>,
    root: &RootMetadata,
    role_name: &str,
) -> Result<(), UpgradeError> {
    let role = root
        .roles
        .get(role_name)
        .ok_or_else(|| UpgradeError::TargetInvalid {
            message: format!("trusted root does not define {role_name} role"),
        })?;
    if role.threshold == 0
        || usize::try_from(role.threshold).unwrap_or(usize::MAX) > role.keyids.len()
    {
        return Err(UpgradeError::TargetInvalid {
            message: format!("trusted {role_name} threshold is invalid"),
        });
    }
    let authorized = role.keyids.iter().cloned().collect::<BTreeSet<_>>();
    let signed = canonical_signed_bytes(&envelope.signed)?;
    let mut valid = BTreeSet::new();
    for signature in &envelope.signatures {
        if !authorized.contains(&signature.keyid) || valid.contains(&signature.keyid) {
            continue;
        }
        let Some(key) = root.keys.get(&signature.keyid) else {
            continue;
        };
        if key.keytype != "ed25519" || key.scheme != "ed25519" {
            continue;
        }
        let Ok(public) = STANDARD.decode(&key.keyval.public) else {
            continue;
        };
        let Ok(signature_bytes) = STANDARD.decode(&signature.sig) else {
            continue;
        };
        if UnparsedPublicKey::new(&ED25519, public)
            .verify(&signed, &signature_bytes)
            .is_ok()
        {
            valid.insert(signature.keyid.clone());
        }
    }
    if valid.len() < usize::try_from(role.threshold).unwrap_or(usize::MAX) {
        return Err(UpgradeError::SignatureThreshold {
            role: role_name.to_owned(),
        });
    }
    Ok(())
}

fn verify_metadata_file(
    bytes: &[u8],
    metadata: &MetadataFile,
    name: &str,
) -> Result<(), UpgradeError> {
    let length = u64::try_from(bytes.len()).map_err(|_| UpgradeError::ArtifactVerification {
        message: format!("{name} length exceeds supported range"),
    })?;
    let expected = metadata
        .hashes
        .get("sha256")
        .ok_or_else(|| UpgradeError::TargetInvalid {
            message: format!("{name} lacks a SHA-256 digest"),
        })?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if metadata.length != length || expected != &actual {
        return Err(UpgradeError::ArtifactVerification {
            message: format!("{name} length or digest differs from parent metadata"),
        });
    }
    Ok(())
}

pub fn canonical_signed_bytes(value: &impl Serialize) -> Result<Vec<u8>, UpgradeError> {
    let value = serde_json::to_value(value).map_err(|error| UpgradeError::TargetInvalid {
        message: format!("metadata cannot be canonicalized: {error}"),
    })?;
    let mut output = Vec::new();
    write_canonical_json(&value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<(), UpgradeError> {
    match value {
        Value::Object(object) => {
            output.push(b'{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                output.extend(serde_json::to_vec(key).map_err(canonical_error)?);
                output.push(b':');
                write_canonical_json(&object[key], output)?;
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            output.extend(serde_json::to_vec(value).map_err(canonical_error)?);
        }
    }
    Ok(())
}

fn canonical_error(error: serde_json::Error) -> UpgradeError {
    UpgradeError::TargetInvalid {
        message: format!("metadata cannot be canonicalized: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use muriarc_core::{
        BackendKind, BackendStateDigest, MigrationClass, ReleaseArtifact, ReleaseManifest,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    use super::*;

    fn keypair() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[7_u8; 32]).unwrap()
    }

    fn sign<T: Clone + Serialize>(
        signed: &T,
        keyid: &str,
        keypair: &Ed25519KeyPair,
    ) -> SignedMetadata<T> {
        let bytes = canonical_signed_bytes(signed).unwrap();
        SignedMetadata {
            signatures: vec![MetadataSignature {
                keyid: keyid.to_owned(),
                sig: STANDARD.encode(keypair.sign(&bytes).as_ref()),
            }],
            signed: signed.clone(),
        }
    }

    fn root_bytes(expires: DateTime<Utc>) -> Vec<u8> {
        let keypair = keypair();
        let keyid = "root-key";
        let root = RootMetadata {
            metadata_type: "root".to_owned(),
            spec_version: SUPPORTED_SPEC_VERSION.to_owned(),
            version: 1,
            expires,
            consistent_snapshot: true,
            keys: BTreeMap::from([(
                keyid.to_owned(),
                TufKey {
                    keytype: "ed25519".to_owned(),
                    scheme: "ed25519".to_owned(),
                    keyval: TufKeyValue {
                        public: STANDARD.encode(keypair.public_key().as_ref()),
                    },
                },
            )]),
            roles: ["root", "timestamp", "snapshot", "targets"]
                .into_iter()
                .map(|role| {
                    (
                        role.to_owned(),
                        TufRole {
                            keyids: vec![keyid.to_owned()],
                            threshold: 1,
                        },
                    )
                })
                .collect(),
        };
        serde_json::to_vec(&sign(&root, keyid, &keypair)).unwrap()
    }

    fn manifest() -> ReleaseManifest {
        let digest: BackendStateDigest = format!("sha256:{}", "a".repeat(64)).parse().unwrap();
        ReleaseManifest {
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
                "native".to_owned(),
                ReleaseArtifact {
                    media_type: "application/octet-stream".to_owned(),
                    digest,
                    size_bytes: 4,
                },
            )]),
        }
    }

    fn metadata_file(bytes: &[u8], version: u64) -> MetadataFile {
        MetadataFile {
            version,
            length: u64::try_from(bytes.len()).unwrap(),
            hashes: BTreeMap::from([("sha256".to_owned(), format!("{:x}", Sha256::digest(bytes)))]),
        }
    }

    fn chain(version: u64, expires: DateTime<Utc>) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let keypair = keypair();
        let keyid = "root-key";
        let targets = TargetsMetadata {
            metadata_type: "targets".to_owned(),
            spec_version: SUPPORTED_SPEC_VERSION.to_owned(),
            version,
            expires,
            targets: BTreeMap::from([(
                "muriarc-controller".to_owned(),
                TargetDescription {
                    length: 4,
                    hashes: BTreeMap::from([(
                        "sha256".to_owned(),
                        format!("{:x}", Sha256::digest(b"test")),
                    )]),
                    custom: manifest(),
                },
            )]),
        };
        let targets_bytes = serde_json::to_vec(&sign(&targets, keyid, &keypair)).unwrap();
        let snapshot = SnapshotMetadata {
            metadata_type: "snapshot".to_owned(),
            spec_version: SUPPORTED_SPEC_VERSION.to_owned(),
            version,
            expires,
            meta: BTreeMap::from([(
                "targets.json".to_owned(),
                metadata_file(&targets_bytes, version),
            )]),
        };
        let snapshot_bytes = serde_json::to_vec(&sign(&snapshot, keyid, &keypair)).unwrap();
        let timestamp = TimestampMetadata {
            metadata_type: "timestamp".to_owned(),
            spec_version: SUPPORTED_SPEC_VERSION.to_owned(),
            version,
            expires,
            meta: BTreeMap::from([(
                "snapshot.json".to_owned(),
                metadata_file(&snapshot_bytes, version),
            )]),
        };
        let timestamp_bytes = serde_json::to_vec(&sign(&timestamp, keyid, &keypair)).unwrap();
        (timestamp_bytes, snapshot_bytes, targets_bytes)
    }

    #[test]
    fn signed_chain_yields_non_constructible_verified_release() {
        let now = Utc::now();
        let expires = now + chrono::Duration::hours(1);
        let mut verifier = TufVerifier::from_trusted_root(&root_bytes(expires), now).unwrap();
        let (timestamp, snapshot, targets) = chain(1, expires);
        let release = verifier
            .verify_release(&timestamp, &snapshot, &targets, "muriarc-controller", now)
            .unwrap();
        assert_eq!(release.manifest.application_version.as_str(), "1.0.0");
        assert_eq!(release.target_length, 4);
        assert_eq!(verifier.trusted_versions().targets, 1);
    }

    #[test]
    fn metadata_rollback_and_bad_signature_are_rejected() {
        let now = Utc::now();
        let expires = now + chrono::Duration::hours(1);
        let mut verifier = TufVerifier::from_trusted_root(&root_bytes(expires), now).unwrap();
        let (timestamp_v2, snapshot_v2, targets_v2) = chain(2, expires);
        verifier
            .verify_release(
                &timestamp_v2,
                &snapshot_v2,
                &targets_v2,
                "muriarc-controller",
                now,
            )
            .unwrap();
        let (timestamp_v1, snapshot_v1, targets_v1) = chain(1, expires);
        assert!(matches!(
            verifier.verify_release(
                &timestamp_v1,
                &snapshot_v1,
                &targets_v1,
                "muriarc-controller",
                now,
            ),
            Err(UpgradeError::MetadataRollback { role, .. }) if role == "timestamp"
        ));

        let mut unsigned: SignedMetadata<TimestampMetadata> =
            serde_json::from_slice(&timestamp_v2).unwrap();
        unsigned.signatures[0].sig = STANDARD.encode([0_u8; 64]);
        assert!(matches!(
            verifier.verify_release(
                &serde_json::to_vec(&unsigned).unwrap(),
                &snapshot_v2,
                &targets_v2,
                "muriarc-controller",
                now,
            ),
            Err(UpgradeError::SignatureThreshold { role }) if role == "timestamp"
        ));
    }
}
