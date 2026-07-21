use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use muriarc_core::Attachment;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom},
};
use uuid::Uuid;

/// V1 uses the same bounded object size for personal and shared deployments.
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AttachmentFileError {
    #[error("attachment exceeds the configured size limit")]
    TooLarge,
    #[error("attachment storage contains an unsafe path")]
    UnsafePath,
    #[error("attachment object already exists")]
    AlreadyExists,
    #[error("attachment object is missing")]
    Missing,
    #[error("attachment object failed integrity verification")]
    Integrity,
    #[error("attachment storage I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub struct StoredAttachmentObject {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size_bytes: i64,
    pub sha256: String,
}

pub struct VerifiedAttachmentObject {
    pub file: File,
    pub size_bytes: u64,
}

/// Filesystem adapter for immutable attachment objects.
///
/// Database metadata remains owned by `MuriArcStore`; this service only
/// installs, verifies, reads and removes opaque content objects.
#[derive(Debug, Clone)]
pub struct AttachmentFiles {
    root: Arc<PathBuf>,
    max_bytes: u64,
}

impl AttachmentFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_limit(root, MAX_ATTACHMENT_BYTES)
    }

    pub fn with_limit(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: Arc::new(root.into()),
            max_bytes,
        }
    }

    pub fn root(&self) -> &Path {
        self.root.as_ref()
    }

    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub async fn initialize(&self) -> Result<(), AttachmentFileError> {
        let root = canonical_storage_root(self.root()).await?;
        ensure_subdirectory(&root, &[".tmp"]).await?;
        ensure_subdirectory(&root, &["objects"]).await?;
        Ok(())
    }

    pub async fn write_bytes(
        &self,
        id: Uuid,
        bytes: &[u8],
    ) -> Result<StoredAttachmentObject, AttachmentFileError> {
        self.write_reader(id, std::io::Cursor::new(bytes)).await
    }

    pub async fn write_reader<R>(
        &self,
        id: Uuid,
        mut reader: R,
    ) -> Result<StoredAttachmentObject, AttachmentFileError>
    where
        R: AsyncRead + Unpin,
    {
        let root = canonical_storage_root(self.root()).await?;
        let temp_dir = ensure_subdirectory(&root, &[".tmp"]).await?;
        let temp_path = temp_dir.join(format!("upload-{id}-{}.part", Uuid::new_v4()));
        let mut installed_path = None;

        let result = async {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .await?;
            let mut hasher = Sha256::new();
            let mut size = 0_u64;
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                size = size
                    .checked_add(read as u64)
                    .ok_or(AttachmentFileError::TooLarge)?;
                if size > self.max_bytes {
                    return Err(AttachmentFileError::TooLarge);
                }
                file.write_all(&buffer[..read]).await?;
                hasher.update(&buffer[..read]);
            }
            file.flush().await?;
            file.sync_all().await?;
            drop(file);

            let sha256 = format!("{:x}", hasher.finalize());
            let object_dir =
                ensure_subdirectory(&root, &["objects", &sha256[..2], &sha256[2..4], &sha256])
                    .await?;
            let destination = object_dir.join(id.to_string());
            match fs::hard_link(&temp_path, &destination).await {
                Ok(()) => installed_path = Some(destination.clone()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(AttachmentFileError::AlreadyExists);
                }
                Err(error) => return Err(error.into()),
            }

            fs::remove_file(&temp_path).await?;
            Ok(StoredAttachmentObject {
                relative_path: format!(
                    "objects/{}/{}/{}/{}",
                    &sha256[..2],
                    &sha256[2..4],
                    sha256,
                    id
                ),
                absolute_path: destination,
                size_bytes: i64::try_from(size).map_err(|_| AttachmentFileError::TooLarge)?,
                sha256,
            })
        }
        .await;

        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
            if let Some(path) = installed_path {
                let _ = fs::remove_file(path).await;
            }
        }
        result
    }

    pub async fn remove_installed_object(
        &self,
        object: &StoredAttachmentObject,
    ) -> Result<(), AttachmentFileError> {
        let root = canonical_storage_root(self.root()).await?;
        let parent = object
            .absolute_path
            .parent()
            .ok_or(AttachmentFileError::UnsafePath)?;
        let canonical_parent = fs::canonicalize(parent).await?;
        if !canonical_parent.starts_with(&root) {
            return Err(AttachmentFileError::UnsafePath);
        }
        match fs::symlink_metadata(&object.absolute_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(AttachmentFileError::UnsafePath)
            }
            Ok(_) => fs::remove_file(&object.absolute_path)
                .await
                .map_err(Into::into),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn open_verified(
        &self,
        attachment: &Attachment,
    ) -> Result<VerifiedAttachmentObject, AttachmentFileError> {
        if attachment.size_bytes < 0
            || attachment.size_bytes as u64 > self.max_bytes
            || attachment.sha256.len() != 64
            || !attachment
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AttachmentFileError::Integrity);
        }
        let root = canonical_storage_root(self.root()).await?;
        let path = resolve_relative_path(&root, &attachment.relative_path)?;
        let path_metadata = match fs::symlink_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(AttachmentFileError::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
            return Err(AttachmentFileError::UnsafePath);
        }
        let canonical_path = fs::canonicalize(&path).await?;
        if !canonical_path.starts_with(&root) {
            return Err(AttachmentFileError::UnsafePath);
        }

        let mut file = File::open(&canonical_path).await?;
        let metadata = file.metadata().await?;
        if metadata.len() != attachment.size_bytes as u64 {
            return Err(AttachmentFileError::Integrity);
        }
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            size = size
                .checked_add(read as u64)
                .ok_or(AttachmentFileError::TooLarge)?;
            if size > self.max_bytes {
                return Err(AttachmentFileError::TooLarge);
            }
            hasher.update(&buffer[..read]);
        }
        let actual_hash = format!("{:x}", hasher.finalize());
        if size != attachment.size_bytes as u64
            || !actual_hash.eq_ignore_ascii_case(&attachment.sha256)
            || file.metadata().await?.len() != size
        {
            return Err(AttachmentFileError::Integrity);
        }
        file.seek(SeekFrom::Start(0)).await?;
        Ok(VerifiedAttachmentObject {
            file,
            size_bytes: size,
        })
    }

    pub async fn read_verified_bytes(
        &self,
        attachment: &Attachment,
    ) -> Result<Vec<u8>, AttachmentFileError> {
        let mut verified = self.open_verified(attachment).await?;
        let capacity =
            usize::try_from(verified.size_bytes).map_err(|_| AttachmentFileError::TooLarge)?;
        let mut bytes = Vec::with_capacity(capacity);
        verified.file.read_to_end(&mut bytes).await?;
        if bytes.len() as u64 != verified.size_bytes
            || !format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(&attachment.sha256)
        {
            return Err(AttachmentFileError::Integrity);
        }
        Ok(bytes)
    }
}

