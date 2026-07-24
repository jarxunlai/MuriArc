use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    AiModelPurpose, Attachment, Observation, ObservationSubjectType, ObservationValueData,
    ObservationValueRecord, RecordMeta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLinkTarget {
    Project,
    Experiment,
    Animal,
    Worksheet,
    CollectionNode,
    DataCell,
}

impl AttachmentLinkTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Experiment => "experiment",
            Self::Animal => "animal",
            Self::Worksheet => "worksheet",
            Self::CollectionNode => "collection_node",
            Self::DataCell => "data_cell",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentLink {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub attachment_id: Uuid,
    pub target_type: AttachmentLinkTarget,
    pub target_id: Uuid,
    pub created_by: Uuid,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeKind {
    Thumbnail,
    Preview,
    Ocr,
    AiInput,
}

impl DerivativeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumbnail",
            Self::Preview => "preview",
            Self::Ocr => "ocr",
            Self::AiInput => "ai_input",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeStatus {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentDerivative {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub attachment_id: Uuid,
    pub kind: DerivativeKind,
    pub media_type: Option<String>,
    pub relative_path: Option<String>,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub status: DerivativeStatus,
    pub error_code: Option<String>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateImageStatus {
    Active,
    Processing,
    PendingApproval,
    Archived,
    Failed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateAiImage {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub conversation_id: Option<Uuid>,
    pub attachment_id: Uuid,
    pub project_id: Option<Uuid>,
    pub status: PrivateImageStatus,
    pub last_activity_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateImageFilter {
    pub lab_id: Uuid,
    pub user_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub status: Option<PrivateImageStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateImageStats {
    pub user_id: Uuid,
    pub image_count: i64,
    pub total_size_bytes: i64,
    pub expiring_count: i64,
    pub failed_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiExtractionStatus {
    Draft,
    PendingApproval,
    Approved,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiObservationDataCell {
    pub definition_id: Uuid,
    pub subject_type: ObservationSubjectType,
    pub subject_id: Uuid,
}

impl AiObservationDataCell {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.definition_id.is_nil() || self.subject_id.is_nil() {
            Err("extraction data cell identifiers must not be nil")
        } else {
            Ok(())
        }
    }

    pub fn matches(&self, observation: &Observation) -> bool {
        self.definition_id == observation.definition_id
            && self.subject_type == observation.subject_type
            && self.subject_id == observation.subject_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiExtractionEvidence {
    pub display_order: i32,
    pub private_image_id: Uuid,
    pub private_attachment_id: Uuid,
    pub promoted_attachment_id: Option<Uuid>,
    pub original_sha256: String,
    pub sanitized_sha256: String,
    pub meta: RecordMeta,
}

impl AiExtractionEvidence {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.display_order < 0
            || self.private_image_id.is_nil()
            || self.private_attachment_id.is_nil()
            || self
                .promoted_attachment_id
                .is_some_and(|value| value.is_nil())
            || self
                .promoted_attachment_id
                .is_some_and(|value| value != self.private_attachment_id)
            || !is_sha256(&self.original_sha256)
            || !is_sha256(&self.sanitized_sha256)
            || self.meta.deleted_at.is_some()
            || self.meta.revision < 1
        {
            Err("AI extraction evidence is invalid")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiExtractionModelTrace {
    pub profile_id: Uuid,
    pub profile_version: i64,
    pub purpose: AiModelPurpose,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub provider_request_id: Option<String>,
    pub trace: Value,
}

impl AiExtractionModelTrace {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.profile_id.is_nil()
            || self.profile_version < 1
            || self.purpose != AiModelPurpose::Vision
            || self.total_tokens < self.input_tokens.saturating_add(self.output_tokens)
            || self
                .provider_request_id
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 256)
            || !self.trace.is_object()
            || serde_json::to_vec(&self.trace).map_or(true, |encoded| encoded.len() > 16 * 1024)
        {
            Err("AI extraction model trace is invalid")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiExtractionItem {
    pub observation: Observation,
    pub value: ObservationValueRecord,
    pub confidence: f64,
    pub selected: bool,
    pub source_label: Option<String>,
}

impl AiExtractionItem {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err("extraction confidence must be between zero and one");
        }
        if self.source_label.as_ref().is_some_and(|label| {
            label.trim().is_empty() || label.len() > 512 || label.chars().any(char::is_control)
        }) {
            return Err("extraction source label is invalid");
        }
        self.observation
            .validate()
            .map_err(|_| "extraction observation is invalid")?;
        self.value
            .validate()
            .map_err(|_| "extraction value is invalid")?;
        if self.value.observation_id != self.observation.id || self.value.version != 1 {
            return Err("extraction value must be the first value of its observation");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiExtractionDraft {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub experiment_event_id: Uuid,
    pub private_image_id: Uuid,
    pub attachment_id: Uuid,
    pub image_sha256: String,
    pub provider: String,
    pub model: String,
    pub tool_run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_cell: Option<AiObservationDataCell>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<AiExtractionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_trace: Option<AiExtractionModelTrace>,
    pub status: AiExtractionStatus,
    pub items: Vec<AiExtractionItem>,
    pub error_code: Option<String>,
    pub meta: RecordMeta,
}

impl AiExtractionDraft {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.user_id.is_nil()
            || self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || self.experiment_event_id.is_nil()
            || self.private_image_id.is_nil()
            || self.attachment_id.is_nil()
            || !is_sha256(&self.image_sha256)
            || self.provider.trim().is_empty()
            || self.provider.len() > 128
            || self.model.trim().is_empty()
            || self.model.len() > 256
            || self.items.is_empty()
            || self.items.len() > 500
        {
            return Err("AI extraction draft metadata is invalid");
        }
        let uses_versioned_evidence =
            self.data_cell.is_some() || !self.evidence.is_empty() || self.model_trace.is_some();
        if uses_versioned_evidence {
            let cell = self
                .data_cell
                .as_ref()
                .ok_or("AI extraction data cell is required")?;
            let trace = self
                .model_trace
                .as_ref()
                .ok_or("AI extraction model trace is required")?;
            cell.validate()?;
            trace.validate()?;
            if self.evidence.is_empty() || self.evidence.len() > 8 {
                return Err("AI extraction evidence count must be between one and eight");
            }
            let mut private_images = std::collections::BTreeSet::new();
            let mut private_attachments = std::collections::BTreeSet::new();
            for (index, evidence) in self.evidence.iter().enumerate() {
                evidence.validate()?;
                if evidence.display_order != index as i32
                    || !private_images.insert(evidence.private_image_id)
                    || !private_attachments.insert(evidence.private_attachment_id)
                {
                    return Err("AI extraction evidence order and identifiers must be unique");
                }
            }
            let first = &self.evidence[0];
            if (
                self.private_image_id,
                self.attachment_id,
                &self.image_sha256,
            ) != (
                first.private_image_id,
                first.private_attachment_id,
                &first.original_sha256,
            ) {
                return Err("legacy extraction image fields must match first evidence");
            }
            if self.items.len() > 20 {
                return Err("AI extraction candidate count is too large");
            }
            for item in &self.items {
                if !cell.matches(&item.observation) {
                    return Err("AI extraction candidate changed its bound data cell");
                }
            }
        }
        for item in &self.items {
            item.validate()?;
            if item.observation.lab_id != self.lab_id
                || item.observation.project_id != self.project_id
                || item.observation.experiment_id != self.experiment_id
                || item.observation.experiment_event_id != self.experiment_event_id
            {
                return Err("AI extraction item scope is invalid");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiExtractionApprovalSelection {
    pub item_index: usize,
    pub value: ObservationValueData,
    pub notes: Option<String>,
}

impl AiExtractionApprovalSelection {
    pub fn validate(&self) -> Result<(), &'static str> {
        if matches!(self.value, ObservationValueData::Number(value) if !value.is_finite())
            || self.notes.as_ref().is_some_and(|value| value.len() > 4_000)
        {
            Err("AI extraction approval edit is invalid")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiExtractionApprovalInput {
    pub expected_revision: i64,
    pub selections: Vec<AiExtractionApprovalSelection>,
}

impl AiExtractionApprovalInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.expected_revision < 1 || self.selections.is_empty() || self.selections.len() > 20 {
            return Err("AI extraction approval metadata is invalid");
        }
        let mut indexes = std::collections::BTreeSet::new();
        for selection in &self.selections {
            selection.validate()?;
            if !indexes.insert(selection.item_index) {
                return Err("AI extraction approval indexes must be unique");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiExtractionRejectionInput {
    pub expected_revision: i64,
}

impl AiExtractionRejectionInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.expected_revision < 1 {
            Err("AI extraction rejection revision is invalid")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedAiExtraction {
    pub draft: AiExtractionDraft,
    pub observations: Vec<Observation>,
    pub attachments: Vec<Attachment>,
    pub links: Vec<AttachmentLink>,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalAttachmentView {
    pub attachment: Attachment,
    pub links: Vec<AttachmentLink>,
    pub derivatives: Vec<AttachmentDerivative>,
    pub preview_supported: bool,
    pub preview_href: Option<String>,
    pub preview_reason: Option<String>,
    pub status: String,
    pub retention_until: Option<DateTime<Utc>>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceOperationInput {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: crate::EntityType,
    pub entity_id: Uuid,
    pub action: crate::AuditAction,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[async_trait::async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn create_attachment_link(
        &self,
        link: &AttachmentLink,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
    async fn list_attachment_links(
        &self,
        attachment_id: Uuid,
    ) -> crate::StoreResult<Vec<AttachmentLink>>;
    async fn create_attachment_derivative(
        &self,
        derivative: &AttachmentDerivative,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
    async fn list_attachment_derivatives(
        &self,
        attachment_id: Uuid,
    ) -> crate::StoreResult<Vec<AttachmentDerivative>>;
    /// Lists the canonical attachment rows belonging to a project, regardless of
    /// which project-scoped entity originally received the upload.
    async fn list_project_attachments(
        &self,
        lab_id: Uuid,
        project_id: Uuid,
    ) -> crate::StoreResult<Vec<Attachment>>;
    async fn create_private_ai_image(
        &self,
        attachment: &Attachment,
        image: &PrivateAiImage,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
    async fn get_private_ai_image(&self, id: Uuid) -> crate::StoreResult<PrivateAiImage>;
    async fn list_private_ai_images(
        &self,
        filter: &PrivateImageFilter,
    ) -> crate::StoreResult<Vec<PrivateAiImage>>;
    async fn archive_private_ai_image(
        &self,
        id: Uuid,
        project_id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<PrivateAiImage>;
    async fn private_ai_image_stats(
        &self,
        lab_id: Uuid,
        now: DateTime<Utc>,
    ) -> crate::StoreResult<Vec<PrivateImageStats>>;
    async fn create_ai_extraction_draft(
        &self,
        draft: &AiExtractionDraft,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
    async fn get_ai_extraction_draft(&self, id: Uuid) -> crate::StoreResult<AiExtractionDraft>;
    async fn list_ai_extraction_drafts(
        &self,
        lab_id: Uuid,
        user_id: Uuid,
        project_id: Option<Uuid>,
    ) -> crate::StoreResult<Vec<AiExtractionDraft>>;
    async fn apply_ai_extraction_draft(
        &self,
        id: Uuid,
        approval: &AiExtractionApprovalInput,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<AppliedAiExtraction>;
    async fn reject_ai_extraction_draft(
        &self,
        id: Uuid,
        rejection: &AiExtractionRejectionInput,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<AiExtractionDraft>;
    /// Records a workspace boundary operation that does not otherwise mutate a
    /// domain row (for example entering an administrator-only private view).
    async fn record_workspace_operation(
        &self,
        operation: WorkspaceOperationInput,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
}
