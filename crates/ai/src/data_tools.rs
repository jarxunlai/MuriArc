use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_core::{AiImportResolution, AuditContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{DomainToolOutput, DomainToolRequest, ToolExecutionError, ToolName, WriteDraft};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiExportResource {
    Animals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiExportFormat {
    Csv,
    Xlsx,
}

/// Transport-neutral artifact metadata that is safe to place in model context.
///
/// Object digests, paths, download URLs and bytes remain in the ordinary
/// human-facing artifact workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiExportArtifactView {
    pub kind: String,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiSourceImportKind {
    Animal,
    Measurement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImportPreviewArguments {
    pub source_id: Uuid,
    pub import_kind: AiSourceImportKind,
    pub experiment_id: Option<Uuid>,
}

pub use muriarc_core::AI_SOURCE_IMPORT_JOB_BINDING_KEY as SOURCE_IMPORT_JOB_BINDING_KEY;

/// Trusted metadata persisted with a source-derived import Job.
///
/// The backend constructs this only after re-reading and validating the source.
/// It is never accepted in tool arguments or an approval payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceImportJobBinding {
    pub schema_version: u32,
    pub source_id: Uuid,
    pub source_revision: i64,
    pub source_project_id: Option<Uuid>,
    pub attachment_id: Uuid,
    pub attachment_revision: i64,
    pub conversation_id: Uuid,
}

impl SourceImportJobBinding {
    pub const SCHEMA_VERSION: u32 = 1;

    pub const fn new(
        source_id: Uuid,
        source_revision: i64,
        source_project_id: Option<Uuid>,
        attachment_id: Uuid,
        attachment_revision: i64,
        conversation_id: Uuid,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            source_id,
            source_revision,
            source_project_id,
            attachment_id,
            attachment_revision,
            conversation_id,
        }
    }

    pub fn validate(&self) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && !self.source_id.is_nil()
            && self.source_revision > 0
            && !self.attachment_id.is_nil()
            && self.attachment_revision > 0
            && !self.conversation_id.is_nil()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPreviewArguments {
    pub job_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCommitDraftArguments {
    pub job_id: Uuid,
    pub preview_hash: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDraftIssueSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDraftPreviewIssue {
    pub row: Option<usize>,
    pub field: Option<String>,
    pub severity: ImportDraftIssueSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDraftPreviewRow {
    pub row_number: usize,
    pub animal_id: Uuid,
    pub animal_display_id: String,
    pub measurement_key: String,
    pub value: String,
    pub unit: Option<String>,
    pub measured_at: String,
}

/// Bounded, display-safe projection of the exact import preview a human is
/// being asked to approve.
///
/// This projection is persisted inside the immutable draft payload and
/// re-derived from the pending import immediately before apply. Object paths,
/// uploaded bytes, and unbounded rows never enter it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDraftPreviewSummary {
    pub import_kind: AiSourceImportKind,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub file_name: String,
    pub sheet_name: String,
    pub total_rows: usize,
    pub accepted_rows: usize,
    pub issue_count: usize,
    pub issues_truncated: bool,
    pub can_confirm: bool,
    pub preview_rows: Vec<ImportDraftPreviewRow>,
    pub preview_rows_truncated: bool,
    pub issues: Vec<ImportDraftPreviewIssue>,
}

#[derive(Debug, Deserialize)]
struct PublicImportDraftPreview {
    import_kind: AiSourceImportKind,
    project_id: Uuid,
    experiment_id: Uuid,
    file_name: String,
    sheet_name: String,
    total_rows: usize,
    accepted_rows: usize,
    issue_count: usize,
    issues_truncated: bool,
    can_confirm: bool,
    preview_rows: Vec<BTreeMap<String, String>>,
    preview_rows_truncated: bool,
    issues: Vec<ImportDraftPreviewIssue>,
}

impl ImportDraftPreviewSummary {
    pub const MAX_PREVIEW_ROWS: usize = 20;
    pub const MAX_PREVIEW_ISSUES: usize = 50;

    /// Parse the snake_case projection produced by a trusted data backend.
    pub fn from_public_preview(value: &Value) -> Result<Self, ToolExecutionError> {
        let raw: PublicImportDraftPreview = serde_json::from_value(value.clone())
            .map_err(|_| rejected("invalid_import_preview"))?;
        let mut preview_rows = Vec::with_capacity(raw.preview_rows.len());
        for mut row in raw.preview_rows {
            if row.len() != 7 {
                return Err(rejected("invalid_import_preview"));
            }
            let row_number = row
                .remove("row_number")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            let animal_id = row
                .remove("animal_id")
                .and_then(|value| Uuid::parse_str(&value).ok())
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            let animal_display_id = row
                .remove("animal_display_id")
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            let measurement_key = row
                .remove("measurement_key")
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            let value = row
                .remove("value")
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            let unit = row.remove("unit").filter(|value| !value.is_empty());
            let measured_at = row
                .remove("measured_at")
                .ok_or_else(|| rejected("invalid_import_preview"))?;
            if !row.is_empty() {
                return Err(rejected("invalid_import_preview"));
            }
            preview_rows.push(ImportDraftPreviewRow {
                row_number,
                animal_id,
                animal_display_id,
                measurement_key,
                value,
                unit,
                measured_at,
            });
        }
        let summary = Self {
            import_kind: raw.import_kind,
            project_id: raw.project_id,
            experiment_id: raw.experiment_id,
            file_name: raw.file_name,
            sheet_name: raw.sheet_name,
            total_rows: raw.total_rows,
            accepted_rows: raw.accepted_rows,
            issue_count: raw.issue_count,
            issues_truncated: raw.issues_truncated,
            can_confirm: raw.can_confirm,
            preview_rows,
            preview_rows_truncated: raw.preview_rows_truncated,
            issues: raw.issues,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), ToolExecutionError> {
        let safe_text = |value: &str, max: usize| {
            value == value.trim()
                && !value.is_empty()
                && value.len() <= max
                && !value.chars().any(char::is_control)
        };
        let file_name_valid = safe_text(&self.file_name, 255)
            && !self.file_name.contains('/')
            && !self.file_name.contains('\\');
        let rows_valid = self.preview_rows.len() <= Self::MAX_PREVIEW_ROWS
            && self.preview_rows.len() <= self.accepted_rows
            && self.preview_rows.iter().all(|row| {
                row.row_number > 0
                    && !row.animal_id.is_nil()
                    && safe_text(&row.animal_display_id, 255)
                    && safe_text(&row.measurement_key, 255)
                    && !row.value.is_empty()
                    && row.value.len() <= 4096
                    && !row.value.chars().any(char::is_control)
                    && row.unit.as_deref().is_none_or(|unit| safe_text(unit, 128))
                    && safe_text(&row.measured_at, 64)
            });
        let issues_valid = self.issues.len() <= Self::MAX_PREVIEW_ISSUES
            && self.issues.len() <= self.issue_count
            && self.issues.iter().all(|issue| {
                issue
                    .field
                    .as_deref()
                    .is_none_or(|field| safe_text(field, 255))
                    && safe_text(&issue.code, 128)
                    && !issue.message.trim().is_empty()
                    && issue.message.len() <= 1024
                    && !issue.message.chars().any(char::is_control)
                    && issue.severity != ImportDraftIssueSeverity::Error
            });
        if self.import_kind != AiSourceImportKind::Measurement
            || self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || !file_name_valid
            || !safe_text(&self.sheet_name, 255)
            || self.accepted_rows > self.total_rows
            || !self.can_confirm
            || self.preview_rows_truncated != (self.accepted_rows > self.preview_rows.len())
            || self.issues_truncated != (self.issue_count > self.issues.len())
            || !rows_valid
            || !issues_valid
        {
            Err(rejected("invalid_import_preview"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportCreateArguments {
    pub project_id: Uuid,
    pub resource: AiExportResource,
    pub format: AiExportFormat,
}

/// Immutable binding persisted inside a BulkImport draft.
///
/// An application backend must deserialize this value with unknown fields
/// denied and then re-read every referenced object before applying it. The
/// model cannot supply paths, URLs, bytes, SQL, or an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCommitDraftPayload {
    pub operation: String,
    pub job_id: Uuid,
    pub preview_hash: String,
    pub expected_revision: i64,
    pub preview: ImportDraftPreviewSummary,
}

impl ImportCommitDraftPayload {
    pub const OPERATION: &'static str = "confirm_import";

    pub fn validate(&self) -> Result<(), ToolExecutionError> {
        if self.operation != Self::OPERATION
            || self.expected_revision < 1
            || !valid_sha256(&self.preview_hash)
        {
            Err(rejected("invalid_import_binding"))
        } else {
            self.preview.validate()
        }
    }
}

pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn rejected(code: &str) -> ToolExecutionError {
    ToolExecutionError::Rejected {
        code: code.to_owned(),
    }
}

/// Data-operation authority resolved from the live human/session permissions.
///
/// The model never constructs this value. External token scopes are applied by
/// the transport before the context reaches a backend, so they can only narrow
/// these project sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDataAccessContext {
    lab_id: Uuid,
    user_id: Uuid,
    importable_project_ids: BTreeSet<Uuid>,
    exportable_project_ids: BTreeSet<Uuid>,
    lab_import: bool,
    conversation_id: Option<Uuid>,
    conversation_project_id: Option<Uuid>,
}

impl AiDataAccessContext {
    pub fn none(lab_id: Uuid, user_id: Uuid) -> Self {
        Self {
            lab_id,
            user_id,
            importable_project_ids: BTreeSet::new(),
            exportable_project_ids: BTreeSet::new(),
            lab_import: false,
            conversation_id: None,
            conversation_project_id: None,
        }
    }

    pub fn new(
        lab_id: Uuid,
        user_id: Uuid,
        importable_project_ids: impl IntoIterator<Item = Uuid>,
        exportable_project_ids: impl IntoIterator<Item = Uuid>,
        lab_import: bool,
    ) -> Self {
        Self {
            lab_id,
            user_id,
            importable_project_ids: importable_project_ids.into_iter().collect(),
            exportable_project_ids: exportable_project_ids.into_iter().collect(),
            lab_import,
            conversation_id: None,
            conversation_project_id: None,
        }
    }

    /// Binds the backend access to the already-authorized conversation scope.
    ///
    /// This value is created by `AiWorkflowService` after resolving the
    /// conversation. It is never accepted from model arguments.
    pub const fn with_conversation(
        mut self,
        conversation_id: Uuid,
        project_id: Option<Uuid>,
    ) -> Self {
        self.conversation_id = Some(conversation_id);
        self.conversation_project_id = project_id;
        self
    }

    pub const fn lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub const fn user_id(&self) -> Uuid {
        self.user_id
    }

    pub const fn can_import_lab(&self) -> bool {
        self.lab_import
    }

    pub const fn conversation_id(&self) -> Option<Uuid> {
        self.conversation_id
    }

    pub const fn conversation_project_id(&self) -> Option<Uuid> {
        self.conversation_project_id
    }

    pub fn can_import_project(&self, project_id: Uuid) -> bool {
        self.importable_project_ids.contains(&project_id)
    }

    pub fn can_export_project(&self, project_id: Uuid) -> bool {
        self.exportable_project_ids.contains(&project_id)
    }

    pub fn can_import_anything(&self) -> bool {
        self.lab_import || !self.importable_project_ids.is_empty()
    }

    pub fn can_export_anything(&self) -> bool {
        !self.exportable_project_ids.is_empty()
    }

    pub fn importable_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.importable_project_ids
    }

    pub fn exportable_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.exportable_project_ids
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiDataApplyResult {
    pub job_id: Uuid,
    pub result: Value,
}

/// Application-layer bridge for bounded import/export operations.
///
/// Implementations live outside `muriarc-ai` so they can reuse the ordinary
/// DataFiles/Job workflows. If no backend is bound, none of these tools are
/// advertised and dispatch remains fail-closed.
#[async_trait]
pub trait AiDataToolBackend: Send + Sync {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName>;

    async fn execute(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError>;

    /// Applies an already human-approved, reinforced bulk-import draft.
    /// Implementations must revalidate scope, job revision, preview hash and
    /// expiry immediately before invoking the ordinary transactional confirm.
    async fn apply_import_draft(
        &self,
        access: &AiDataAccessContext,
        draft: &WriteDraft,
        resolution: &AiImportResolution,
        audit: &AuditContext,
    ) -> Result<AiDataApplyResult, ToolExecutionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_import_preview() -> ImportDraftPreviewSummary {
        ImportDraftPreviewSummary {
            import_kind: AiSourceImportKind::Measurement,
            project_id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            file_name: "measurements.csv".to_owned(),
            sheet_name: "Sheet1".to_owned(),
            total_rows: 1,
            accepted_rows: 1,
            issue_count: 0,
            issues_truncated: false,
            can_confirm: true,
            preview_rows: vec![ImportDraftPreviewRow {
                row_number: 2,
                animal_id: Uuid::new_v4(),
                animal_display_id: "M-001".to_owned(),
                measurement_key: "body_weight".to_owned(),
                value: "22.4".to_owned(),
                unit: Some("g".to_owned()),
                measured_at: "2026-07-23T08:00:00Z".to_owned(),
            }],
            preview_rows_truncated: false,
            issues: Vec::new(),
        }
    }

    #[test]
    fn access_context_is_fail_closed_and_project_bounded() {
        let lab_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let importable = Uuid::new_v4();
        let exportable = Uuid::new_v4();
        let none = AiDataAccessContext::none(lab_id, user_id);
        assert!(!none.can_import_anything());
        assert!(!none.can_export_anything());

        let access = AiDataAccessContext::new(lab_id, user_id, [importable], [exportable], true);
        assert!(access.can_import_lab());
        assert!(access.can_import_project(importable));
        assert!(!access.can_import_project(exportable));
        assert!(access.can_export_project(exportable));
        assert!(!access.can_export_project(importable));
    }

    #[test]
    fn persisted_import_binding_rejects_unknown_or_unsafe_shapes() {
        let valid = serde_json::json!({
            "operation": "confirm_import",
            "job_id": Uuid::new_v4(),
            "preview_hash": "a".repeat(64),
            "expected_revision": 2,
            "preview": valid_import_preview(),
        });
        let binding: ImportCommitDraftPayload = serde_json::from_value(valid).unwrap();
        binding.validate().unwrap();

        let unsafe_shape = serde_json::json!({
            "operation": "confirm_import",
            "job_id": Uuid::new_v4(),
            "preview_hash": "a".repeat(64),
            "expected_revision": 2,
            "preview": valid_import_preview(),
            "path": "C:/arbitrary.csv",
        });
        assert!(
            serde_json::from_value::<ImportCommitDraftPayload>(unsafe_shape).is_err(),
            "arbitrary paths must not enter an approval payload"
        );
        assert!(!valid_sha256("not-a-hash"));
    }

    #[test]
    fn trusted_public_preview_is_parsed_into_the_approval_projection() {
        let project_id = Uuid::new_v4();
        let experiment_id = Uuid::new_v4();
        let animal_id = Uuid::new_v4();
        let value = serde_json::json!({
            "job_id": Uuid::new_v4(),
            "job_revision": 2,
            "project_id": project_id,
            "import_kind": "measurement",
            "experiment_id": experiment_id,
            "file_name": "measurements.csv",
            "sheet_name": "Sheet1",
            "headers": ["display_id"],
            "mapping": {},
            "preview_hash": "a".repeat(64),
            "total_rows": 1,
            "accepted_rows": 1,
            "preview_rows": [{
                "row_number": "2",
                "animal_id": animal_id.to_string(),
                "animal_display_id": "M-001",
                "measurement_key": "body_weight",
                "value": "22.4",
                "unit": "g",
                "measured_at": "2026-07-23T08:00:00+00:00"
            }],
            "preview_rows_truncated": false,
            "can_confirm": true,
            "issue_count": 0,
            "issues": [],
            "issues_truncated": false,
            "expires_at": "2026-07-24T08:00:00Z"
        });

        let preview = ImportDraftPreviewSummary::from_public_preview(&value).unwrap();
        assert_eq!(preview.project_id, project_id);
        assert_eq!(preview.experiment_id, experiment_id);
        assert_eq!(preview.preview_rows[0].animal_id, animal_id);
        assert_eq!(preview.preview_rows[0].measurement_key, "body_weight");
        assert_eq!(preview.preview_rows[0].value, "22.4");
    }

    #[allow(dead_code)]
    fn trait_remains_object_safe(value: std::sync::Arc<dyn AiDataToolBackend>) {
        drop(value);
    }
}
