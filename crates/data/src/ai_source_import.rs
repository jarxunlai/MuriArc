use std::path::Path;

use chrono::{DateTime, Utc};
use muriarc_core::{
    AiConversationSource, AiConversationSourceKind, AiConversationSourceStatus, Attachment,
};
use thiserror::Error;
use uuid::Uuid;

use crate::ImportKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAiSourceImport {
    pub file_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AiSourceImportValidationError {
    #[error("AI source is unavailable in the current conversation")]
    SourceUnavailable,
    #[error("AI source attachment metadata is invalid")]
    InvalidAttachment,
    #[error("AI source is not a supported ordinary import file")]
    UnsupportedFile,
    #[error("AI source does not match the requested import scope")]
    ScopeMismatch,
}

/// Revalidates an AI source before it can enter the ordinary import workflow.
///
/// The caller must first read `bytes` through `AttachmentFiles::read_verified_bytes`.
/// This function intentionally accepts no path, URL, caller-provided bytes, or
/// idempotency key from the model.
#[allow(clippy::too_many_arguments)]
pub fn validate_ai_source_import(
    source: &AiConversationSource,
    attachment: &Attachment,
    bytes: &[u8],
    lab_id: Uuid,
    user_id: Uuid,
    conversation_id: Uuid,
    conversation_project_id: Option<Uuid>,
    import_kind: ImportKind,
    now: DateTime<Utc>,
) -> Result<ValidatedAiSourceImport, AiSourceImportValidationError> {
    let available = match source.status {
        AiConversationSourceStatus::Ready => source.expires_at > now,
        AiConversationSourceStatus::Archived => true,
        AiConversationSourceStatus::Staged
        | AiConversationSourceStatus::Failed
        | AiConversationSourceStatus::Expired => false,
    };
    if source.id.is_nil()
        || source.meta.deleted_at.is_some()
        || source.lab_id != lab_id
        || source.user_id != user_id
        || source.project_id != conversation_project_id
        || source.conversation_id != Some(conversation_id)
        || !available
    {
        return Err(AiSourceImportValidationError::SourceUnavailable);
    }

    let expected_attachment_project = (source.status == AiConversationSourceStatus::Archived)
        .then_some(source.project_id)
        .flatten();
    if attachment.id != source.attachment_id
        || attachment.lab_id != source.lab_id
        || attachment.project_id != expected_attachment_project
        || attachment.entity_type != "ai_conversation_source"
        || attachment.entity_id != source.id
        || attachment.meta.deleted_at.is_some()
        || attachment.size_bytes < 1
        || attachment.size_bytes as usize != bytes.len()
        || attachment.sha256.len() != 64
    {
        return Err(AiSourceImportValidationError::InvalidAttachment);
    }

    match import_kind {
        ImportKind::Animal if conversation_project_id.is_some() => {
            return Err(AiSourceImportValidationError::ScopeMismatch);
        }
        ImportKind::Measurement if conversation_project_id.is_none() => {
            return Err(AiSourceImportValidationError::ScopeMismatch);
        }
        ImportKind::Animal | ImportKind::Measurement => {}
    }

    let file_name = attachment.file_name.trim();
    if file_name.is_empty()
        || file_name != attachment.file_name
        || file_name.len() > 255
        || file_name.contains(['/', '\\'])
        || Path::new(file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(file_name)
    {
        return Err(AiSourceImportValidationError::UnsupportedFile);
    }
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(AiSourceImportValidationError::UnsupportedFile)?;
    let media_type = attachment
        .media_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or(AiSourceImportValidationError::UnsupportedFile)?;

    let supported = match extension.as_str() {
        "csv" => {
            source.kind == AiConversationSourceKind::DelimitedText
                && matches!(
                    media_type.as_str(),
                    "text/csv" | "application/csv" | "text/plain"
                )
                && !bytes.contains(&0)
                && !has_zip_magic(bytes)
        }
        "xlsx" => {
            source.kind == AiConversationSourceKind::Spreadsheet
                && matches!(
                    media_type.as_str(),
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                        | "application/zip"
                )
                && has_zip_magic(bytes)
        }
        _ => false,
    };
    if !supported {
        return Err(AiSourceImportValidationError::UnsupportedFile);
    }

    Ok(ValidatedAiSourceImport {
        file_name: file_name.to_owned(),
    })
}

pub fn ai_source_import_idempotency_key(
    source_id: Uuid,
    import_kind: ImportKind,
    experiment_id: Option<Uuid>,
) -> String {
    let kind = match import_kind {
        ImportKind::Animal => "animal".to_owned(),
        ImportKind::Measurement => format!(
            "measurement:{}",
            experiment_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "missing".to_owned())
        ),
    };
    format!("ai-source-import:{source_id}:{kind}")
}

fn has_zip_magic(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([b'P', b'K', 3, 4]) | Some([b'P', b'K', 5, 6]) | Some([b'P', b'K', 7, 8])
    )
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use muriarc_core::{AiConversationSourceKind, RecordMeta};

    use super::*;

    fn source_and_attachment(
        project_id: Option<Uuid>,
        conversation_id: Uuid,
        now: DateTime<Utc>,
    ) -> (AiConversationSource, Attachment) {
        let source_id = Uuid::new_v4();
        let attachment_id = Uuid::new_v4();
        let source = AiConversationSource {
            id: source_id,
            lab_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            conversation_id: Some(conversation_id),
            project_id,
            attachment_id,
            kind: AiConversationSourceKind::DelimitedText,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + Duration::hours(1),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        };
        let bytes = b"display_id,sex\nM-1,female\n";
        let attachment = Attachment {
            id: attachment_id,
            lab_id: source.lab_id,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source_id,
            file_name: "animals.csv".to_owned(),
            media_type: Some("text/csv".to_owned()),
            relative_path: "objects/opaque".to_owned(),
            size_bytes: bytes.len() as i64,
            sha256: "a".repeat(64),
            version: 1,
            meta: RecordMeta::new(now),
        };
        (source, attachment)
    }

    #[test]
    fn source_import_is_conversation_and_project_bound() {
        let now = Utc::now();
        let conversation_id = Uuid::new_v4();
        let (source, attachment) = source_and_attachment(None, conversation_id, now);
        let bytes = b"display_id,sex\nM-1,female\n";

        validate_ai_source_import(
            &source,
            &attachment,
            bytes,
            source.lab_id,
            source.user_id,
            conversation_id,
            None,
            ImportKind::Animal,
            now,
        )
        .unwrap();

        let mut unbound = source.clone();
        unbound.conversation_id = None;
        assert_eq!(
            validate_ai_source_import(
                &unbound,
                &attachment,
                bytes,
                source.lab_id,
                source.user_id,
                conversation_id,
                None,
                ImportKind::Animal,
                now,
            ),
            Err(AiSourceImportValidationError::SourceUnavailable)
        );
        assert_eq!(
            validate_ai_source_import(
                &source,
                &attachment,
                bytes,
                source.lab_id,
                source.user_id,
                Uuid::new_v4(),
                None,
                ImportKind::Animal,
                now,
            ),
            Err(AiSourceImportValidationError::SourceUnavailable)
        );
        assert_eq!(
            validate_ai_source_import(
                &source,
                &attachment,
                bytes,
                source.lab_id,
                source.user_id,
                conversation_id,
                Some(Uuid::new_v4()),
                ImportKind::Measurement,
                now,
            ),
            Err(AiSourceImportValidationError::SourceUnavailable)
        );
    }

    #[test]
    fn only_csv_or_xlsx_material_is_accepted() {
        let now = Utc::now();
        let conversation_id = Uuid::new_v4();
        let (mut source, mut attachment) = source_and_attachment(None, conversation_id, now);
        let bytes = b"display_id\nM-1\n";
        attachment.file_name = "notes.txt".to_owned();
        attachment.media_type = Some("text/plain".to_owned());
        source.kind = AiConversationSourceKind::Text;
        attachment.size_bytes = bytes.len() as i64;

        assert_eq!(
            validate_ai_source_import(
                &source,
                &attachment,
                bytes,
                source.lab_id,
                source.user_id,
                conversation_id,
                None,
                ImportKind::Animal,
                now,
            ),
            Err(AiSourceImportValidationError::UnsupportedFile)
        );
    }
}
