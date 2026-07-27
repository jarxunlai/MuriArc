use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use muriarc_core::DeploymentGenerationManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    EvidenceError, ExpectedFacts, FixtureComponentKind, FixtureFile, FixtureManifest, Sha256Digest,
    expected_facts_digest, fixture_manifest_digest, validate_fixture_path,
};

pub const FIXTURE_MANIFEST_FILE: &str = "fixture-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetVerificationResult {
    pub fixture_id: uuid::Uuid,
    pub fixture_manifest_digest: Sha256Digest,
    pub expected_facts_digest: Sha256Digest,
    pub fixture_content_digest: Sha256Digest,
    pub verified_file_count: u64,
    pub verified_bytes: u64,
}

pub fn load_and_verify_fixture(
    fixture_root: &Path,
    expected_manifest_digest: Option<&Sha256Digest>,
) -> Result<(FixtureManifest, ExpectedFacts, AssetVerificationResult), EvidenceError> {
    let root_metadata = fs::symlink_metadata(fixture_root).map_err(io)?;
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err(EvidenceError::AssetVerification {
            message: "fixture root must be a real directory".to_owned(),
        });
    }
    let manifest_path = fixture_root.join(FIXTURE_MANIFEST_FILE);
    let manifest_metadata = fs::symlink_metadata(&manifest_path).map_err(io)?;
    if !manifest_metadata.file_type().is_file() || manifest_metadata.file_type().is_symlink() {
        return Err(EvidenceError::AssetVerification {
            message: "fixture manifest must be a regular non-symlink file".to_owned(),
        });
    }
    let manifest_bytes = fs::read(&manifest_path).map_err(io)?;
    let manifest: FixtureManifest =
        serde_json::from_slice(&manifest_bytes).map_err(serialization)?;
    manifest.validate()?;
    let manifest_digest = fixture_manifest_digest(&manifest)?;
    if expected_manifest_digest.is_some_and(|expected| expected != &manifest_digest) {
        return Err(EvidenceError::AssetVerification {
            message: "fixture manifest differs from Catalog digest".to_owned(),
        });
    }

    let observed = inventory_regular_files(fixture_root)?;
    let expected = manifest
        .files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let observed_paths = observed.keys().cloned().collect::<BTreeSet<_>>();
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    if observed_paths != expected_paths {
        return Err(EvidenceError::AssetVerification {
            message: "fixture contains missing or unregistered files".to_owned(),
        });
    }

    let mut verified_bytes = 0_u64;
    for (path, absolute) in &observed {
        let expected_file = expected[path];
        let (size, digest) = hash_file(absolute)?;
        if size != expected_file.size_bytes || digest != expected_file.sha256 {
            return Err(EvidenceError::AssetVerification {
                message: format!("fixture file {path} length or digest differs"),
            });
        }
        verified_bytes =
            verified_bytes
                .checked_add(size)
                .ok_or_else(|| EvidenceError::AssetVerification {
                    message: "fixture byte count overflowed".to_owned(),
                })?;
    }

    let facts_file = manifest
        .files
        .iter()
        .find(|file| file.kind == FixtureComponentKind::ExpectedFacts)
        .ok_or(EvidenceError::IncompleteRecoverySet)?;
    let facts_bytes = fs::read(fixture_root.join(&facts_file.path)).map_err(io)?;
    let facts: ExpectedFacts = serde_json::from_slice(&facts_bytes).map_err(serialization)?;
    let canonical_facts = serde_json::to_vec(&facts).map_err(serialization)?;
    if canonical_facts != facts_bytes {
        return Err(EvidenceError::ExpectedFactsMismatch);
    }
    facts.validate(&manifest)?;
    let facts_digest = expected_facts_digest(&facts)?;
    if facts_digest != manifest.expected_facts_digest {
        return Err(EvidenceError::ExpectedFactsMismatch);
    }
    verify_generation_manifest(fixture_root, &manifest)?;

    let content_digest = fixture_content_digest(&manifest.files)?;
    let result = AssetVerificationResult {
        fixture_id: manifest.fixture_id,
        fixture_manifest_digest: manifest_digest,
        expected_facts_digest: facts_digest,
        fixture_content_digest: content_digest,
        verified_file_count: u64::try_from(observed.len()).map_err(|_| {
            EvidenceError::AssetVerification {
                message: "fixture file count exceeds supported range".to_owned(),
            }
        })?,
        verified_bytes,
    };
    Ok((manifest, facts, result))
}

