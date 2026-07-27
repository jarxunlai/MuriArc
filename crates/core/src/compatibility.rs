use std::{collections::BTreeMap, fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const CURRENT_APPLICATION_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CURRENT_DATA_EPOCH: &str = "preview_epoch_0";
pub const CURRENT_GATEWAY_CONTRACT_REVISION: &str = "gateway-v1";
/// A formal release must change this to `permanent-upgrade` together with its
/// version/Epoch transition. The RC readiness gate rejects preview support.
pub const CURRENT_RELEASE_SUPPORT: &str = "preview-only-adoption";
pub const GENERATION_MANIFEST_FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Sqlite,
    Postgres,
}

impl BackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

macro_rules! validated_string_type {
    ($name:ident, $validator:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                $validator(value)?;
                Ok(Self(value.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

fn validate_application_version(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err("application version must be a non-empty SemVer-compatible token".to_owned());
    }
    Ok(())
}

fn validate_data_epoch(value: &str) -> Result<(), String> {
    if value == "preview_epoch_0"
        || (value.len() == 5
            && value.starts_with('E')
            && value[1..].bytes().all(|byte| byte.is_ascii_digit()))
    {
        Ok(())
    } else {
        Err("data epoch must be preview_epoch_0 or E followed by four digits".to_owned())
    }
}

fn validate_backend_state_digest(value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err("backend state digest must use sha256".to_owned());
    };
    if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("backend state digest must contain 64 hexadecimal characters".to_owned())
    }
}

fn validate_gateway_contract_revision(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err("gateway contract revision contains unsupported characters".to_owned());
    }
    Ok(())
}

validated_string_type!(ApplicationVersion, validate_application_version);
validated_string_type!(DataEpoch, validate_data_epoch);
validated_string_type!(BackendStateDigest, validate_backend_state_digest);
validated_string_type!(GatewayContractRevision, validate_gateway_contract_revision);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationFingerprint {
    pub version: i64,
    pub description: String,
    pub checksum_sha384: String,
}

