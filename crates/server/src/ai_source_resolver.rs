use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use muriarc_ai::{
    AssistantSourceBundle, AssistantSourceError, AssistantSourceResolutionRequest,
    AssistantSourceResolver, ResolvedAssistantSource, VisionImageInput,
};
use muriarc_core::{AiConversationSource, AiConversationSourceStatus, Attachment, MuriArcStore};
use muriarc_data::{
    AiSourceMaterial, AttachmentFiles, extract_ai_source_material, extract_ai_source_vision_assets,
};

/// Resolves opaque source IDs inside the trusted Server boundary.
///
/// Metadata scope is checked before the immutable attachment object is read
/// and verified. Paths, hashes and raw object metadata are never returned to
/// the assistant.
pub(crate) struct ServerAiSourceResolver {
    store: Arc<dyn MuriArcStore>,
    files: AttachmentFiles,
}

impl ServerAiSourceResolver {
    pub(crate) fn new(store: Arc<dyn MuriArcStore>, attachment_root: &Path) -> Self {
        Self {
            store,
            files: AttachmentFiles::new(attachment_root),
        }
    }
}

#[async_trait]
impl AssistantSourceResolver for ServerAiSourceResolver {
    async fn resolve(
        &self,
        request: AssistantSourceResolutionRequest,
    ) -> Result<AssistantSourceBundle, AssistantSourceError> {
        let mut resolved = Vec::with_capacity(request.source_ids.len());
        for source_id in &request.source_ids {
            let source = self
                .store
                .get_ai_conversation_source(*source_id)
                .await
                .map_err(|_| AssistantSourceError::Unavailable)?;
            ensure_source_scope(&source, &request)?;
            let attachment = self
                .store
                .get_attachment(source.attachment_id)
                .await
                .map_err(|_| AssistantSourceError::InvalidMaterial)?;
            ensure_attachment_scope(&source, &attachment)?;
            let bytes = self
                .files
                .read_verified_bytes(&attachment)
                .await
                .map_err(|_| AssistantSourceError::InvalidMaterial)?;
            resolved.push(resolve_material(source, attachment, &bytes)?);
        }
        AssistantSourceBundle::try_from_sources(resolved)
    }
}

fn ensure_source_scope(
    source: &AiConversationSource,
    request: &AssistantSourceResolutionRequest,
) -> Result<(), AssistantSourceError> {
    let available = match source.status {
        AiConversationSourceStatus::Ready => source.expires_at > Utc::now(),
        AiConversationSourceStatus::Archived => true,
        AiConversationSourceStatus::Staged
        | AiConversationSourceStatus::Failed
        | AiConversationSourceStatus::Expired => false,
    };
    if source.id.is_nil()
        || source.meta.deleted_at.is_some()
        || source.lab_id != request.lab_id
        || source.user_id != request.user_id
        || source.project_id != request.project_id
        || source.conversation_id != Some(request.conversation_id)
        || !available
    {
        Err(AssistantSourceError::Unavailable)
    } else {
        Ok(())
    }
}

fn ensure_attachment_scope(
    source: &AiConversationSource,
    attachment: &Attachment,
) -> Result<(), AssistantSourceError> {
    let expected_project = (source.status == AiConversationSourceStatus::Archived)
        .then_some(source.project_id)
        .flatten();
    if attachment.id != source.attachment_id
        || attachment.lab_id != source.lab_id
        || attachment.project_id != expected_project
        || attachment.entity_type != "ai_conversation_source"
        || attachment.entity_id != source.id
        || attachment.meta.deleted_at.is_some()
    {
        Err(AssistantSourceError::InvalidMaterial)
    } else {
        Ok(())
    }
}

