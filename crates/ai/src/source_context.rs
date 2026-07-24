use std::collections::BTreeSet;

use async_trait::async_trait;
use muriarc_core::AiConversationSourceRef;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MAX_VISION_IMAGE_BASE64_BYTES, MAX_VISION_IMAGES, MAX_VISION_TOTAL_BASE64_BYTES,
    VisionImageInput,
};

pub const MAX_ASSISTANT_SOURCES: usize = 10;
pub const MAX_ASSISTANT_SOURCE_CONTEXT_BYTES: usize = 160 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAssistantSource {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub attachment_id: Uuid,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub material: Value,
    pub images: Vec<VisionImageInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssistantSourceBundle {
    source_ids: Vec<Uuid>,
    source_refs: Vec<AiConversationSourceRef>,
    context: String,
    images: Vec<VisionImageInput>,
}

impl AssistantSourceBundle {
    pub fn empty() -> Self {
        Self {
            source_ids: Vec::new(),
            source_refs: Vec::new(),
            context: String::new(),
            images: Vec::new(),
        }
    }

    pub fn try_from_sources(
        sources: Vec<ResolvedAssistantSource>,
    ) -> Result<Self, AssistantSourceError> {
        if sources.len() > MAX_ASSISTANT_SOURCES {
            return Err(AssistantSourceError::TooManySources);
        }
        let mut seen = BTreeSet::new();
        let mut source_ids = Vec::with_capacity(sources.len());
        let mut source_refs = Vec::with_capacity(sources.len());
        let mut images = Vec::new();
        let mut records = Vec::with_capacity(sources.len());
        for source in sources {
            if source.source_id.is_nil()
                || !seen.insert(source.source_id)
                || source.file_name.trim().is_empty()
                || source.file_name.len() > 255
                || source.file_name.chars().any(char::is_control)
                || source.media_type.trim().is_empty()
                || source.media_type.len() > 127
                || !source.media_type.is_ascii()
                || source.media_type.chars().any(char::is_control)
            {
                return Err(AssistantSourceError::InvalidSource);
            }
            let source_ref = AiConversationSourceRef {
                source_id: source.source_id,
                source_revision: source.source_revision,
                attachment_id: source.attachment_id,
                file_name: source.file_name.clone(),
                media_type: Some(source.media_type.clone()),
                size_bytes: source.size_bytes,
            };
            source_ref
                .validate()
                .map_err(|_| AssistantSourceError::InvalidSource)?;
            source_ids.push(source.source_id);
            source_refs.push(source_ref);
            images.extend(source.images);
            records.push(json!({
                "source_id": source.source_id,
                "file_name": source.file_name,
                "media_type": source.media_type,
                "material": source.material,
            }));
        }
        if images.len() > MAX_VISION_IMAGES {
            return Err(AssistantSourceError::TooManyVisionImages);
        }
        let mut total_image_bytes = 0_usize;
        for image in &images {
            if !matches!(
                image.media_type.as_str(),
                "image/jpeg" | "image/png" | "image/tiff"
            ) || image.data_base64.is_empty()
                || image.data_base64.len() > MAX_VISION_IMAGE_BASE64_BYTES
            {
                return Err(AssistantSourceError::InvalidVisionImage);
            }
            total_image_bytes = total_image_bytes
                .checked_add(image.data_base64.len())
                .ok_or(AssistantSourceError::VisionPayloadTooLarge)?;
        }
        if total_image_bytes > MAX_VISION_TOTAL_BASE64_BYTES {
            return Err(AssistantSourceError::VisionPayloadTooLarge);
        }
        let context = if records.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&json!({
                "kind": "muriarc_user_sources",
                "sources": records,
            }))
            .map_err(|_| AssistantSourceError::InvalidSource)?
        };
        if context.len() > MAX_ASSISTANT_SOURCE_CONTEXT_BYTES {
            return Err(AssistantSourceError::ContextTooLarge);
        }
        Ok(Self {
            source_ids,
            source_refs,
            context,
            images,
        })
    }

    pub fn source_ids(&self) -> &[Uuid] {
        &self.source_ids
    }

    pub fn source_refs(&self) -> &[AiConversationSourceRef] {
        &self.source_refs
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn images(&self) -> &[VisionImageInput] {
        &self.images
    }

    pub fn is_empty(&self) -> bool {
        self.source_ids.is_empty()
    }
}

/// Trusted, transport-resolved source selection for one assistant turn.
///
/// The client supplies only opaque IDs. Implementations must re-read the
/// source/attachment records, enforce owner and project/conversation scope,
/// verify the immutable object, and return only bounded inert material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantSourceResolutionRequest {
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub conversation_id: Uuid,
    pub project_id: Option<Uuid>,
    pub source_ids: Vec<Uuid>,
}

#[async_trait]
pub trait AssistantSourceResolver: Send + Sync {
    async fn resolve(
        &self,
        request: AssistantSourceResolutionRequest,
    ) -> Result<AssistantSourceBundle, AssistantSourceError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AssistantSourceError {
    #[error("too many AI sources were selected for one turn")]
    TooManySources,
    #[error("an AI source is invalid or duplicated")]
    InvalidSource,
    #[error("selected AI source material exceeds the bounded context size")]
    ContextTooLarge,
    #[error("selected AI sources contain too many vision images")]
    TooManyVisionImages,
    #[error("a selected AI source contains an invalid vision image")]
    InvalidVisionImage,
    #[error("selected AI source images exceed the bounded vision payload")]
    VisionPayloadTooLarge,
    #[error("selected AI sources were not resolved by the trusted transport")]
    ResolutionRequired,
    #[error("an AI source is unavailable, expired, or outside the conversation scope")]
    Unavailable,
    #[error("an AI source object could not be verified or parsed")]
    InvalidMaterial,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u128, text: &str) -> ResolvedAssistantSource {
        ResolvedAssistantSource {
            source_id: Uuid::from_u128(id),
            source_revision: 1,
            attachment_id: Uuid::from_u128(id + 100),
            file_name: format!("source-{id}.txt"),
            media_type: "text/plain".to_owned(),
            size_bytes: i64::try_from(text.len()).unwrap(),
            material: json!({"kind": "text", "text": text}),
            images: Vec::new(),
        }
    }

    #[test]
    fn bundle_is_bounded_and_preserves_opaque_source_ids() {
        let bundle =
            AssistantSourceBundle::try_from_sources(vec![source(1, "animal M-1")]).unwrap();
        assert_eq!(bundle.source_ids(), [Uuid::from_u128(1)]);
        assert!(bundle.context().contains("animal M-1"));
        assert!(!bundle.is_empty());
    }

    #[test]
    fn duplicate_sources_and_oversized_context_are_rejected() {
        assert_eq!(
            AssistantSourceBundle::try_from_sources(vec![source(1, "a"), source(1, "b")]),
            Err(AssistantSourceError::InvalidSource)
        );
        assert_eq!(
            AssistantSourceBundle::try_from_sources(vec![source(
                1,
                &"a".repeat(MAX_ASSISTANT_SOURCE_CONTEXT_BYTES)
            )]),
            Err(AssistantSourceError::ContextTooLarge)
        );
    }
}
