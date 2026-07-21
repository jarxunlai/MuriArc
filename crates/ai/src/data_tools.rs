use std::collections::BTreeSet;

use async_trait::async_trait;
use muriarc_core::AuditContext;
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
            Ok(())
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
}

impl AiDataAccessContext {
    pub fn none(lab_id: Uuid, user_id: Uuid) -> Self {
        Self {
            lab_id,
            user_id,
            importable_project_ids: BTreeSet::new(),
            exportable_project_ids: BTreeSet::new(),
            lab_import: false,
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
        }
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
        audit: &AuditContext,
    ) -> Result<AiDataApplyResult, ToolExecutionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
        });
        let binding: ImportCommitDraftPayload = serde_json::from_value(valid).unwrap();
        binding.validate().unwrap();

        let unsafe_shape = serde_json::json!({
            "operation": "confirm_import",
            "job_id": Uuid::new_v4(),
            "preview_hash": "a".repeat(64),
            "expected_revision": 2,
            "path": "C:/arbitrary.csv",
        });
        assert!(
            serde_json::from_value::<ImportCommitDraftPayload>(unsafe_shape).is_err(),
            "arbitrary paths must not enter an approval payload"
        );
        assert!(!valid_sha256("not-a-hash"));
    }

    #[allow(dead_code)]
    fn trait_remains_object_safe(value: std::sync::Arc<dyn AiDataToolBackend>) {
        drop(value);
    }
}
