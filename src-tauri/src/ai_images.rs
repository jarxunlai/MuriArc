use std::{collections::BTreeSet, sync::Arc};

use chrono::{Duration, Utc};
use muriarc_ai::{
    AiExecutionContext, AiWorkflowService, DataCellVisionExtractionError,
    DataCellVisionExtractionRequest, MAX_SANITIZED_VISION_INPUT_BYTES, MAX_VISION_IMAGES,
    MAX_VISION_TOTAL_BASE64_BYTES, PreparedAssistantImage, ProviderCredentials,
    extract_data_cell_vision, sanitize_vision_input,
};
use muriarc_core::{
    Actor, AiExtractionApprovalInput, AiExtractionApprovalSelection, AiExtractionDraft,
    AiExtractionEvidence, AiExtractionItem, AiExtractionModelTrace, AiExtractionRejectionInput,
    AiExtractionStatus, AiModelProfileBinding, AiModelProfileStore, AiModelPurpose,
    AiObservationDataCell, AppliedAiExtraction, Attachment, AttachmentDerivative, AuditContext,
    DerivativeKind, DerivativeStatus, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, Observation,
    ObservationSubjectType, ObservationValueData, ObservationValueRecord, ParticipationFilter,
    PrivateAiImage, PrivateImageFilter, PrivateImageStatus, RecordMeta, StoreError, WorkspaceStore,
    WriteSource,
};
use muriarc_data::{
    AttachmentContentKind, AttachmentFileError, AttachmentFiles, inspect_attachment,
};
use muriarc_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    ai::DesktopAiError,
    settings::{ResolvedAiProvider, SettingsService},
};

const PRIVATE_IMAGE_RETENTION_DAYS: i64 = 30;

#[derive(Clone)]
pub(crate) struct DesktopAiImages {
    store: Arc<SqliteStore>,
    files: AttachmentFiles,
    settings: SettingsService,
}

impl DesktopAiImages {
    pub(crate) fn new(
        store: Arc<SqliteStore>,
        files: AttachmentFiles,
        settings: SettingsService,
    ) -> Self {
        Self {
            store,
            files,
            settings,
        }
    }