fn resolve_material(
    source: AiConversationSource,
    attachment: Attachment,
    bytes: &[u8],
) -> Result<ResolvedAssistantSource, AssistantSourceError> {
    let media_type = attachment
        .media_type
        .clone()
        .ok_or(AssistantSourceError::InvalidMaterial)?;
    let material =
        extract_ai_source_material(source.kind, &attachment.file_name, Some(&media_type), bytes)
            .map_err(|_| AssistantSourceError::InvalidMaterial)?;
    let requires_vision = matches!(
        material,
        AiSourceMaterial::Image { .. }
            | AiSourceMaterial::ScannedPdf {
                requires_vision: true
            }
    );
    let images = if requires_vision {
        let assets = extract_ai_source_vision_assets(source.kind, Some(&media_type), bytes)
            .map_err(|_| AssistantSourceError::InvalidMaterial)?;
        if assets.is_empty() {
            return Err(AssistantSourceError::InvalidMaterial);
        }
        assets
            .into_iter()
            .map(|asset| VisionImageInput {
                media_type: asset.media_type,
                data_base64: STANDARD.encode(asset.bytes),
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(ResolvedAssistantSource {
        source_id: source.id,
        source_revision: source.meta.revision,
        attachment_id: attachment.id,
        file_name: attachment.file_name,
        media_type,
        size_bytes: attachment.size_bytes,
        material: serde_json::to_value(material)
            .map_err(|_| AssistantSourceError::InvalidMaterial)?,
        images,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use muriarc_core::{AiConversationSourceKind, Attachment, RecordMeta};
    use uuid::Uuid;

    use super::*;

    fn source(now: chrono::DateTime<Utc>) -> AiConversationSource {
        AiConversationSource {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            conversation_id: None,
            project_id: None,
            attachment_id: Uuid::new_v4(),
            kind: AiConversationSourceKind::Text,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + Duration::hours(1),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        }
    }

    #[test]
    fn owner_project_and_conversation_scope_is_fail_closed() {
        let now = Utc::now();
        let mut source = source(now);
        let request = AssistantSourceResolutionRequest {
            lab_id: source.lab_id,
            user_id: source.user_id,
            conversation_id: Uuid::new_v4(),
            project_id: None,
            source_ids: vec![source.id],
        };
        assert_eq!(
            ensure_source_scope(&source, &request),
            Err(AssistantSourceError::Unavailable)
        );

        source.conversation_id = Some(request.conversation_id);
        assert_eq!(ensure_source_scope(&source, &request), Ok(()));

        source.conversation_id = Some(Uuid::new_v4());
        assert_eq!(
            ensure_source_scope(&source, &request),
            Err(AssistantSourceError::Unavailable)
        );
        source.conversation_id = Some(request.conversation_id);
        source.user_id = Uuid::new_v4();
        assert_eq!(
            ensure_source_scope(&source, &request),
            Err(AssistantSourceError::Unavailable)
        );
    }

    #[test]
    fn trusted_snapshot_keeps_object_secrets_out_of_model_context() {
        let now = Utc::now();
        let mut source = source(now);
        source.conversation_id = Some(Uuid::new_v4());
        let bytes = b"animal M-001 needs review";
        let attachment = Attachment {
            id: source.attachment_id,
            lab_id: source.lab_id,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source.id,
            file_name: "notes.md".to_owned(),
            media_type: Some("text/markdown".to_owned()),
            relative_path: "private/secret/object".to_owned(),
            size_bytes: i64::try_from(bytes.len()).unwrap(),
            sha256: "do-not-expose-this-hash".to_owned(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        let resolved = resolve_material(source.clone(), attachment.clone(), bytes).unwrap();
        let bundle = AssistantSourceBundle::try_from_sources(vec![resolved]).unwrap();

        assert_eq!(bundle.source_refs()[0].source_id, source.id);
        assert_eq!(bundle.source_refs()[0].attachment_id, source.attachment_id);
        assert_eq!(bundle.source_refs()[0].size_bytes, attachment.size_bytes);
        assert!(!bundle.context().contains(&attachment.relative_path));
        assert!(!bundle.context().contains(&attachment.sha256));
        assert!(!bundle.context().contains(&attachment.id.to_string()));
    }
}
