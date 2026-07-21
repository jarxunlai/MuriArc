use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{Attachment, Observation, ObservationValueRecord, RecordMeta};

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
            || self.image_sha256.len() != 64
            || self.provider.trim().is_empty()
            || self.model.trim().is_empty()
            || self.items.is_empty()
            || self.items.len() > 500
        {
            return Err("AI extraction draft metadata is invalid");
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
pub struct AppliedAiExtraction {
    pub draft: AiExtractionDraft,
    pub observations: Vec<Observation>,
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
        expected_revision: i64,
        selected_indexes: &[usize],
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<AppliedAiExtraction>;
    /// Records a workspace boundary operation that does not otherwise mutate a
    /// domain row (for example entering an administrator-only private view).
    async fn record_workspace_operation(
        &self,
        operation: WorkspaceOperationInput,
        audit: &crate::AuditContext,
    ) -> crate::StoreResult<()>;
}