    pub(crate) async fn upload(
        &self,
        workflow: &AiWorkflowService,
        context: &AiExecutionContext,
        input: UploadPrivateAiImageInput,
    ) -> Result<PrivateImageView, DesktopAiError> {
        validate_private_image_upload_size(input.bytes.len())?;
        let file_name = validate_file_name(&input.file_name)?;
        if let Some(conversation_id) = input.conversation_id {
            workflow
                .conversation_model_profile(context, conversation_id)
                .await?;
        }
        let attachment_id = Uuid::new_v4();
        let object = self.files.write_bytes(attachment_id, &input.bytes).await?;
        let inspection = match inspect_attachment(
            &object.absolute_path,
            &file_name,
            input.media_type.as_deref(),
        )
        .await
        {
            Ok(inspection)
                if matches!(
                    inspection.kind,
                    AttachmentContentKind::Jpeg
                        | AttachmentContentKind::Png
                        | AttachmentContentKind::Webp
                        | AttachmentContentKind::Gif
                ) =>
            {
                inspection
            }
            Ok(_) | Err(_) => {
                self.files.remove_installed_object(&object).await.ok();
                return Err(DesktopAiError::InvalidImageEvidence);
            }
        };
        let now = Utc::now();
        let image_id = Uuid::new_v4();
        let attachment = Attachment {
            id: attachment_id,
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_type: "ai_private_image".to_owned(),
            entity_id: image_id,
            file_name,
            media_type: inspection.media_type,
            relative_path: object.relative_path.clone(),
            size_bytes: object.size_bytes,
            sha256: object.sha256.clone(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        let image = PrivateAiImage {
            id: image_id,
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            conversation_id: input.conversation_id,
            attachment_id,
            project_id: None,
            status: PrivateImageStatus::Active,
            last_activity_at: now,
            expires_at: now + Duration::days(PRIVATE_IMAGE_RETENTION_DAYS),
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        let audit = desktop_audit(context, "upload_private_ai_image");
        if let Err(error) = self
            .store
            .create_private_ai_image(&attachment, &image, &audit)
            .await
        {
            self.files.remove_installed_object(&object).await.ok();
            return Err(error.into());
        }
        Ok(PrivateImageView::new(image, attachment))
    }

    pub(crate) async fn list(
        &self,
        conversation_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> Result<Vec<PrivateImageView>, DesktopAiError> {
        let images = self
            .store
            .list_private_ai_images(&PrivateImageFilter {
                lab_id: LOCAL_LAB_ID,
                user_id: Some(LOCAL_USER_ID),
                conversation_id,
                project_id,
                status: None,
            })
            .await?;
        let mut views = Vec::with_capacity(images.len());
        for image in images {
            let attachment = self.store.get_attachment(image.attachment_id).await?;
            ensure_private_image_scope(&image, &attachment)?;
            views.push(PrivateImageView::new(image, attachment));
        }
        Ok(views)
    }

    pub(crate) async fn read(&self, id: Uuid) -> Result<PrivateImageContent, DesktopAiError> {
        let (image, attachment) = self.private_image(id).await?;
        if !private_image_is_readable(image.status) {
            return Err(DesktopAiError::InvalidImageEvidence);
        }
        let bytes = self.files.read_verified_bytes(&attachment).await?;
        Ok(PrivateImageContent {
            media_type: attachment.media_type,
            bytes,
        })
    }

    pub(crate) async fn archive(
        &self,
        workflow: &AiWorkflowService,
        context: &AiExecutionContext,
        id: Uuid,
        input: ArchivePrivateAiImageInput,
    ) -> Result<PrivateImageView, DesktopAiError> {
        let (image, _) = self.private_image(id).await?;
        if let Some(conversation_id) = image.conversation_id {
            workflow
                .conversation_model_profile(context, conversation_id)
                .await?;
        }
        if image.status == PrivateImageStatus::PendingApproval {
            return Err(DesktopAiError::InvalidImageEvidence);
        }
        let updated = self
            .store
            .archive_private_ai_image(
                id,
                input.project_id,
                input.expected_revision,
                Utc::now(),
                &desktop_audit(context, "archive_private_ai_image"),
            )
            .await?;
        let attachment = self.store.get_attachment(updated.attachment_id).await?;
        Ok(PrivateImageView::new(updated, attachment))
    }

    pub(crate) async fn prepare_assistant_images(
        &self,
        context: &AiExecutionContext,
        conversation_id: Option<Uuid>,
        project_id: Option<Uuid>,
        image_ids: &[Uuid],
    ) -> Result<Vec<PreparedAssistantImage>, DesktopAiError> {
        validate_image_ids(image_ids)?;
        let mut inputs = Vec::with_capacity(image_ids.len());
        let mut projected_base64_bytes = 0;
        for image_id in image_ids {
            let (image, attachment) = self.private_image(*image_id).await?;
            if image.status != PrivateImageStatus::Active
                || image
                    .conversation_id
                    .is_some_and(|bound| Some(bound) != conversation_id)
                || image
                    .project_id
                    .is_some_and(|bound| Some(bound) != project_id)
            {
                return Err(DesktopAiError::InvalidImageEvidence);
            }
            let media_type = attachment
                .media_type
                .as_deref()
                .filter(|value| {
                    matches!(
                        *value,
                        "image/jpeg" | "image/png" | "image/webp" | "image/gif"
                    )
                })
                .ok_or(DesktopAiError::InvalidImageEvidence)?
                .to_owned();
            if attachment.size_bytes <= 0
                || usize::try_from(attachment.size_bytes)
                    .ok()
                    .is_none_or(|size| size > MAX_SANITIZED_VISION_INPUT_BYTES)
            {
                return Err(DesktopAiError::InvalidImageEvidence);
            }
            let attachment_size = usize::try_from(attachment.size_bytes)
                .map_err(|_| DesktopAiError::InvalidImageEvidence)?;
            projected_base64_bytes = accumulate_vision_base64_bytes(
                projected_base64_bytes,
                base64_encoded_len(attachment_size).ok_or(DesktopAiError::InvalidImageEvidence)?,
            )?;
            inputs.push((image, attachment, media_type));
        }

        let mut prepared = Vec::with_capacity(inputs.len());
        let mut actual_base64_bytes = 0;
        for (image, attachment, media_type) in inputs {
            let provider_image = self
                .prepare_ai_input(context, &image, &attachment, &media_type)
                .await?;
            actual_base64_bytes = accumulate_vision_base64_bytes(
                actual_base64_bytes,
                provider_image.provider_input().data_base64.len(),
            )?;
            prepared.push(provider_image);
        }
        Ok(prepared)
    }

    pub(crate) async fn resolve_vision_provider(
        &self,
        requested_profile_id: Option<Uuid>,
    ) -> Result<(AiModelProfileBinding, ResolvedAiProvider), DesktopAiError> {
        let explicitly_selected = requested_profile_id.is_some();
        let _profile_operation = self.settings.profile_coordinator().lock().await;
        let profile_id = match requested_profile_id {
            Some(profile_id) if !profile_id.is_nil() => profile_id,
            Some(_) => return Err(DesktopAiError::VisionModelUnavailable),
            None => self
                .store
                .get_ai_user_model_defaults(LOCAL_USER_ID)
                .await?
                .and_then(|defaults| defaults.default_vision_profile_id)
                .ok_or(DesktopAiError::VisionModelSelectionRequired)?,
        };
        let profile = match self.store.get_ai_model_profile(profile_id).await {
            Ok(profile) => profile,
            Err(StoreError::NotFound { .. }) if explicitly_selected => {
                return Err(DesktopAiError::VisionModelUnavailable);
            }
            Err(StoreError::NotFound { .. }) => {
                return Err(DesktopAiError::VisionModelSelectionRequired);
            }
            Err(error) => return Err(error.into()),
        };
        if profile.lab_id != LOCAL_LAB_ID
            || profile.user_id != LOCAL_USER_ID
            || profile.archived_at.is_some()
            || profile.meta.deleted_at.is_some()
        {
            return Err(if explicitly_selected {
                DesktopAiError::VisionModelUnavailable
            } else {
                DesktopAiError::VisionModelSelectionRequired
            });
        }
        let binding = AiModelProfileBinding {
            profile_id,
            profile_version: profile.current_version,
        };
        let resolved = match self
            .settings
            .resolve_provider_for_profile(self.store.as_ref(), binding)
            .await
        {
            Ok(resolved) if resolved.supports_vision => resolved,
            Ok(_) => {
                return Err(if explicitly_selected {
                    DesktopAiError::VisionModelUnavailable
                } else {
                    DesktopAiError::VisionModelSelectionRequired
                });
            }
            Err(error)
                if matches!(
                    &error,
                    crate::settings::SettingsError::Storage
                        | crate::settings::SettingsError::CredentialStore
                        | crate::settings::SettingsError::ModelProfileStore(
                            StoreError::Database(_) | StoreError::Serialization(_)
                        )
                ) =>
            {
                return Err(DesktopAiError::Settings(error));
            }
            Err(_) if explicitly_selected => {
                return Err(DesktopAiError::VisionModelUnavailable);
            }
            Err(_) => return Err(DesktopAiError::VisionModelSelectionRequired),
        };
        Ok((binding, resolved))
    }

    pub(crate) async fn list_extractions(
        &self,
        project_id: Option<Uuid>,
    ) -> Result<Vec<AiExtractionDraft>, DesktopAiError> {
        self.store
            .list_ai_extraction_drafts(LOCAL_LAB_ID, LOCAL_USER_ID, project_id)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn create_extraction(
        &self,
        context: &AiExecutionContext,
        input: CreateAiExtractionInput,
    ) -> Result<AiExtractionDraft, DesktopAiError> {
        validate_image_ids(&input.image_ids)?;
        let data_cell = AiObservationDataCell {
            definition_id: input.current_data_cell.definition_id,
            subject_type: input.current_data_cell.subject_type,
            subject_id: input.current_data_cell.subject_id,
        };
        data_cell
            .validate()
            .map_err(|_| DesktopAiError::InvalidExtraction)?;
        let experiment = self.store.get_experiment(input.experiment_id).await?;
        if experiment.lab_id != LOCAL_LAB_ID
            || experiment.project_id != input.project_id
            || experiment.meta.deleted_at.is_some()
        {
            return Err(DesktopAiError::InvalidExtraction);
        }
        let event = self
            .store
            .get_experiment_event(input.experiment_event_id)
            .await?;
        if (event.lab_id, event.project_id, event.experiment_id)
            != (LOCAL_LAB_ID, input.project_id, input.experiment_id)
        {
            return Err(DesktopAiError::InvalidExtraction);
        }
        let definitions = self
            .store
            .list_observation_definitions(input.experiment_id)
            .await?;
        let definition = definitions
            .into_iter()
            .find(|definition| definition.id == data_cell.definition_id)
            .ok_or(DesktopAiError::InvalidExtraction)?;
        self.validate_data_cell_scope(input.project_id, input.experiment_id, &data_cell)
            .await?;
        let images = self
            .prepare_extraction_images(context, input.project_id, &input.image_ids)
            .await?;
        let (model_profile, resolved) = self
            .resolve_vision_provider(input.vision_model_profile_id)
            .await?;
        let credentials = match resolved.api_key.as_ref() {
            Some(secret) => ProviderCredentials::bearer(secret.as_str())
                .map_err(|_| DesktopAiError::ProviderUnavailable)?,
            None => ProviderCredentials::none(),
        };
        let prepared_images = images
            .iter()
            .map(|image| image.prepared.clone())
            .collect::<Vec<_>>();
        let extraction = extract_data_cell_vision(
            &resolved.provider,
            credentials,
            DataCellVisionExtractionRequest {
                model_profile,
                runtime: resolved.runtime,
                definition: &definition,
                images: &prepared_images,
            },
        )
        .await
        .map_err(map_extraction_error)?;
        let (candidate_value, confidence, source_label) = extraction.candidate.into_parts();
        let now = Utc::now();
        let observation = Observation::new(
            LOCAL_LAB_ID,
            input.project_id,
            input.experiment_id,
            input.experiment_event_id,
            data_cell.definition_id,
            data_cell.subject_type,
            data_cell.subject_id,
            now,
        )
        .map_err(|_| DesktopAiError::InvalidExtraction)?;
        let item = build_pending_extraction_item(
            observation,
            candidate_value,
            confidence,
            source_label,
            now,
        )?;
        let evidence = images
            .iter()
            .enumerate()
            .map(|(index, image)| AiExtractionEvidence {
                display_order: i32::try_from(index).unwrap_or(i32::MAX),
                private_image_id: image.image.id,
                private_attachment_id: image.attachment.id,
                promoted_attachment_id: None,
                original_sha256: image.attachment.sha256.clone(),
                sanitized_sha256: image.prepared.evidence().sanitized_sha256.clone(),
                meta: RecordMeta::new(now),
            })
            .collect::<Vec<_>>();
        let first = evidence
            .first()
            .ok_or(DesktopAiError::InvalidImageEvidence)?;
        let draft = AiExtractionDraft {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            project_id: input.project_id,
            experiment_id: input.experiment_id,
            experiment_event_id: input.experiment_event_id,
            private_image_id: first.private_image_id,
            attachment_id: first.private_attachment_id,
            image_sha256: first.original_sha256.clone(),
            provider: extraction.provider_id,
            model: extraction.model,
            tool_run_id: None,
            data_cell: Some(data_cell),
            evidence,
            model_trace: Some(AiExtractionModelTrace {
                profile_id: extraction.model_profile.profile_id,
                profile_version: extraction.model_profile.profile_version,
                purpose: AiModelPurpose::Vision,
                input_tokens: extraction.usage.input_tokens,
                output_tokens: extraction.usage.output_tokens,
                total_tokens: extraction.usage.total_tokens,
                provider_request_id: extraction.provider_request_id,
                trace: json!({
                    "purpose": "vision",
                    "imageCount": input.image_ids.len(),
                    "estimatedInputTokens": extraction.estimated_input_tokens,
                    "inputTokenCountIsEstimate": true,
                    "transport": "desktop",
                }),
            }),
            status: AiExtractionStatus::PendingApproval,
            items: vec![item],
            error_code: None,
            meta: RecordMeta::new(now),
        };
        draft
            .validate()
            .map_err(|_| DesktopAiError::InvalidExtraction)?;
        self.store
            .create_ai_extraction_draft(
                &draft,
                &desktop_audit(context, "create_ai_extraction_candidate"),
            )
            .await?;
        Ok(draft)
    }

    pub(crate) async fn approve_extraction(
        &self,
        context: &AiExecutionContext,
        id: Uuid,
        input: ApproveAiExtractionInput,
    ) -> Result<AppliedAiExtraction, DesktopAiError> {
        let draft = self.store.get_ai_extraction_draft(id).await?;
        if draft.lab_id != LOCAL_LAB_ID || draft.user_id != LOCAL_USER_ID {
            return Err(DesktopAiError::InvalidExtraction);
        }
        let approval = build_extraction_approval(input)?;
        self.store
            .apply_ai_extraction_draft(
                id,
                &approval,
                &desktop_audit(context, "approve_ai_extraction_candidate"),
            )
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn reject_extraction(
        &self,
        context: &AiExecutionContext,
        id: Uuid,
        input: RejectAiExtractionInput,
    ) -> Result<AiExtractionDraft, DesktopAiError> {
        let draft = self.store.get_ai_extraction_draft(id).await?;
        if draft.lab_id != LOCAL_LAB_ID || draft.user_id != LOCAL_USER_ID {
            return Err(DesktopAiError::InvalidExtraction);
        }
        let rejection = AiExtractionRejectionInput {
            expected_revision: input.expected_revision,
        };
        rejection
            .validate()
            .map_err(|_| DesktopAiError::InvalidExtraction)?;
        self.store
            .reject_ai_extraction_draft(
                id,
                &rejection,
                &desktop_audit(context, "reject_ai_extraction_candidate"),
            )
            .await
            .map_err(Into::into)
    }

    async fn private_image(
        &self,
        id: Uuid,
    ) -> Result<(PrivateAiImage, Attachment), DesktopAiError> {
        let image = self.store.get_private_ai_image(id).await?;
        let attachment = self.store.get_attachment(image.attachment_id).await?;
        ensure_private_image_scope(&image, &attachment)?;
        Ok((image, attachment))
    }

    async fn prepare_ai_input(
        &self,
        context: &AiExecutionContext,
        image: &PrivateAiImage,
        attachment: &Attachment,
        media_type: &str,
    ) -> Result<PreparedAssistantImage, DesktopAiError> {
        if let Some(derivative) = self
            .store
            .list_attachment_derivatives(attachment.id)
            .await?
            .into_iter()
            .find(|derivative| {
                derivative.kind == DerivativeKind::AiInput
                    && derivative.status == DerivativeStatus::Ready
            })
        {
            if derivative.lab_id != LOCAL_LAB_ID
                || derivative.project_id.is_some()
                || derivative.attachment_id != attachment.id
                || derivative.media_type.as_deref() != Some(media_type)
                || derivative.relative_path.is_none()
                || derivative.size_bytes.is_none()
                || derivative.sha256.is_none()
            {
                return Err(DesktopAiError::InvalidImageEvidence);
            }
            let surrogate = Attachment {
                id: derivative.id,
                lab_id: derivative.lab_id,
                project_id: derivative.project_id,
                entity_type: "attachment_derivative".to_owned(),
                entity_id: attachment.id,
                file_name: format!("ai-input-{}", attachment.file_name),
                media_type: derivative.media_type.clone(),
                relative_path: derivative
                    .relative_path
                    .clone()
                    .ok_or(DesktopAiError::InvalidImageEvidence)?,
                size_bytes: derivative
                    .size_bytes
                    .ok_or(DesktopAiError::InvalidImageEvidence)?,
                sha256: derivative
                    .sha256
                    .clone()
                    .ok_or(DesktopAiError::InvalidImageEvidence)?,
                version: 1,
                meta: derivative.meta.clone(),
            };
            let bytes = self.files.read_verified_bytes(&surrogate).await?;
            let sanitized = sanitize_vision_input(media_type, &bytes)
                .map_err(|_| DesktopAiError::InvalidImageEvidence)?;
            if derivative.sha256.as_deref() != Some(sanitized.sha256()) {
                return Err(DesktopAiError::InvalidImageEvidence);
            }
            return sanitized
                .prepared_image(image.id)
                .map_err(|_| DesktopAiError::InvalidImageEvidence);
        }

        let original = self.files.read_verified_bytes(attachment).await?;
        let sanitized = sanitize_vision_input(media_type, &original)
            .map_err(|_| DesktopAiError::InvalidImageEvidence)?;
        let derivative_id = Uuid::new_v4();
        let object = self
            .files
            .write_bytes(derivative_id, sanitized.bytes())
            .await?;
        if object.sha256 != sanitized.sha256() {
            self.files.remove_installed_object(&object).await.ok();
            return Err(DesktopAiError::InvalidImageEvidence);
        }
        let derivative = AttachmentDerivative {
            id: derivative_id,
            lab_id: LOCAL_LAB_ID,
            // Private AI input remains outside project scope until the
            // separate human approval transaction promotes its evidence.
            project_id: None,
            attachment_id: attachment.id,
            kind: DerivativeKind::AiInput,
            media_type: Some(sanitized.media_type().to_owned()),
            relative_path: Some(object.relative_path.clone()),
            size_bytes: Some(object.size_bytes),
            sha256: Some(object.sha256.clone()),
            status: DerivativeStatus::Ready,
            error_code: None,
            meta: RecordMeta::new(Utc::now()),
        };
        if let Err(error) = self
            .store
            .create_attachment_derivative(
                &derivative,
                &desktop_audit(context, "sanitize_private_ai_image"),
            )
            .await
        {
            self.files.remove_installed_object(&object).await.ok();
            return Err(error.into());
        }
        sanitized
            .prepared_image(image.id)
            .map_err(|_| DesktopAiError::InvalidImageEvidence)
    }

    async fn prepare_extraction_images(
        &self,
        context: &AiExecutionContext,
        project_id: Uuid,
        image_ids: &[Uuid],
    ) -> Result<Vec<PreparedPrivateImage>, DesktopAiError> {
        let prepared = self
            .prepare_assistant_images(context, None, Some(project_id), image_ids)
            .await?;
        let mut images = Vec::with_capacity(image_ids.len());
        for (image_id, prepared) in image_ids.iter().zip(prepared) {
            let (image, attachment) = self.private_image(*image_id).await?;
            images.push(PreparedPrivateImage {
                prepared,
                image,
                attachment,
            });
        }
        Ok(images)
    }

    async fn validate_data_cell_scope(
        &self,
        project_id: Uuid,
        experiment_id: Uuid,
        cell: &AiObservationDataCell,
    ) -> Result<(), DesktopAiError> {
        let valid = match cell.subject_type {
            ObservationSubjectType::Experiment => cell.subject_id == experiment_id,
            ObservationSubjectType::Animal => !self
                .store
                .list_participations(&ParticipationFilter {
                    project_id,
                    experiment_id: Some(experiment_id),
                    animal_id: Some(cell.subject_id),
                    cohort_id: None,
                })
                .await?
                .is_empty(),
            ObservationSubjectType::Sample => {
                let sample = self.store.get_sample(cell.subject_id).await?;
                (sample.lab_id, sample.project_id, sample.experiment_id)
                    == (LOCAL_LAB_ID, project_id, Some(experiment_id))
            }
            ObservationSubjectType::Artifact => {
                let attachment = self.store.get_attachment(cell.subject_id).await?;
                (attachment.lab_id, attachment.project_id) == (LOCAL_LAB_ID, Some(project_id))
            }
        };
        valid.then_some(()).ok_or(DesktopAiError::InvalidExtraction)
    }
}

#[derive(Debug)]
struct PreparedPrivateImage {
    prepared: PreparedAssistantImage,
    image: PrivateAiImage,
    attachment: Attachment,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UploadPrivateAiImageInput {
    pub file_name: String,
    pub media_type: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePrivateAiImageInput {
    pub project_id: Uuid,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrivateImageView {
    pub image: PrivateAiImage,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_href: Option<String>,
    pub retention_days: i64,
}

impl PrivateImageView {
    fn new(image: PrivateAiImage, attachment: Attachment) -> Self {
        let retention_days = (image.expires_at - Utc::now()).num_days().max(0);
        Self {
            // Desktop content is intentionally read through the scoped
            // `read_private_ai_image` command, then exposed as a revocable
            // renderer-owned Blob URL. It must never be a raw filesystem URL.
            content_href: None,
            preview_href: None,
            file_name: attachment.file_name,
            media_type: attachment.media_type,
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256,
            retention_days,
            image,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrivateImageContent {
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAiExtractionInput {
    pub image_ids: Vec<Uuid>,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub experiment_event_id: Uuid,
    pub current_data_cell: CreateAiExtractionDataCellInput,
    pub vision_model_profile_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAiExtractionDataCellInput {
    pub definition_id: Uuid,
    pub subject_type: ObservationSubjectType,
    pub subject_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApproveAiExtractionInput {
    pub expected_revision: i64,
    pub selections: Vec<ApproveAiExtractionSelectionInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApproveAiExtractionSelectionInput {
    pub item_index: usize,
    pub value: ObservationValueData,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RejectAiExtractionInput {
    pub expected_revision: i64,
}

fn validate_image_ids(image_ids: &[Uuid]) -> Result<(), DesktopAiError> {
    if image_ids.is_empty()
        || image_ids.len() > MAX_VISION_IMAGES
        || image_ids.iter().any(Uuid::is_nil)
        || image_ids.iter().copied().collect::<BTreeSet<_>>().len() != image_ids.len()
    {
        Err(DesktopAiError::InvalidImageEvidence)
    } else {
        Ok(())
    }
}

fn validate_private_image_upload_size(size: usize) -> Result<(), DesktopAiError> {
    if size == 0 || size > MAX_SANITIZED_VISION_INPUT_BYTES {
        Err(DesktopAiError::InvalidImageEvidence)
    } else {
        Ok(())
    }
}

fn private_image_is_readable(status: PrivateImageStatus) -> bool {
    matches!(
        status,
        PrivateImageStatus::Active
            | PrivateImageStatus::PendingApproval
            | PrivateImageStatus::Archived
    )
}

fn base64_encoded_len(input_bytes: usize) -> Option<usize> {
    input_bytes.checked_add(2)?.checked_div(3)?.checked_mul(4)
}

fn accumulate_vision_base64_bytes(current: usize, next: usize) -> Result<usize, DesktopAiError> {
    let total = current
        .checked_add(next)
        .ok_or(DesktopAiError::InvalidImageEvidence)?;
    if total > MAX_VISION_TOTAL_BASE64_BYTES {
        Err(DesktopAiError::InvalidImageEvidence)
    } else {
        Ok(total)
    }
}

fn build_pending_extraction_item(
    observation: Observation,
    candidate_value: ObservationValueData,
    confidence: f64,
    source_label: Option<String>,
    now: chrono::DateTime<Utc>,
) -> Result<AiExtractionItem, DesktopAiError> {
    let mut value = ObservationValueRecord::new(observation.id, 1, candidate_value, now, now)
        .map_err(|_| DesktopAiError::InvalidExtraction)?;
    value.recorded_by = Some(LOCAL_USER_ID);
    value.notes = Some("AI visual extraction; pending human approval".to_owned());
    let item = AiExtractionItem {
        observation,
        value,
        confidence,
        // A Provider may propose a candidate but never pre-approve it.
        selected: false,
        source_label,
    };
    item.validate()
        .map_err(|_| DesktopAiError::InvalidExtraction)?;
    Ok(item)
}

fn build_extraction_approval(
    input: ApproveAiExtractionInput,
) -> Result<AiExtractionApprovalInput, DesktopAiError> {
    let approval = AiExtractionApprovalInput {
        expected_revision: input.expected_revision,
        selections: input
            .selections
            .into_iter()
            .map(|selection| AiExtractionApprovalSelection {
                item_index: selection.item_index,
                value: selection.value,
                notes: selection.notes,
            })
            .collect(),
    };
    approval
        .validate()
        .map_err(|_| DesktopAiError::InvalidExtraction)?;
    Ok(approval)
}

fn map_extraction_error(error: DataCellVisionExtractionError) -> DesktopAiError {
    match error {
        DataCellVisionExtractionError::Provider(_) => DesktopAiError::ProviderUnavailable,
        DataCellVisionExtractionError::InvalidImageEvidence => DesktopAiError::InvalidImageEvidence,
        DataCellVisionExtractionError::InvalidRequest
        | DataCellVisionExtractionError::ContextExceeded { .. }
        | DataCellVisionExtractionError::InvalidResponse => DesktopAiError::InvalidExtraction,
    }
}

fn validate_file_name(value: &str) -> Result<String, DesktopAiError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        Err(DesktopAiError::InvalidImageEvidence)
    } else {
        Ok(value.to_owned())
    }
}

fn ensure_private_image_scope(
    image: &PrivateAiImage,
    attachment: &Attachment,
) -> Result<(), DesktopAiError> {
    if image.lab_id != LOCAL_LAB_ID
        || image.user_id != LOCAL_USER_ID
        || image.meta.deleted_at.is_some()
        || attachment.lab_id != LOCAL_LAB_ID
        || attachment.entity_type != "ai_private_image"
        || attachment.entity_id != image.id
        || attachment.id != image.attachment_id
        || attachment.meta.deleted_at.is_some()
    {
        Err(DesktopAiError::InvalidImageEvidence)
    } else {
        Ok(())
    }
}

fn desktop_audit(context: &AiExecutionContext, reason: &'static str) -> AuditContext {
    AuditContext {
        actor: Actor::human(context.user_id, context.user_display_name.clone()),
        source: WriteSource::Desktop,
        request_id: Some(Uuid::new_v4().to_string()),
        reason: Some(reason.to_owned()),
    }
}

impl From<AttachmentFileError> for DesktopAiError {
    fn from(error: AttachmentFileError) -> Self {
        match error {
            AttachmentFileError::TooLarge => Self::InvalidImageEvidence,
            other => Self::ImageStorage(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_ai::{AccessGrant, ScopeSet, ToolScope};
    use muriarc_core::AiOperationStore;

    use crate::{application::DesktopState, data::DesktopDataState};

    #[test]
    fn image_and_approval_contracts_reject_transport_authority_fields() {
        let forged_upload = json!({
            "fileName": "evidence.png",
            "mediaType": "image/png",
            "bytes": [137, 80, 78, 71],
            "userId": Uuid::new_v4(),
        });
        assert!(serde_json::from_value::<UploadPrivateAiImageInput>(forged_upload).is_err());

        let forged_approval = json!({
            "expectedRevision": 1,
            "selections": [{
                "itemIndex": 0,
                "value": {"type": "number", "value": 12.5},
                "notes": "checked",
            }],
            "projectId": Uuid::new_v4(),
            "actorUserId": Uuid::new_v4(),
        });
        assert!(serde_json::from_value::<ApproveAiExtractionInput>(forged_approval).is_err());

        let forged_rejection = json!({
            "expectedRevision": 1,
            "actorUserId": Uuid::new_v4(),
        });
        assert!(serde_json::from_value::<RejectAiExtractionInput>(forged_rejection).is_err());
    }

    #[test]
    fn common_provider_image_formats_are_the_only_accepted_names_at_the_contract() {
        assert_eq!(validate_file_name("evidence.png").unwrap(), "evidence.png");
        for invalid in ["", "../escape.png", "nested/image.png", "script\n.png"] {
            assert!(validate_file_name(invalid).is_err());
        }
    }

    #[test]
    fn image_ids_require_one_to_eight_unique_non_nil_values() {
        let id = Uuid::new_v4();
        assert!(validate_image_ids(&[id]).is_ok());
        assert!(validate_image_ids(&[]).is_err());
        assert!(validate_image_ids(&[Uuid::nil()]).is_err());
        assert!(validate_image_ids(&[id, id]).is_err());
        assert!(validate_image_ids(&[Uuid::new_v4(); MAX_VISION_IMAGES + 1]).is_err());
    }

    #[test]
    fn desktop_upload_rejects_empty_or_oversized_images_before_writing_files() {
        assert!(validate_private_image_upload_size(1).is_ok());
        assert!(validate_private_image_upload_size(MAX_SANITIZED_VISION_INPUT_BYTES).is_ok());
        assert!(matches!(
            validate_private_image_upload_size(0),
            Err(DesktopAiError::InvalidImageEvidence)
        ));
        assert!(matches!(
            validate_private_image_upload_size(MAX_SANITIZED_VISION_INPUT_BYTES + 1),
            Err(DesktopAiError::InvalidImageEvidence)
        ));
    }

    #[test]
    fn desktop_owner_can_read_active_pending_and_archived_evidence_only() {
        for status in [
            PrivateImageStatus::Active,
            PrivateImageStatus::PendingApproval,
            PrivateImageStatus::Archived,
        ] {
            assert!(private_image_is_readable(status));
        }
        for status in [
            PrivateImageStatus::Processing,
            PrivateImageStatus::Expired,
            PrivateImageStatus::Failed,
        ] {
            assert!(!private_image_is_readable(status));
        }
    }

    #[test]
    fn desktop_rejects_multi_image_base64_payload_before_the_total_limit_is_crossed() {
        let first = MAX_VISION_TOTAL_BASE64_BYTES / 2;
        let second = MAX_VISION_TOTAL_BASE64_BYTES - first;
        let total = accumulate_vision_base64_bytes(0, first).unwrap();
        let total = accumulate_vision_base64_bytes(total, second).unwrap();
        assert_eq!(total, MAX_VISION_TOTAL_BASE64_BYTES);
        assert!(matches!(
            accumulate_vision_base64_bytes(total, 4),
            Err(DesktopAiError::InvalidImageEvidence)
        ));
        assert_eq!(base64_encoded_len(1), Some(4));
        assert_eq!(base64_encoded_len(3), Some(4));
        assert_eq!(base64_encoded_len(4), Some(8));
    }

    #[test]
    fn desktop_extraction_candidate_stays_unselected_until_human_approval() {
        let now = Utc::now();
        let experiment_id = Uuid::new_v4();
        let observation = Observation::new(
            LOCAL_LAB_ID,
            Uuid::new_v4(),
            experiment_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            ObservationSubjectType::Experiment,
            experiment_id,
            now,
        )
        .unwrap();
        let item = build_pending_extraction_item(
            observation,
            ObservationValueData::Number(12.5),
            0.8,
            Some("visible reading".to_owned()),
            now,
        )
        .unwrap();

        assert!(!item.selected);
        assert_eq!(item.value.recorded_by, Some(LOCAL_USER_ID));
        assert_eq!(
            item.value.notes.as_deref(),
            Some("AI visual extraction; pending human approval")
        );
    }

    #[test]
    fn desktop_approval_maps_only_human_editable_candidate_fields() {
        let approval = build_extraction_approval(ApproveAiExtractionInput {
            expected_revision: 7,
            selections: vec![ApproveAiExtractionSelectionInput {
                item_index: 0,
                value: ObservationValueData::Number(12.75),
                notes: Some("researcher corrected decimal".to_owned()),
            }],
        })
        .unwrap();

        assert_eq!(approval.expected_revision, 7);
        assert_eq!(approval.selections.len(), 1);
        assert_eq!(approval.selections[0].item_index, 0);
        assert_eq!(
            approval.selections[0].value,
            ObservationValueData::Number(12.75)
        );
        assert_eq!(
            approval.selections[0].notes.as_deref(),
            Some("researcher corrected decimal")
        );
    }

    #[tokio::test]
    async fn missing_default_vision_profile_fails_before_a_provider_can_be_resolved() {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        let images = DesktopAiImages::new(
            store,
            AttachmentFiles::new(temp.path().join("attachments")),
            SettingsService::for_app_data(temp.path()),
        );

        let error = match images.resolve_vision_provider(None).await {
            Err(error) => error,
            Ok(_) => panic!("missing vision default must be rejected"),
        };
        assert_eq!(error.code(), "vision_model_selection_required");
    }

    #[tokio::test]
    async fn desktop_private_image_read_and_ai_input_derivative_remain_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        DesktopState::initialize(&database).await.unwrap();
        let data = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let store = Arc::new(data.store_ref().clone());
        let domain_store: Arc<dyn MuriArcStore> = store.clone();
        let operation_store: Arc<dyn AiOperationStore> = store.clone();
        let workflow = AiWorkflowService::new(domain_store, operation_store);
        let images = DesktopAiImages::new(
            store.clone(),
            data.attachments_ref().clone(),
            SettingsService::for_app_data(temp.path()),
        );
        let context = AiExecutionContext::new(
            LOCAL_LAB_ID,
            LOCAL_USER_ID,
            "Local researcher",
            Uuid::new_v4().to_string(),
            [],
            [],
            true,
            AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
        );
        let bytes = include_bytes!("../icons/32x32.png").to_vec();

        let uploaded = images
            .upload(
                &workflow,
                &context,
                UploadPrivateAiImageInput {
                    file_name: "desktop-private-image.png".to_owned(),
                    media_type: Some("image/png".to_owned()),
                    conversation_id: None,
                    bytes: bytes.clone(),
                },
            )
            .await
            .unwrap();
        assert!(uploaded.content_href.is_none());
        assert!(uploaded.preview_href.is_none());
        assert!(uploaded.image.project_id.is_none());

        let content = images.read(uploaded.image.id).await.unwrap();
        assert_eq!(content.media_type.as_deref(), Some("image/png"));
        assert_eq!(content.bytes, bytes);

        let prepared = images
            .prepare_assistant_images(&context, None, None, &[uploaded.image.id])
            .await
            .unwrap();
        assert_eq!(prepared.len(), 1);
        let derivatives = store
            .list_attachment_derivatives(uploaded.image.attachment_id)
            .await
            .unwrap();
        assert_eq!(derivatives.len(), 1);
        assert_eq!(derivatives[0].kind, DerivativeKind::AiInput);
        assert_eq!(derivatives[0].status, DerivativeStatus::Ready);
        assert!(derivatives[0].project_id.is_none());
        assert_eq!(
            derivatives[0].sha256.as_deref(),
            Some(prepared[0].evidence().sanitized_sha256.as_str())
        );
    }
}
