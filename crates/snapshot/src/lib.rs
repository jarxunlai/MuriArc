#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    io::{Read, Seek, Write},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_PATH: &str = "manifest.json";
pub const MAX_SNAPSHOT_ENTRIES: usize = 50_000;
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SNAPSHOT_UNCOMPRESSED_BYTES: u64 = 100 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub schema_version: u32,
    pub snapshot_id: Uuid,
    pub origin_instance_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
    pub lab_id: Uuid,
    pub project_ids: Vec<Uuid>,
    pub entries: Vec<SnapshotEntry>,
}

impl SnapshotManifest {
    pub fn new(
        origin_instance_id: Uuid,
        lab_id: Uuid,
        created_by: Option<Uuid>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            snapshot_id: Uuid::new_v4(),
            origin_instance_id,
            created_at: now,
            created_by,
            lab_id,
            project_ids: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(SnapshotError::UnsupportedVersion(self.schema_version));
        }
        if self.entries.len() > MAX_SNAPSHOT_ENTRIES {
            return Err(SnapshotError::TooManyEntries(self.entries.len()));
        }
        let mut paths = BTreeSet::new();
        let mut total_size = 0_u64;
        for entry in &self.entries {
            validate_path(&entry.path)?;
            if entry.path == MANIFEST_PATH || !paths.insert(entry.path.as_str()) {
                return Err(SnapshotError::DuplicatePath(entry.path.clone()));
            }
            validate_sha256(&entry.sha256)?;
            total_size = total_size
                .checked_add(entry.size_bytes)
                .ok_or(SnapshotError::SnapshotTooLarge)?;
            if total_size > MAX_SNAPSHOT_UNCOMPRESSED_BYTES {
                return Err(SnapshotError::SnapshotTooLarge);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    JsonLines,
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub path: String,
    pub kind: EntryKind,
    pub entity_type: Option<String>,
    pub record_count: Option<u64>,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleEntry {
    pub path: String,
    pub kind: EntryKind,
    pub entity_type: Option<String>,
    pub record_count: Option<u64>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictDecision {
    Insert,
    SkipIdentical,
    Conflict,
}

pub fn decide_conflict(existing_sha256: Option<&str>, incoming_sha256: &str) -> ConflictDecision {
    match existing_sha256 {
        None => ConflictDecision::Insert,
        Some(existing) if existing.eq_ignore_ascii_case(incoming_sha256) => {
            ConflictDecision::SkipIdentical
        }
        Some(_) => ConflictDecision::Conflict,
    }
}

pub fn write_bundle<W: Write + Seek>(
    writer: W,
    mut manifest: SnapshotManifest,
    entries: Vec<BundleEntry>,
) -> Result<W, SnapshotError> {
    if entries.len() > MAX_SNAPSHOT_ENTRIES {
        return Err(SnapshotError::TooManyEntries(entries.len()));
    }
    let mut paths = BTreeSet::new();
    let mut total_size = 0_u64;
    for entry in &entries {
        validate_path(&entry.path)?;
        if entry.path == MANIFEST_PATH || !paths.insert(entry.path.as_str()) {
            return Err(SnapshotError::DuplicatePath(entry.path.clone()));
        }
        total_size = total_size
            .checked_add(entry.bytes.len() as u64)
            .ok_or(SnapshotError::SnapshotTooLarge)?;
        if total_size > MAX_SNAPSHOT_UNCOMPRESSED_BYTES {
            return Err(SnapshotError::SnapshotTooLarge);
        }
    }

    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    manifest.entries.clear();
    for entry in entries {
        let sha256 = sha256_hex(&entry.bytes);
        zip.start_file(&entry.path, options)?;
        zip.write_all(&entry.bytes)?;
        manifest.entries.push(SnapshotEntry {
            path: entry.path,
            kind: entry.kind,
            entity_type: entry.entity_type,
            record_count: entry.record_count,
            size_bytes: entry.bytes.len() as u64,
            sha256,
        });
    }
    manifest.validate()?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    zip.start_file(MANIFEST_PATH, options)?;
    zip.write_all(&manifest_bytes)?;
    Ok(zip.finish()?)
}

pub fn verify_bundle<R: Read + Seek>(reader: R) -> Result<SnapshotManifest, SnapshotError> {
    let mut archive = ZipArchive::new(reader)?;
    if archive.len() > MAX_SNAPSHOT_ENTRIES + 1 {
        return Err(SnapshotError::TooManyEntries(
            archive.len().saturating_sub(1),
        ));
    }
    let manifest: SnapshotManifest = {
        let file = archive
            .by_name(MANIFEST_PATH)
            .map_err(|_| SnapshotError::MissingManifest)?;
        if file.size() > MAX_MANIFEST_BYTES {
            return Err(SnapshotError::ManifestTooLarge);
        }
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(SnapshotError::ManifestTooLarge);
        }
        serde_json::from_slice(&bytes)?
    };
    manifest.validate()?;

    let expected_paths = manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .chain(std::iter::once(MANIFEST_PATH))
        .collect::<BTreeSet<_>>();
    let mut archive_paths = BTreeSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let path = file.name().to_owned();
        validate_path(&path)?;
        if !archive_paths.insert(path.clone()) {
            return Err(SnapshotError::DuplicateArchivePath(path));
        }
        if !expected_paths.contains(path.as_str()) {
            return Err(SnapshotError::UnexpectedEntry(path));
        }
    }
    if archive_paths.len() != expected_paths.len() {
        let missing = expected_paths
            .into_iter()
            .find(|path| !archive_paths.contains(*path))
            .unwrap_or(MANIFEST_PATH);
        return Err(SnapshotError::MissingEntry(missing.to_owned()));
    }

    for entry in &manifest.entries {
        let mut file = archive
            .by_name(&entry.path)
            .map_err(|_| SnapshotError::MissingEntry(entry.path.clone()))?;
        if file.size() != entry.size_bytes {
            return Err(SnapshotError::SizeMismatch(entry.path.clone()));
        }
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut HashWriter(&mut hasher))?;
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&entry.sha256) {
            return Err(SnapshotError::ChecksumMismatch(entry.path.clone()));
        }
    }
    Ok(manifest)
}

struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validate_sha256(value: &str) -> Result<(), SnapshotError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(SnapshotError::InvalidChecksum(value.to_owned()))
    }
}
fn validate_path(path: &str) -> Result<(), SnapshotError> {
    let invalid = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if invalid {
        Err(SnapshotError::InvalidPath(path.to_owned()))
    } else {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("unsupported snapshot schema version {0}")]
    UnsupportedVersion(u32),
    #[error("snapshot manifest is missing")]
    MissingManifest,
    #[error("snapshot manifest exceeds the supported size")]
    ManifestTooLarge,
    #[error("snapshot contains too many entries: {0}")]
    TooManyEntries(usize),
    #[error("snapshot exceeds the supported uncompressed size")]
    SnapshotTooLarge,
    #[error("snapshot entry is missing: {0}")]
    MissingEntry(String),
    #[error("snapshot contains an unexpected entry: {0}")]
    UnexpectedEntry(String),
    #[error("snapshot archive contains a duplicate path: {0}")]
    DuplicateArchivePath(String),
    #[error("duplicate or reserved snapshot path: {0}")]
    DuplicatePath(String),
    #[error("invalid snapshot path: {0}")]
    InvalidPath(String),
    #[error("invalid SHA-256: {0}")]
    InvalidChecksum(String),
    #[error("snapshot entry size mismatch: {0}")]
    SizeMismatch(String),
    #[error("snapshot entry checksum mismatch: {0}")]
    ChecksumMismatch(String),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_verifies_manifest_and_hashes() {
        let now = Utc::now();
        let manifest = SnapshotManifest::new(Uuid::new_v4(), Uuid::new_v4(), None, now);
        let cursor = write_bundle(
            Cursor::new(Vec::new()),
            manifest.clone(),
            vec![BundleEntry {
                path: "data/animals.jsonl".into(),
                kind: EntryKind::JsonLines,
                entity_type: Some("animal".into()),
                record_count: Some(1),
                bytes: b"{\"id\":\"a\"}\n".to_vec(),
            }],
        )
        .unwrap();
        let verified = verify_bundle(Cursor::new(cursor.into_inner())).unwrap();
        assert_eq!(verified.snapshot_id, manifest.snapshot_id);
        assert_eq!(verified.entries.len(), 1);
    }

    #[test]
    fn traversal_paths_are_rejected() {
        let manifest = SnapshotManifest::new(Uuid::new_v4(), Uuid::new_v4(), None, Utc::now());
        let result = write_bundle(
            Cursor::new(Vec::new()),
            manifest,
            vec![BundleEntry {
                path: "../secret".into(),
                kind: EntryKind::Attachment,
                entity_type: None,
                record_count: None,
                bytes: vec![],
            }],
        );
        assert!(matches!(result, Err(SnapshotError::InvalidPath(_))));
    }

    #[test]
    fn conflicts_never_overwrite_silently() {
        assert_eq!(decide_conflict(None, "new"), ConflictDecision::Insert);
        assert_eq!(
            decide_conflict(Some("ABC"), "abc"),
            ConflictDecision::SkipIdentical
        );
        assert_eq!(
            decide_conflict(Some("old"), "new"),
            ConflictDecision::Conflict
        );
    }
}
