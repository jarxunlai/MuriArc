use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use muriarc_ai::{
    AssistantSourceBundle, AssistantSourceError, AssistantSourceResolutionRequest,
    AssistantSourceResolver, ResolvedAssistantSource, VisionImageInput,
};
use muriarc_core::{
    AiConversationSource, AiConversationSourceStatus, Attachment, MuriArcStore, WorkspaceStore,
};
use muriarc_data::{AiSourceMaterial, extract_ai_source_material, extract_ai_source_vision_assets};

use crate::data::DesktopDataState;

/// Desktop equivalent of the Server source resolver. Keeping it behind the
/// native state prevents the webview from injecting source content or paths.
pub(crate) struct DesktopAiSourceResolver {
    data: DesktopDataState,
}

impl DesktopAiSourceResolver {
    pub(crate) fn new(data: DesktopDataState) -> Self {
        Self { data }
    }
}

#[async_trait]
impl AssistantSourceResolver for DesktopAiSourceResolver {
    async fn resolve(
        &self,
        request: AssistantSourceResolutionRequest,
    ) -> Result<AssistantSourceBundle, AssistantSourceError> {
        let mut resolved = Vec::with_capacity(request.source_ids.len());
        for source_id in &request.source_ids {
            let source = self
                .data
                .store_ref()
                .get_ai_conversation_source(*source_id)
                .await
                .map_err(|_| AssistantSourceError::Unavailable)?;
            ensure_source_scope(&source, &request)?;
            let attachment = self
                .data
                .store_ref()
                .get_attachment(source.attachment_id)
                .await
                .map_err(|_| AssistantSourceError::InvalidMaterial)?;
            ensure_attachment_scope(&source, &attachment)?;
            let bytes = self
                .data
                .attachments_ref()
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

    #[test]
    fn native_resolver_requires_one_exact_conversation_binding() {
        let now = Utc::now();
        let conversation_id = Uuid::new_v4();
        let mut source = AiConversationSource {
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
        };
        let request = AssistantSourceResolutionRequest {
            lab_id: source.lab_id,
            user_id: source.user_id,
            conversation_id,
            project_id: None,
            source_ids: vec![source.id],
        };

        assert_eq!(
            ensure_source_scope(&source, &request),
            Err(AssistantSourceError::Unavailable)
        );
        source.conversation_id = Some(conversation_id);
        assert_eq!(ensure_source_scope(&source, &request), Ok(()));
        source.conversation_id = Some(Uuid::new_v4());
        assert_eq!(
            ensure_source_scope(&source, &request),
            Err(AssistantSourceError::Unavailable)
        );
    }

    #[test]
    fn native_snapshot_matches_server_safe_metadata_contract() {
        let now = Utc::now();
        let source = AiConversationSource {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            conversation_id: Some(Uuid::new_v4()),
            project_id: None,
            attachment_id: Uuid::new_v4(),
            kind: AiConversationSourceKind::Text,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + Duration::hours(1),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        };
        let bytes = b"native source";
        let attachment = Attachment {
            id: source.attachment_id,
            lab_id: source.lab_id,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source.id,
            file_name: "native.txt".to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: "native/private/object".to_owned(),
            size_bytes: i64::try_from(bytes.len()).unwrap(),
            sha256: "native-secret-hash".to_owned(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        let resolved = resolve_material(source.clone(), attachment.clone(), bytes).unwrap();
        let bundle = AssistantSourceBundle::try_from_sources(vec![resolved]).unwrap();

        assert_eq!(bundle.source_refs()[0].source_revision, 1);
        assert_eq!(bundle.source_refs()[0].attachment_id, source.attachment_id);
        assert!(!bundle.context().contains(&attachment.relative_path));
        assert!(!bundle.context().contains(&attachment.sha256));
        assert!(!bundle.context().contains(&attachment.id.to_string()));
    }
}