pub fn backend_state_digest(
    backend: BackendKind,
    migrations: &[MigrationFingerprint],
) -> BackendStateDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"MuriArc/backend-state/v1\0");
    hasher.update(backend.as_str().as_bytes());
    hasher.update(b"\0");
    for migration in migrations {
        hasher.update(migration.version.to_be_bytes());
        hasher.update(b"\0");
        hasher.update(migration.description.as_bytes());
        hasher.update(b"\0");
        hasher.update(migration.checksum_sha384.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{:x}", hasher.finalize())
        .parse()
        .expect("SHA-256 formatting always produces a valid backend state digest")
}

pub fn bytes_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseIdentity {
    pub application_version: ApplicationVersion,
    pub data_epoch: DataEpoch,
    pub backend_state_digest: BackendStateDigest,
    pub gateway_contract_revision: GatewayContractRevision,
}

impl ReleaseIdentity {
    pub fn current(backend: BackendKind, migrations: &[MigrationFingerprint]) -> Self {
        Self {
            application_version: CURRENT_APPLICATION_VERSION
                .parse()
                .expect("package version must be a valid ApplicationVersion"),
            data_epoch: CURRENT_DATA_EPOCH
                .parse()
                .expect("current epoch constant must be valid"),
            backend_state_digest: backend_state_digest(backend, migrations),
            gateway_contract_revision: CURRENT_GATEWAY_CONTRACT_REVISION
                .parse()
                .expect("gateway revision constant must be valid"),
        }
    }

    pub fn parse(
        application_version: String,
        data_epoch: String,
        backend_state_digest: String,
        gateway_contract_revision: String,
    ) -> Result<Self, String> {
        Ok(Self {
            application_version: application_version.try_into()?,
            data_epoch: data_epoch.try_into()?,
            backend_state_digest: backend_state_digest.try_into()?,
            gateway_contract_revision: gateway_contract_revision.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentState {
    pub identity: ReleaseIdentity,
    pub generation_id: Uuid,
    pub write_lease_id: Option<Uuid>,
    pub first_write_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityIssue {
    pub code: String,
    pub detail: String,
}

impl CompatibilityIssue {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub backend: BackendKind,
    pub expected: ReleaseIdentity,
    pub observed: Option<DeploymentState>,
    pub issues: Vec<CompatibilityIssue>,
}

impl CompatibilityReport {
    pub fn is_compatible(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn require_compatible(&self) -> Result<&DeploymentState, String> {
        if self.issues.is_empty() {
            self.observed
                .as_ref()
                .ok_or_else(|| "deployment_state_missing".to_owned())
        } else {
            Err(self
                .issues
                .iter()
                .map(|issue| format!("{}: {}", issue.code, issue.detail))
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    /// Read-only activation deliberately has no Write Lease. Every other
    /// identity, migration, generation, and storage compatibility issue still
    /// blocks readiness. This is not a general-purpose bypass for ordinary
    /// Server startup.
    pub fn require_read_only_compatible(&self) -> Result<&DeploymentState, String> {
        let state = self
            .observed
            .as_ref()
            .ok_or_else(|| "deployment_state_missing".to_owned())?;
        if state.identity != self.expected
            || self
                .issues
                .iter()
                .any(|issue| issue.code != "write_lease_missing")
        {
            return Err(self
                .issues
                .iter()
                .map(|issue| format!("{}: {}", issue.code, issue.detail))
                .collect::<Vec<_>>()
                .join("; "));
        }
        if self
            .issues
            .iter()
            .filter(|issue| issue.code == "write_lease_missing")
            .count()
            != 1
            || state.write_lease_id.is_some()
        {
            return Err("read_only_activation_requires_exactly_one_missing_write_lease".to_owned());
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentRecoveryInventory {
    pub attachment_records: u64,
    pub encrypted_secret_records: u64,
    pub secret_reference_records: u64,
    pub ai_history_records: u64,
    pub audit_records: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentGenerationManifest {
    pub format_version: u32,
    pub generation_id: Uuid,
    pub data_epoch: DataEpoch,
    pub backend_state_digest: BackendStateDigest,
}

impl DeploymentGenerationManifest {
    pub fn from_state(state: &DeploymentState) -> Self {
        Self {
            format_version: GENERATION_MANIFEST_FORMAT,
            generation_id: state.generation_id,
            data_epoch: state.identity.data_epoch.clone(),
            backend_state_digest: state.identity.backend_state_digest.clone(),
        }
    }

    pub fn validate(&self, state: &DeploymentState) -> Result<(), CompatibilityIssue> {
        if self.format_version != GENERATION_MANIFEST_FORMAT {
            return Err(CompatibilityIssue::new(
                "generation_manifest_format_mismatch",
                format!(
                    "expected format {}, observed {}",
                    GENERATION_MANIFEST_FORMAT, self.format_version
                ),
            ));
        }
        if self.generation_id != state.generation_id
            || self.data_epoch != state.identity.data_epoch
            || self.backend_state_digest != state.identity.backend_state_digest
        {
            return Err(CompatibilityIssue::new(
                "generation_manifest_mismatch",
                "database and data-root generation identities differ",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationClass {
    M0,
    M1,
    M2,
    M3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiPersistenceImpact {
    UiPresentation,
    UiDataAccess,
    UiPersistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityReview {
    Known,
    Legacy,
    Unknown,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreservedValue<T> {
    pub raw: String,
    pub known: Option<T>,
    pub review: CompatibilityReview,
}

impl<T> PreservedValue<T> {
    pub fn decode(raw: impl Into<String>, decoder: impl FnOnce(&str) -> Option<T>) -> Self {
        let raw = raw.into();
        match decoder(&raw) {
            Some(known) => Self {
                raw,
                known: Some(known),
                review: CompatibilityReview::Known,
            },
            None => Self {
                raw,
                known: None,
                review: CompatibilityReview::NeedsReview,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionedJson<T> {
    pub schema_version: u32,
    pub payload: T,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReleaseCatalogEntry {
    pub application_version: &'static str,
    pub data_epoch: &'static str,
    pub gateway_contract_revision: &'static str,
    pub support: &'static str,
}

pub const RELEASE_CATALOG: &[ReleaseCatalogEntry] = &[ReleaseCatalogEntry {
    application_version: CURRENT_APPLICATION_VERSION,
    data_epoch: CURRENT_DATA_EPOCH,
    gateway_contract_revision: CURRENT_GATEWAY_CONTRACT_REVISION,
    support: CURRENT_RELEASE_SUPPORT,
}];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseArtifact {
    pub media_type: String,
    pub digest: BackendStateDigest,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub format_version: u32,
    pub application_version: ApplicationVersion,
    pub data_epoch: DataEpoch,
    pub gateway_contract_revision: GatewayContractRevision,
    pub backend_states: BTreeMap<BackendKind, BackendStateDigest>,
    pub postgres_major: u16,
    pub bootstrap_protocol_revision: u32,
    pub controller_protocol_min: u32,
    pub controller_protocol_max: u32,
    pub migration_class: MigrationClass,
    pub artifacts: BTreeMap<String, ReleaseArtifact>,
}

impl ReleaseManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != 1 {
            return Err("release manifest format must be 1".to_owned());
        }
        if self.postgres_major < 17 {
            return Err("release manifest requires PostgreSQL 17 or newer".to_owned());
        }
        if self.bootstrap_protocol_revision == 0
            || self.controller_protocol_min == 0
            || self.controller_protocol_min > self.controller_protocol_max
        {
            return Err("release manifest control protocol range is invalid".to_owned());
        }
        for backend in [BackendKind::Sqlite, BackendKind::Postgres] {
            if !self.backend_states.contains_key(&backend) {
                return Err(format!(
                    "release manifest is missing the {} backend state",
                    backend.as_str()
                ));
            }
        }
        if self.artifacts.is_empty() {
            return Err("release manifest must pin at least one artifact".to_owned());
        }
        for (name, artifact) in &self.artifacts {
            if name.trim().is_empty()
                || artifact.media_type.trim().is_empty()
                || artifact.size_bytes == 0
            {
                return Err("release manifest contains an invalid artifact".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PersistentDataRegistryEntry {
    pub id: &'static str,
    pub kind: &'static str,
    pub owner: &'static str,
    pub compatibility_floor: &'static str,
    pub policy: &'static str,
}

pub const PERSISTENT_DATA_REGISTRY: &[PersistentDataRegistryEntry] = &[
    PersistentDataRegistryEntry {
        id: "database.business_and_auth",
        kind: "database_objects",
        owner: "store-adapters",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "append-only-migrations; expand-backfill-switch-contract",
    },
    PersistentDataRegistryEntry {
        id: "database.credential_policy_and_login_backoff",
        kind: "security_state",
        owner: "server-auth",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "policy-versioned; hmac-identity; generation-fenced; joint-recovery",
    },
    PersistentDataRegistryEntry {
        id: "database.persisted_enum_and_json",
        kind: "semantic_payloads",
        owner: "core-application-ai",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "explicit-version; preserve-unknown; needs-review",
    },
    PersistentDataRegistryEntry {
        id: "crypto.ai_envelopes",
        kind: "encrypted_envelopes",
        owner: "server-ai-settings",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "key-versioned; no-implicit-rotation; joint-recovery",
    },
    PersistentDataRegistryEntry {
        id: "filesystem.attachments_data",
        kind: "file_layout",
        owner: "data",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "content-hash; generation-bound; joint-recovery",
    },
    PersistentDataRegistryEntry {
        id: "snapshot.lab_archive",
        kind: "snapshot_format",
        owner: "snapshot-data",
        compatibility_floor: CURRENT_DATA_EPOCH,
        policy: "versioned-manifest; verified-read; no-silent-drop",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_release_manifest() -> ReleaseManifest {
        let digest: BackendStateDigest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .parse()
                .unwrap();
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
                "native-linux-x86_64".to_owned(),
                ReleaseArtifact {
                    media_type: "application/vnd.muriarc.native.v1+tar".to_owned(),
                    digest,
                    size_bytes: 1024,
                },
            )]),
        }
    }

    #[test]
    fn backend_digest_is_order_and_backend_sensitive() {
        let migrations = vec![MigrationFingerprint {
            version: 1,
            description: "initial".to_owned(),
            checksum_sha384: "abc".to_owned(),
        }];
        assert_ne!(
            backend_state_digest(BackendKind::Sqlite, &migrations),
            backend_state_digest(BackendKind::Postgres, &migrations)
        );
        let mut changed = migrations.clone();
        changed[0].checksum_sha384 = "def".to_owned();
        assert_ne!(
            backend_state_digest(BackendKind::Sqlite, &migrations),
            backend_state_digest(BackendKind::Sqlite, &changed)
        );
    }

    #[test]
    fn unknown_persisted_value_is_preserved_for_review() {
        let decoded = PreservedValue::<u8>::decode("future_state", |_| None);
        assert_eq!(decoded.raw, "future_state");
        assert_eq!(decoded.known, None);
        assert_eq!(decoded.review, CompatibilityReview::NeedsReview);
    }

    #[test]
    fn release_support_matches_preview_or_permanent_epoch() {
        let expected = if CURRENT_DATA_EPOCH == "preview_epoch_0" {
            "preview-only-adoption"
        } else {
            "permanent-upgrade"
        };
        assert_eq!(CURRENT_RELEASE_SUPPORT, expected);
        assert_eq!(
            RELEASE_CATALOG.last().unwrap().support,
            CURRENT_RELEASE_SUPPORT
        );
    }

    #[test]
    fn release_manifest_requires_both_backends_and_pinned_artifacts() {
        let manifest = valid_release_manifest();
        assert_eq!(manifest.validate(), Ok(()));

        let mut missing_backend = manifest.clone();
        missing_backend
            .backend_states
            .remove(&BackendKind::Postgres);
        assert_eq!(
            missing_backend.validate().unwrap_err(),
            "release manifest is missing the postgres backend state"
        );

        let mut empty_artifacts = manifest;
        empty_artifacts.artifacts.clear();
        assert_eq!(
            empty_artifacts.validate().unwrap_err(),
            "release manifest must pin at least one artifact"
        );
    }

    #[test]
    fn release_manifest_rejects_invalid_control_protocol_ranges() {
        let mut manifest = valid_release_manifest();
        manifest.controller_protocol_min = 2;
        manifest.controller_protocol_max = 1;
        assert_eq!(
            manifest.validate().unwrap_err(),
            "release manifest control protocol range is invalid"
        );
    }

    #[test]
    fn read_only_compatibility_allows_only_the_missing_lease_boundary() {
        let identity = ReleaseIdentity::parse(
            "1.0.0".to_owned(),
            "E0001".to_owned(),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            "gateway-v1".to_owned(),
        )
        .unwrap();
        let report = CompatibilityReport {
            backend: BackendKind::Postgres,
            expected: identity.clone(),
            observed: Some(DeploymentState {
                identity,
                generation_id: Uuid::new_v4(),
                write_lease_id: None,
                first_write_at: None,
                updated_at: Utc::now(),
            }),
            issues: vec![CompatibilityIssue::new(
                "write_lease_missing",
                "read-only activation deliberately has no lease",
            )],
        };
        assert!(report.require_read_only_compatible().is_ok());
        let mut changed = report;
        changed.issues.push(CompatibilityIssue::new(
            "backend_state_digest_mismatch",
            "wrong schema",
        ));
        assert!(changed.require_read_only_compatible().is_err());
    }
}
