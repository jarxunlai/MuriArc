use std::path::Path;

use chrono::{Duration, Utc};
use muriarc_core::{
    Actor, AiConversationSource, AiConversationSourceFilter, AiConversationSourceKind,
    AiConversationSourceStatus, AiOperationStore, Attachment, AuditContext, LOCAL_LAB_ID,
    LOCAL_USER_ID, MuriArcStore, RecordMeta, StoreError, WorkspaceStore, WriteSource,
};
use muriarc_data::{
    AttachmentContentKind, AttachmentInspection, DEFAULT_MAX_UPLOAD_BYTES,
    extract_ai_source_material, inspect_attachment,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::{DesktopDataError, DesktopDataState};

const RETENTION_DAYS: i64 = 30;

impl DesktopDataState {
    pub(crate) async fn upload_ai_source(
        &self,
        input: UploadAiSourceInput,
    ) -> Result<AiSourceView, DesktopDataError> {
        self.cleanup_expired_ai_sources_best_effort("upload_ai_source")
            .await;
        let conversation_id = parse_source_id("conversation", &input.conversation_id)?;
        let requested_project_id = input
            .project_id
            .as_deref()
            .map(|value| parse_source_id("project", value))
            .transpose()?;
        let project_id = self
            .source_context(Some(conversation_id), requested_project_id, true)
            .await?;
        let file_name = valid_source_file_name(input.file_name)?;
        let declared_media_type = valid_source_media_type(input.media_type)?;
        if input.bytes.len() as u64 > DEFAULT_MAX_UPLOAD_BYTES {
            return Err(muriarc_data::AttachmentFileError::TooLarge.into());
        }

        let attachment_id = Uuid::new_v4();
        let object = self
            .attachments_ref()
            .write_bytes(attachment_id, &input.bytes)
            .await?;
        let inspected = match inspect_attachment(
            &object.absolute_path,
            &file_name,
            declared_media_type.as_deref(),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                let _ = self
                    .attachments_ref()
                    .remove_installed_object(&object)
                    .await;
                return Err(error.into());
            }
        };
        let (kind, media_type) = match classify_source(
            &input.bytes,
            &file_name,
            declared_media_type.as_deref(),
            inspected,
        ) {
            Ok(value) => value,
            Err(error) => {
                let _ = self
                    .attachments_ref()
                    .remove_installed_object(&object)
                    .await;
                return Err(error);
            }
        };
        if let Err(error) =
            extract_ai_source_material(kind, &file_name, Some(&media_type), &input.bytes)
        {
            let _ = self
                .attachments_ref()
                .remove_installed_object(&object)
                .await;
            return Err(StoreError::Validation(error.to_string()).into());
        }

        let now = Utc::now();
        let source_id = Uuid::new_v4();
        let attachment = Attachment {
            id: attachment_id,
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source_id,
            file_name,
            media_type: Some(media_type),
            relative_path: object.relative_path.clone(),
            size_bytes: object.size_bytes,
            sha256: object.sha256.clone(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        let source = AiConversationSource {
            id: source_id,
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            conversation_id: Some(conversation_id),
            project_id,
            attachment_id,
            kind,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + Duration::days(RETENTION_DAYS),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        };
        let audit = self.source_audit("upload_ai_source").await?;
        if let Err(error) = self
            .store_ref()
            .create_ai_conversation_source(&attachment, &source, &audit)
            .await
        {
            if let Err(cleanup_error) = self
                .attachments_ref()
                .remove_installed_object(&object)
                .await
            {
                eprintln!(
                    "MuriArc rejected AI source object cleanup failed: attachment_id={attachment_id}, error={cleanup_error}"
                );
            }
            return Err(error.into());
        }
        Ok(AiSourceView::new(source, attachment))
    }

    pub(crate) async fn list_ai_sources(
        &self,
        input: ListAiSourcesInput,
    ) -> Result<Vec<AiSourceView>, DesktopDataError> {
        self.cleanup_expired_ai_sources_best_effort("list_ai_sources")
            .await;
        let conversation_id = parse_source_id("conversation", &input.conversation_id)?;
        let requested_project_id = input
            .project_id
            .as_deref()
            .map(|value| parse_source_id("project", value))
            .transpose()?;
        let project_id = self
            .source_context(Some(conversation_id), requested_project_id, false)
            .await?;
        let mut sources = self
            .store_ref()
            .list_ai_conversation_sources(&AiConversationSourceFilter {
                lab_id: LOCAL_LAB_ID,
                user_id: LOCAL_USER_ID,
                conversation_id: Some(conversation_id),
                project_id,
                status: input.status,
                unconsumed_only: true,
            })
            .await?;
        // `None` in the shared store filter means “no predicate”; a source
        // listing still represents one exact nullable project scope.
        sources.retain(|source| {
            source.project_id == project_id && source.conversation_id == Some(conversation_id)
        });
        let mut views = Vec::with_capacity(sources.len());
        for source in sources {
            let attachment = self
                .store_ref()
                .get_attachment(source.attachment_id)
                .await?;
            views.push(AiSourceView::new(source, attachment));
        }
        Ok(views)
    }

    pub(crate) async fn archive_ai_source(
        &self,
        source_id: &str,
        input: ArchiveAiSourceInput,
    ) -> Result<AiSourceView, DesktopDataError> {
        let id = parse_source_id("AI source", source_id)?;
        let project_id = parse_source_id("project", &input.project_id)?;
        let source = self.owned_source(id).await?;
        let conversation_id = source
            .conversation_id
            .ok_or(DesktopDataError::ScopeMismatch)?;
        if self
            .source_context(Some(conversation_id), Some(project_id), true)
            .await?
            != Some(project_id)
        {
            return Err(DesktopDataError::ScopeMismatch);
        }
        if source
            .project_id
            .is_some_and(|bound_project| bound_project != project_id)
        {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let project = self.store_ref().get_project(project_id).await?;
        ensure_local_source_lab(project.lab_id)?;
        let audit = self.source_audit("archive_ai_source").await?;
        let source = self
            .store_ref()
            .archive_ai_conversation_source(
                id,
                project_id,
                input.expected_revision,
                Utc::now(),
                &audit,
            )
            .await?;
        let attachment = self
            .store_ref()
            .get_attachment(source.attachment_id)
            .await?;
        Ok(AiSourceView::new(source, attachment))
    }

    pub(crate) async fn delete_ai_source(&self, source_id: &str) -> Result<(), DesktopDataError> {
        let id = parse_source_id("AI source", source_id)?;
        let source = match self.owned_source(id).await {
            Ok(source) => source,
            Err(DesktopDataError::Store(StoreError::NotFound { .. })) => return Ok(()),
            Err(error) => return Err(error),
        };
        let attachment = self
            .store_ref()
            .get_attachment(source.attachment_id)
            .await?;
        let audit = self.source_audit("delete_ai_source").await?;
        self.store_ref()
            .discard_ai_conversation_source(id, source.meta.revision, Utc::now(), &audit)
            .await?;
        match self
            .attachments_ref()
            .remove_verified_object(&attachment)
            .await
        {
            Ok(()) => {
                if let Err(error) = self
                    .store_ref()
                    .complete_ai_conversation_source_object_deletion(
                        id,
                        attachment.id,
                        Utc::now(),
                        &audit,
                    )
                    .await
                {
                    eprintln!(
                        "MuriArc AI source cleanup completion remains queued: source_id={id}, attachment_id={}, error={error}",
                        attachment.id
                    );
                }
            }
            Err(error) => {
                eprintln!(
                    "MuriArc AI source object cleanup remains queued: source_id={id}, attachment_id={}, error={error}",
                    attachment.id
                );
            }
        }
        Ok(())
    }

    async fn source_context(
        &self,
        conversation_id: Option<Uuid>,
        requested_project_id: Option<Uuid>,
        require_writable: bool,
    ) -> Result<Option<Uuid>, DesktopDataError> {
        let conversation_project = if let Some(conversation_id) = conversation_id {
            let conversation = self
                .store_ref()
                .get_ai_conversation(conversation_id)
                .await?;
            if conversation.lab_id != LOCAL_LAB_ID || conversation.user_id != LOCAL_USER_ID {
                return Err(DesktopDataError::ScopeMismatch);
            }
            if require_writable && conversation.archived_at.is_some() {
                return Err(StoreError::Conflict("AI conversation is archived".to_owned()).into());
            }
            conversation.project_id
        } else {
            None
        };
        if conversation_id.is_some()
            && requested_project_id.is_some()
            && conversation_project != requested_project_id
        {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let project_id = conversation_project.or(requested_project_id);
        if let Some(project_id) = project_id {
            let project = self.store_ref().get_project(project_id).await?;
            ensure_local_source_lab(project.lab_id)?;
        }
        Ok(project_id)
    }

    async fn owned_source(&self, id: Uuid) -> Result<AiConversationSource, DesktopDataError> {
        let source = self.store_ref().get_ai_conversation_source(id).await?;
        if source.lab_id != LOCAL_LAB_ID || source.user_id != LOCAL_USER_ID {
            return Err(DesktopDataError::ScopeMismatch);
        }
        Ok(source)
    }

    async fn source_audit(&self, reason: &'static str) -> Result<AuditContext, DesktopDataError> {
        let operator = self.store_ref().get_user(LOCAL_USER_ID).await?;
        Ok(AuditContext {
            actor: Actor::human(LOCAL_USER_ID, operator.display_name),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some(reason.to_owned()),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UploadAiSourceInput {
    pub file_name: String,
    pub media_type: Option<String>,
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListAiSourcesInput {
    pub conversation_id: String,
    pub project_id: Option<String>,
    pub status: Option<AiConversationSourceStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveAiSourceInput {
    pub project_id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiSourceView {
    pub id: String,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: AiConversationSourceKind,
    pub status: AiConversationSourceStatus,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub revision: i64,
    pub created_at: String,
    pub expires_at: String,
}

impl AiSourceView {
    fn new(source: AiConversationSource, attachment: Attachment) -> Self {
        Self {
            id: source.id.to_string(),
            conversation_id: source.conversation_id.map(|value| value.to_string()),
            project_id: source.project_id.map(|value| value.to_string()),
            kind: source.kind,
            status: source.status,
            file_name: attachment.file_name,
            media_type: attachment
                .media_type
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            size_bytes: attachment.size_bytes,
            revision: source.meta.revision,
            created_at: source.meta.created_at.to_rfc3339(),
            expires_at: source.expires_at.to_rfc3339(),
        }
    }
}

fn classify_source(
    bytes: &[u8],
    file_name: &str,
    declared_media_type: Option<&str>,
    inspection: AttachmentInspection,
) -> Result<(AiConversationSourceKind, String), DesktopDataError> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(unsupported_source)?;
    let declared = declared_media_type
        .map(|value| value.split(';').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty());
    let (kind, canonical_media_type, allowed_media_types): (
        AiConversationSourceKind,
        &'static str,
        &'static [&'static str],
    ) = match extension.as_str() {
        "xlsx"
            if inspection.kind == AttachmentContentKind::Opaque
                && matches!(
                    bytes.get(..4),
                    Some([b'P', b'K', 3, 4]) | Some([b'P', b'K', 5, 6]) | Some([b'P', b'K', 7, 8])
                ) =>
        {
            (
                AiConversationSourceKind::Spreadsheet,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &[
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    "application/zip",
                ],
            )
        }
        "csv" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/csv",
            &["text/csv", "application/csv", "text/plain"],
        ),
        "tsv" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::DelimitedText,
            "text/tab-separated-values",
            &["text/tab-separated-values", "text/plain"],
        ),
        "txt" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "text/plain",
            &["text/plain"],
        ),
        "md" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "text/markdown",
            &["text/markdown", "text/plain"],
        ),
        "json" if inspection.kind == AttachmentContentKind::Opaque => (
            AiConversationSourceKind::Text,
            "application/json",
            &["application/json", "text/json", "text/plain"],
        ),
        "pdf" if inspection.kind == AttachmentContentKind::Pdf => (
            AiConversationSourceKind::Pdf,
            "application/pdf",
            &["application/pdf"],
        ),
        "png" if inspection.kind == AttachmentContentKind::Png => {
            (AiConversationSourceKind::Image, "image/png", &["image/png"])
        }
        "jpg" | "jpeg" if inspection.kind == AttachmentContentKind::Jpeg => (
            AiConversationSourceKind::Image,
            "image/jpeg",
            &["image/jpeg"],
        ),
        "tif" | "tiff" if inspection.kind == AttachmentContentKind::Tiff => (
            AiConversationSourceKind::Image,
            "image/tiff",
            &["image/tiff"],
        ),
        _ => return Err(unsupported_source()),
    };
    if declared.is_some_and(|value| {
        value != "application/octet-stream" && !allowed_media_types.contains(&value)
    }) {
        return Err(StoreError::Validation(
            "the declared media type does not match the AI source file extension".to_owned(),
        )
        .into());
    }
    Ok((
        kind,
        inspection
            .media_type
            .filter(|value| value != "application/octet-stream")
            .unwrap_or_else(|| canonical_media_type.to_owned()),
    ))
}

fn valid_source_file_name(value: String) -> Result<String, DesktopDataError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > 255
        || matches!(value.as_str(), "." | "..")
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(StoreError::Validation("AI source file name is invalid".to_owned()).into());
    }
    Ok(value)
}

fn valid_source_media_type(value: Option<String>) -> Result<Option<String>, DesktopDataError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| {
        value.len() > 127 || !value.is_ascii() || value.chars().any(char::is_control)
    }) {
        return Err(StoreError::Validation("AI source media type is invalid".to_owned()).into());
    }
    Ok(value)
}

fn unsupported_source() -> DesktopDataError {
    StoreError::Validation(
        "AI sources accept XLSX, CSV, TSV, TXT, MD, JSON, PDF, PNG, JPEG, and TIFF only".to_owned(),
    )
    .into()
}

fn parse_source_id(field: &'static str, value: &str) -> Result<Uuid, DesktopDataError> {
    Uuid::parse_str(value).map_err(|_| DesktopDataError::InvalidId(field))
}

fn ensure_local_source_lab(lab_id: Uuid) -> Result<(), DesktopDataError> {
    if lab_id == LOCAL_LAB_ID {
        Ok(())
    } else {
        Err(DesktopDataError::ScopeMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::DesktopState;
    use muriarc_core::{AiConversation, Project, ProvenanceFilter};
    use tempfile::tempdir;

    #[test]
    fn native_source_transport_requires_a_conversation_for_upload_and_list() {
        assert!(
            serde_json::from_value::<UploadAiSourceInput>(serde_json::json!({
                "fileName": "unbound.csv",
                "mediaType": "text/csv",
                "bytes": [97, 10]
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<ListAiSourcesInput>(serde_json::json!({})).is_err());
    }

    #[tokio::test]
    async fn listing_opportunistically_retires_expired_sources() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let now = Utc::now();
        let audit = state
            .source_audit("opportunistic_cleanup_fixture")
            .await
            .unwrap();
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            user_id: LOCAL_USER_ID,
            title: "Expired source cleanup".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        state
            .store_ref()
            .create_ai_conversation(&conversation, &audit)
            .await
            .unwrap();
        let attachment_id = Uuid::new_v4();
        let object = state
            .attachments_ref()
            .write_bytes(attachment_id, b"expired source")
            .await
            .unwrap();
        let source_id = Uuid::new_v4();
        let attachment = Attachment {
            id: attachment_id,
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source_id,
            file_name: "expired.txt".to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: object.relative_path,
            size_bytes: object.size_bytes,
            sha256: object.sha256,
            version: 1,
            meta: RecordMeta::new(now - Duration::days(2)),
        };
        let source = AiConversationSource {
            id: source_id,
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            conversation_id: Some(conversation.id),
            project_id: None,
            attachment_id,
            kind: AiConversationSourceKind::Text,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now - Duration::days(2),
            expires_at: now - Duration::days(1),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now - Duration::days(2)),
        };
        state
            .store_ref()
            .create_ai_conversation_source(&attachment, &source, &audit)
            .await
            .unwrap();

        let listed = state
            .list_ai_sources(ListAiSourcesInput {
                conversation_id: conversation.id.to_string(),
                project_id: None,
                status: None,
            })
            .await
            .unwrap();
        assert!(listed.is_empty());
        assert!(!object.absolute_path.exists());
        assert!(
            state
                .store_ref()
                .list_pending_ai_conversation_source_object_deletions(LOCAL_LAB_ID, 100)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn startup_cleanup_failure_is_logged_without_blocking_desktop_initialization() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let store = muriarc_store_sqlite::SqliteStore::connect_path(&database)
            .await
            .unwrap();
        store.migrate().await.unwrap();
        sqlx::query("DROP TABLE ai_conversation_source_object_deletions")
            .execute(store.pool())
            .await
            .unwrap();
        drop(store);

        let initialized = DesktopDataState::initialize(&database, temp.path()).await;
        assert!(
            initialized.is_ok(),
            "retention maintenance failure must not prevent Desktop startup"
        );
    }

    #[tokio::test]
    async fn local_sources_are_scoped_archived_and_discarded() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let now = Utc::now();
        let project = Project::new(LOCAL_LAB_ID, "AI source project", now).unwrap();
        let audit = state.source_audit("source_test_fixture").await.unwrap();
        state
            .store_ref()
            .create_project(&project, &audit)
            .await
            .unwrap();
        let project_conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: Some(project.id),
            user_id: LOCAL_USER_ID,
            title: "Project source conversation".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let other_project_conversation = AiConversation {
            id: Uuid::new_v4(),
            title: "Other project source conversation".to_owned(),
            ..project_conversation.clone()
        };
        let lab_conversation = AiConversation {
            id: Uuid::new_v4(),
            project_id: None,
            title: "Lab source conversation".to_owned(),
            ..project_conversation.clone()
        };
        for conversation in [
            &project_conversation,
            &other_project_conversation,
            &lab_conversation,
        ] {
            state
                .store_ref()
                .create_ai_conversation(conversation, &audit)
                .await
                .unwrap();
        }

        let uploaded = state
            .upload_ai_source(UploadAiSourceInput {
                file_name: "notes.md".to_owned(),
                media_type: Some("text/markdown".to_owned()),
                conversation_id: project_conversation.id.to_string(),
                project_id: Some(project.id.to_string()),
                bytes: b"# source".to_vec(),
            })
            .await
            .unwrap();
        let uploaded_json = serde_json::to_value(&uploaded).unwrap();
        assert!(uploaded_json.get("attachmentId").is_none());
        assert!(uploaded_json.get("sha256").is_none());
        assert!(uploaded_json.get("relativePath").is_none());
        assert_eq!(uploaded.status, AiConversationSourceStatus::Ready);
        assert_eq!(uploaded.kind, AiConversationSourceKind::Text);
        assert_eq!(
            state
                .list_ai_sources(ListAiSourcesInput {
                    conversation_id: project_conversation.id.to_string(),
                    project_id: Some(project.id.to_string()),
                    status: None,
                })
                .await
                .unwrap()
                .len(),
            1
        );
        let other_conversation_source = state
            .upload_ai_source(UploadAiSourceInput {
                file_name: "other-notes.md".to_owned(),
                media_type: Some("text/markdown".to_owned()),
                conversation_id: other_project_conversation.id.to_string(),
                project_id: Some(project.id.to_string()),
                bytes: b"# other source".to_vec(),
            })
            .await
            .unwrap();
        assert_eq!(
            state
                .list_ai_sources(ListAiSourcesInput {
                    conversation_id: project_conversation.id.to_string(),
                    project_id: Some(project.id.to_string()),
                    status: None,
                })
                .await
                .unwrap()
                .len(),
            1,
            "sources from another conversation must not be enumerable"
        );
        state
            .delete_ai_source(&other_conversation_source.id)
            .await
            .unwrap();
        let archived = state
            .archive_ai_source(
                &uploaded.id,
                ArchiveAiSourceInput {
                    project_id: project.id.to_string(),
                    expected_revision: uploaded.revision,
                },
            )
            .await
            .unwrap();
        assert_eq!(archived.status, AiConversationSourceStatus::Archived);
        assert_eq!(archived.project_id, Some(project.id.to_string()));
        assert!(matches!(
            state.delete_ai_source(&archived.id).await,
            Err(DesktopDataError::Store(StoreError::Conflict(_)))
        ));

        let second = state
            .upload_ai_source(UploadAiSourceInput {
                file_name: "table.csv".to_owned(),
                media_type: Some("text/csv".to_owned()),
                conversation_id: lab_conversation.id.to_string(),
                project_id: None,
                bytes: b"animal,value\nM-1,12\n".to_vec(),
            })
            .await
            .unwrap();
        assert!(matches!(
            state
                .archive_ai_source(
                    &second.id,
                    ArchiveAiSourceInput {
                        project_id: project.id.to_string(),
                        expected_revision: second.revision,
                    },
                )
                .await,
            Err(DesktopDataError::ScopeMismatch)
        ));
        let second_id = Uuid::parse_str(&second.id).unwrap();
        let second_source = state
            .store_ref()
            .get_ai_conversation_source(second_id)
            .await
            .unwrap();
        let second_attachment = state
            .store_ref()
            .get_attachment(second_source.attachment_id)
            .await
            .unwrap();
        let second_object_path = state
            .attachments_ref()
            .root()
            .join(&second_attachment.relative_path);
        assert!(second_object_path.exists());
        state.delete_ai_source(&second.id).await.unwrap();
        assert!(
            !second_object_path.exists(),
            "manual source discard must also remove the verified immutable object"
        );
        assert!(
            state
                .store_ref()
                .list_pending_ai_conversation_source_object_deletions(LOCAL_LAB_ID, 100)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            state
                .store_ref()
                .get_ai_conversation_source(Uuid::parse_str(&second.id).unwrap())
                .await,
            Err(StoreError::NotFound { .. })
        ));
        assert!(
            !state
                .store_ref()
                .list_provenance(&ProvenanceFilter {
                    lab_id: LOCAL_LAB_ID,
                    entity_type: Some(muriarc_core::EntityType::AiConversationSource),
                    entity_id: Some(Uuid::parse_str(&second.id).unwrap()),
                    ..ProvenanceFilter::default()
                })
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn conversation_scope_is_inferred_and_cannot_be_widened_by_the_renderer() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let now = Utc::now();
        let project = Project::new(LOCAL_LAB_ID, "Scoped source project", now).unwrap();
        let audit = state.source_audit("source_scope_fixture").await.unwrap();
        state
            .store_ref()
            .create_project(&project, &audit)
            .await
            .unwrap();

        let project_conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: Some(project.id),
            user_id: LOCAL_USER_ID,
            title: "Project conversation".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        state
            .store_ref()
            .create_ai_conversation(&project_conversation, &audit)
            .await
            .unwrap();
        let inferred = state
            .upload_ai_source(UploadAiSourceInput {
                file_name: "project-notes.md".to_owned(),
                media_type: Some("text/markdown".to_owned()),
                conversation_id: project_conversation.id.to_string(),
                project_id: None,
                bytes: b"project scoped".to_vec(),
            })
            .await
            .unwrap();
        assert_eq!(inferred.project_id, Some(project.id.to_string()));

        let lab_conversation = AiConversation {
            id: Uuid::new_v4(),
            project_id: None,
            title: "Lab conversation".to_owned(),
            ..project_conversation.clone()
        };
        state
            .store_ref()
            .create_ai_conversation(&lab_conversation, &audit)
            .await
            .unwrap();
        assert!(matches!(
            state
                .upload_ai_source(UploadAiSourceInput {
                    file_name: "widened.md".to_owned(),
                    media_type: Some("text/markdown".to_owned()),
                    conversation_id: lab_conversation.id.to_string(),
                    project_id: Some(project.id.to_string()),
                    bytes: b"must be rejected".to_vec(),
                })
                .await,
            Err(DesktopDataError::ScopeMismatch)
        ));

        let archived_conversation = AiConversation {
            id: Uuid::new_v4(),
            title: "Archived conversation".to_owned(),
            archived_at: Some(now),
            ..project_conversation
        };
        state
            .store_ref()
            .create_ai_conversation(&archived_conversation, &audit)
            .await
            .unwrap();
        assert!(matches!(
            state
                .upload_ai_source(UploadAiSourceInput {
                    file_name: "archived.md".to_owned(),
                    media_type: Some("text/markdown".to_owned()),
                    conversation_id: archived_conversation.id.to_string(),
                    project_id: Some(project.id.to_string()),
                    bytes: b"must unarchive first".to_vec(),
                })
                .await,
            Err(DesktopDataError::Store(StoreError::Conflict(_)))
        ));
    }

    #[tokio::test]
    async fn rejected_source_cleans_installed_object_and_metadata() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let before = state
            .store_ref()
            .list_lab_attachments(LOCAL_LAB_ID)
            .await
            .unwrap()
            .len();
        let now = Utc::now();
        let audit = state.source_audit("rejected_source_fixture").await.unwrap();
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            user_id: LOCAL_USER_ID,
            title: "Rejected source conversation".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        state
            .store_ref()
            .create_ai_conversation(&conversation, &audit)
            .await
            .unwrap();
        let rejected = state
            .upload_ai_source(UploadAiSourceInput {
                file_name: "source.docx".to_owned(),
                media_type: Some("application/octet-stream".to_owned()),
                conversation_id: conversation.id.to_string(),
                project_id: None,
                bytes: b"not accepted".to_vec(),
            })
            .await;
        assert!(rejected.is_err());
        assert_eq!(
            state
                .store_ref()
                .list_lab_attachments(LOCAL_LAB_ID)
                .await
                .unwrap()
                .len(),
            before
        );
        let object_root = state.attachments_ref().root().join("objects");
        let mut entries = tokio::fs::read_dir(object_root).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }
}
