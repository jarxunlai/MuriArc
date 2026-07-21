#![forbid(unsafe_code)]

mod audit;
mod migrate;
mod model;

use std::{fs::OpenOptions, io::Write, path::Path};

use serde::Serialize;
use thiserror::Error;

pub use audit::audit_legacy;
pub use migrate::migrate_legacy;
pub use model::{
    CageCountMismatch, DuplicateIdentifierGroup, DuplicateIdentifierSummary, EntityCounts,
    LegacyAuditReport, LegacySchemaReport, MigrationReport, RejectedPedigreeLink, SourceDigest,
    TargetVerification, ValidationIssue, ValidationSeverity,
};

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum LegacyMigrationError {
    #[error("legacy source does not exist or is not a regular file: {0}")]
    SourceNotFound(String),
    #[error("target already exists; refusing to overwrite it: {0}")]
    TargetExists(String),
    #[error("report already exists; refusing to overwrite it: {0}")]
    ReportExists(String),
    #[error("target parent directory does not exist: {0}")]
    TargetParentMissing(String),
    #[error("legacy schema is incompatible; table {table} is missing columns {missing:?}")]
    IncompatibleSchema { table: String, missing: Vec<String> },
    #[error("legacy schema is incompatible; required table is missing: {0}")]
    MissingTable(String),
    #[error("legacy audit found blocking validation issues: {0:?}")]
    AuditBlocked(Vec<ValidationIssue>),
    #[error("legacy source changed while it was being read (before {before}, after {after})")]
    SourceChanged { before: String, after: String },
    #[error("legacy reference cannot be mapped: {0}")]
    InvalidReference(String),
    #[error("target verification failed: {0}")]
    Verification(String),
    #[error("SQLite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, LegacyMigrationError>;

/// Writes a pretty JSON report with create-new semantics.
///
/// Migration reports are part of the provenance record, so an existing report
/// is never silently replaced.
pub fn write_json_report<T: Serialize>(path: &Path, report: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                LegacyMigrationError::ReportExists(path.display().to_string())
            } else {
                LegacyMigrationError::Io(error)
            }
        })?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
