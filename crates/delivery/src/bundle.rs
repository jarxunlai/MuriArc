use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use muriarc_core::ApplicationVersion;
use muriarc_upgrade::DeploymentProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SERVER_BUNDLE_FORMAT: u32 = 1;
pub const SERVER_BUNDLE_MANIFEST: &str = "bundle-manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleFileRole {
    Server,
    Controller,
    UpgradeExecutor,
    Verifier,
    UiAsset,
    BundleManifest,
    SystemdService,
    Sysusers,
    Tmpfiles,
    DeliveryDescriptor,
    EnvironmentExample,
    ComposeFile,
    ComposeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleFile {
    pub path: String,
    pub role: BundleFileRole,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerBundleManifest {
    pub format_version: u32,
    pub application_version: ApplicationVersion,
    pub profile: DeploymentProfile,
    pub files: Vec<BundleFile>,
}

impl ServerBundleManifest {
    pub fn validate(&self) -> Result<(), DeliveryError> {
        if self.format_version != SERVER_BUNDLE_FORMAT || self.profile == DeploymentProfile::Desktop
        {
            return Err(DeliveryError::InvalidBundle(
                "Server bundle format or profile is invalid".to_owned(),
            ));
        }
        let mut paths = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            if file.size_bytes == 0
                || !valid_sha256(&file.sha256)
                || !paths.insert(file.path.as_str())
            {
                return Err(DeliveryError::InvalidBundle(
                    "bundle files must be non-empty, digest-pinned, and path-unique".to_owned(),
                ));
            }
            roles.insert(file.role);
        }
        let required = match self.profile {
            DeploymentProfile::NativeSystem => BTreeSet::from([
                BundleFileRole::Server,
                BundleFileRole::Controller,
                BundleFileRole::UpgradeExecutor,
                BundleFileRole::Verifier,
                BundleFileRole::UiAsset,
                BundleFileRole::SystemdService,
                BundleFileRole::Sysusers,
                BundleFileRole::Tmpfiles,
                BundleFileRole::DeliveryDescriptor,
                BundleFileRole::EnvironmentExample,
            ]),
            DeploymentProfile::ManagedCompose => BTreeSet::from([
                BundleFileRole::Controller,
                BundleFileRole::UpgradeExecutor,
                BundleFileRole::Verifier,
                BundleFileRole::ComposeFile,
                BundleFileRole::ComposeDescriptor,
                BundleFileRole::EnvironmentExample,
            ]),
            DeploymentProfile::Desktop => unreachable!("Desktop rejected above"),
        };
        if !required.is_subset(&roles) {
            return Err(DeliveryError::InvalidBundle(
                "bundle is missing one or more mandatory delivery roles".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, DeliveryError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| DeliveryError::Serialization(error.to_string()))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedServerBundle {
    pub application_version: ApplicationVersion,
    pub profile: DeploymentProfile,
    pub manifest_digest: String,
    pub content_digest: String,
    pub file_count: u64,
    pub total_bytes: u64,
}

pub fn verify_server_bundle(
    root: &Path,
    expected_manifest_digest: Option<&str>,
) -> Result<(ServerBundleManifest, VerifiedServerBundle), DeliveryError> {
    let root_metadata = fs::symlink_metadata(root).map_err(io)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidBundle(
            "bundle root must be a real directory".to_owned(),
        ));
    }
    let manifest_path = root.join(SERVER_BUNDLE_MANIFEST);
    let manifest_metadata = fs::symlink_metadata(&manifest_path).map_err(io)?;
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidBundle(
            "bundle manifest must be a regular non-symlink file".to_owned(),
        ));
    }
    let manifest: ServerBundleManifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(io)?)
            .map_err(|error| DeliveryError::Serialization(error.to_string()))?;
    manifest.validate()?;
    let manifest_digest = manifest.digest()?;
    if expected_manifest_digest.is_some_and(|expected| expected != manifest_digest) {
        return Err(DeliveryError::InvalidBundle(
            "bundle manifest differs from signed metadata".to_owned(),
        ));
    }

    let observed = inventory(root)?;
    let expected = manifest
        .files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    if observed.keys().collect::<BTreeSet<_>>() != expected.keys().collect::<BTreeSet<_>>() {
        return Err(DeliveryError::InvalidBundle(
            "bundle contains missing or unregistered files".to_owned(),
        ));
    }
    let mut total_bytes = 0_u64;
    for (relative, absolute) in &observed {
        let (size, digest) = hash_file(absolute)?;
        let expected_file = expected[relative];
        if size != expected_file.size_bytes || digest != expected_file.sha256 {
            return Err(DeliveryError::InvalidBundle(format!(
                "bundle file {relative} length or digest differs"
            )));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| DeliveryError::InvalidBundle("bundle size overflowed".to_owned()))?;
    }
    let content_digest = content_digest(&manifest.files)?;
    let verified = VerifiedServerBundle {
        application_version: manifest.application_version.clone(),
        profile: manifest.profile,
        manifest_digest,
        content_digest,
        file_count: u64::try_from(observed.len())
            .map_err(|_| DeliveryError::InvalidBundle("file count overflowed".to_owned()))?,
        total_bytes,
    };
    Ok((manifest, verified))
}