async fn canonical_storage_root(configured_root: &Path) -> Result<PathBuf, AttachmentFileError> {
    fs::create_dir_all(configured_root).await?;
    let metadata = fs::symlink_metadata(configured_root).await?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AttachmentFileError::UnsafePath);
    }
    Ok(fs::canonicalize(configured_root).await?)
}

async fn ensure_subdirectory(
    root: &Path,
    components: &[&str],
) -> Result<PathBuf, AttachmentFileError> {
    let mut current = root.to_path_buf();
    for component in components {
        if component.is_empty()
            || *component == "."
            || *component == ".."
            || component.contains(['/', '\\'])
        {
            return Err(AttachmentFileError::UnsafePath);
        }
        current.push(component);
        match fs::create_dir(&current).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(&current).await?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AttachmentFileError::UnsafePath);
        }
        let canonical = fs::canonicalize(&current).await?;
        if !canonical.starts_with(root) {
            return Err(AttachmentFileError::UnsafePath);
        }
        current = canonical;
    }
    Ok(current)
}

fn resolve_relative_path(root: &Path, relative: &str) -> Result<PathBuf, AttachmentFileError> {
    if relative.is_empty() || relative.contains('\\') {
        return Err(AttachmentFileError::UnsafePath);
    }
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err(AttachmentFileError::UnsafePath);
    }
    let mut resolved = root.to_path_buf();
    let mut count = 0;
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                resolved.push(value);
                count += 1;
            }
            _ => return Err(AttachmentFileError::UnsafePath),
        }
    }
    if count == 0 {
        return Err(AttachmentFileError::UnsafePath);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use muriarc_core::RecordMeta;

    use super::*;

    fn attachment(object: &StoredAttachmentObject) -> Attachment {
        Attachment {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            project_id: None,
            entity_type: "animal".to_owned(),
            entity_id: Uuid::new_v4(),
            file_name: "result.txt".to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: object.relative_path.clone(),
            size_bytes: object.size_bytes,
            sha256: object.sha256.clone(),
            version: 1,
            meta: RecordMeta::new(Utc::now()),
        }
    }

    async fn directory_is_empty(path: &Path) -> bool {
        fs::read_dir(path)
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap()
            .is_none()
    }

    #[tokio::test]
    async fn empty_objects_verify_and_object_keys_never_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let files = AttachmentFiles::with_limit(root.path(), 4);
        let id = Uuid::new_v4();
        let stored = files.write_bytes(id, b"").await.unwrap();
        assert_eq!(stored.size_bytes, 0);
        assert_eq!(
            stored.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(
            files
                .read_verified_bytes(&attachment(&stored))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            files.write_bytes(id, b"").await,
            Err(AttachmentFileError::AlreadyExists)
        ));
        assert!(stored.absolute_path.is_file());
        assert!(directory_is_empty(&root.path().join(".tmp")).await);
    }

    #[tokio::test]
    async fn oversized_and_polluted_objects_fail_closed_and_clean_staging() {
        let root = tempfile::tempdir().unwrap();
        let files = AttachmentFiles::with_limit(root.path(), 4);
        assert!(matches!(
            files.write_bytes(Uuid::new_v4(), b"12345").await,
            Err(AttachmentFileError::TooLarge)
        ));
        assert!(directory_is_empty(&root.path().join(".tmp")).await);

        let stored = files.write_bytes(Uuid::new_v4(), b"1234").await.unwrap();
        let record = attachment(&stored);
        fs::write(&stored.absolute_path, b"xxxx").await.unwrap();
        assert!(matches!(
            files.read_verified_bytes(&record).await,
            Err(AttachmentFileError::Integrity)
        ));
    }

    #[tokio::test]
    async fn unsafe_metadata_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let files = AttachmentFiles::with_limit(root.path(), 16);
        let stored = files.write_bytes(Uuid::new_v4(), b"trusted").await.unwrap();
        let mut record = attachment(&stored);
        record.relative_path = "../outside".to_owned();
        assert!(matches!(
            files.open_verified(&record).await,
            Err(AttachmentFileError::UnsafePath)
        ));
    }
}