pub fn fixture_content_digest(files: &[FixtureFile]) -> Result<Sha256Digest, EvidenceError> {
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    hasher.update(b"MuriArc/fixture-content/v1\0");
    for file in files {
        validate_fixture_path(&file.path)?;
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.size_bytes.to_be_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_str().as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("sha256:{:x}", hasher.finalize())
        .parse()
        .expect("SHA-256 formatting is valid"))
}

fn inventory_regular_files(root: &Path) -> Result<BTreeMap<String, PathBuf>, EvidenceError> {
    let mut output = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(io)? {
            let entry = entry.map_err(io)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(io)?;
            if metadata.file_type().is_symlink() {
                return Err(EvidenceError::AssetVerification {
                    message: "fixture may not contain symlinks".to_owned(),
                });
            }
            if metadata.file_type().is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.file_type().is_file() {
                return Err(EvidenceError::AssetVerification {
                    message: "fixture may contain only regular files and directories".to_owned(),
                });
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| EvidenceError::AssetVerification {
                    message: "fixture path escaped its root".to_owned(),
                })?
                .to_str()
                .ok_or_else(|| EvidenceError::AssetVerification {
                    message: "fixture paths must be UTF-8".to_owned(),
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if relative == FIXTURE_MANIFEST_FILE {
                continue;
            }
            validate_fixture_path(&relative)?;
            if output.insert(relative, path).is_some() {
                return Err(EvidenceError::AssetVerification {
                    message: "fixture contains duplicate normalized paths".to_owned(),
                });
            }
        }
    }
    Ok(output)
}

fn verify_generation_manifest(
    fixture_root: &Path,
    fixture: &FixtureManifest,
) -> Result<(), EvidenceError> {
    let file = fixture
        .files
        .iter()
        .find(|file| file.kind == FixtureComponentKind::GenerationManifest)
        .ok_or(EvidenceError::IncompleteRecoverySet)?;
    let bytes = fs::read(fixture_root.join(&file.path)).map_err(io)?;
    let generation: DeploymentGenerationManifest =
        serde_json::from_slice(&bytes).map_err(serialization)?;
    if generation.generation_id != fixture.generation_id
        || generation.data_epoch != fixture.release_identity.data_epoch
        || generation.backend_state_digest != fixture.release_identity.backend_state_digest
    {
        return Err(EvidenceError::AssetVerification {
            message: "deployment generation manifest differs from fixture identity".to_owned(),
        });
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(u64, Sha256Digest), EvidenceError> {
    let mut file = File::open(path).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 1024 * 64];
    loop {
        let read = file.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(
                u64::try_from(read).map_err(|_| EvidenceError::AssetVerification {
                    message: "fixture file length exceeds supported range".to_owned(),
                })?,
            )
            .ok_or_else(|| EvidenceError::AssetVerification {
                message: "fixture file length overflowed".to_owned(),
            })?;
        hasher.update(&buffer[..read]);
    }
    let digest = format!("sha256:{:x}", hasher.finalize())
        .parse()
        .expect("SHA-256 formatting is valid");
    Ok((size, digest))
}

fn io(error: std::io::Error) -> EvidenceError {
    EvidenceError::Io {
        message: error.to_string(),
    }
}

fn serialization(error: serde_json::Error) -> EvidenceError {
    EvidenceError::Serialization {
        message: error.to_string(),
    }
}