pub fn stage_verified_release(
    bundle_root: &Path,
    manifest: &ServerBundleManifest,
    release_root: &Path,
) -> Result<PathBuf, DeliveryError> {
    manifest.validate()?;
    let destination = release_root.join(manifest.application_version.as_str());
    if destination.exists() {
        return Err(DeliveryError::AlreadyInstalled(destination));
    }
    fs::create_dir_all(release_root).map_err(io)?;
    let staging = release_root.join(format!(
        ".staging-{}-{}",
        manifest.application_version,
        std::process::id()
    ));
    if staging.exists() {
        return Err(DeliveryError::InvalidBundle(
            "release staging path already exists".to_owned(),
        ));
    }
    fs::create_dir(&staging).map_err(io)?;
    let result = (|| {
        for file in &manifest.files {
            let source = bundle_root.join(&file.path);
            let target = staging.join(&file.path);
            let parent = target.parent().ok_or_else(|| {
                DeliveryError::InvalidBundle("bundle target has no parent".to_owned())
            })?;
            fs::create_dir_all(parent).map_err(io)?;
            copy_create_new(&source, &target, file.role)?;
        }
        copy_create_new(
            &bundle_root.join(SERVER_BUNDLE_MANIFEST),
            &staging.join(SERVER_BUNDLE_MANIFEST),
            BundleFileRole::BundleManifest,
        )?;
        fs::rename(&staging, &destination).map_err(io)?;
        Ok(destination.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(unix)]
pub fn activate_release_link(release: &Path, current: &Path) -> Result<(), DeliveryError> {
    use std::os::unix::fs::symlink;

    if !release.is_absolute() || !release.is_dir() || !current.is_absolute() {
        return Err(DeliveryError::InvalidPolicy(
            "release activation requires absolute existing release paths".to_owned(),
        ));
    }
    let parent = current.parent().ok_or_else(|| {
        DeliveryError::InvalidPolicy("current release link has no parent".to_owned())
    })?;
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = parent.join(format!(".current-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(io)?;
    }
    symlink(release, &temporary).map_err(io)?;
    fs::rename(&temporary, current).map_err(io)
}

fn inventory(root: &Path) -> Result<BTreeMap<String, PathBuf>, DeliveryError> {
    let mut output = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(io)? {
            let entry = entry.map_err(io)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(io)?;
            if metadata.file_type().is_symlink() {
                return Err(DeliveryError::InvalidBundle(
                    "bundle may not contain symlinks".to_owned(),
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(DeliveryError::InvalidBundle(
                    "bundle may contain only regular files".to_owned(),
                ));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| DeliveryError::InvalidBundle("bundle path escaped root".to_owned()))?
                .to_str()
                .ok_or_else(|| {
                    DeliveryError::InvalidBundle("bundle paths must be UTF-8".to_owned())
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if relative == SERVER_BUNDLE_MANIFEST {
                continue;
            }
            validate_relative_path(&relative)?;
            if output.insert(relative, path).is_some() {
                return Err(DeliveryError::InvalidBundle(
                    "bundle contains duplicate normalized paths".to_owned(),
                ));
            }
        }
    }
    Ok(output)
}

fn content_digest(files: &[BundleFile]) -> Result<String, DeliveryError> {
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    hasher.update(b"MuriArc/server-bundle/v1\0");
    for file in files {
        validate_relative_path(&file.path)?;
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.size_bytes.to_be_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn copy_create_new(
    source: &Path,
    target: &Path,
    role: BundleFileRole,
) -> Result<(), DeliveryError> {
    let metadata = fs::symlink_metadata(source).map_err(io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidBundle(
            "bundle copy source is not a regular file".to_owned(),
        ));
    }
    let mut input = File::open(source).map_err(io)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let executable = matches!(
            role,
            BundleFileRole::Server
                | BundleFileRole::Controller
                | BundleFileRole::UpgradeExecutor
                | BundleFileRole::Verifier
        );
        options.mode(if executable { 0o750 } else { 0o640 });
    }
    let mut output = options.open(target).map_err(io)?;
    std::io::copy(&mut input, &mut output).map_err(io)?;
    output.flush().map_err(io)?;
    output.sync_all().map_err(io)
}

fn hash_file(path: &Path) -> Result<(u64, String), DeliveryError> {
    let mut file = File::open(path).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(read).map_err(|_| {
                DeliveryError::InvalidBundle("file size exceeds supported range".to_owned())
            })?)
            .ok_or_else(|| DeliveryError::InvalidBundle("file size overflowed".to_owned()))?;
        hasher.update(&buffer[..read]);
    }
    Ok((length, format!("sha256:{:x}", hasher.finalize())))
}

fn validate_relative_path(path: &str) -> Result<(), DeliveryError> {
    let value = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || value.is_absolute()
        || value
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(DeliveryError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn io(error: std::io::Error) -> DeliveryError {
    DeliveryError::Io(error.to_string())
}

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error("unsafe bundle path: {0}")]
    UnsafePath(String),
    #[error("invalid delivery bundle: {0}")]
    InvalidBundle(String),
    #[error("invalid delivery policy: {0}")]
    InvalidPolicy(String),
    #[error("delivery prerequisite is missing: {0}")]
    Prerequisite(String),
    #[error("release is already installed: {}", .0.display())]
    AlreadyInstalled(PathBuf),
    #[error("service command failed: {0}")]
    Command(String),
    #[error("delivery I/O failed: {0}")]
    Io(String),
    #[error("delivery serialization failed: {0}")]
    Serialization(String),
}
