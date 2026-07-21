use std::path::Path;

use chrono::Utc;
use muriarc_core::{
    Actor, AnimalFilter, Attachment, AuditContext, Job, JobKind, JobStatus, LOCAL_LAB_ID,
    LOCAL_USER_ID, MuriArcStore, RecordMeta, StoreError, WriteSource,
};
use muriarc_data::{
    AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata, AttachmentFileError,
    AttachmentFiles, DataError, DataFiles, ExportFormat, ImportKind, ImportRemapJobResult,
    StoredAttachmentObject, artifact_metadata, build_lab_snapshot, export_animals,
};
use muriarc_importer::{AnimalExportFilter, FieldMapping, MeasurementFieldMapping};
use muriarc_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

const MAX_ATTACHMENT_FILE_NAME_BYTES: usize = 255;
const MAX_ATTACHMENT_MEDIA_TYPE_BYTES: usize = 127;
const MAX_ATTACHMENT_VERSION: i32 = 1_000_000;

#[derive(Debug, Error)]
pub(crate) enum DesktopDataError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Data(#[from] DataError),
    #[error(transparent)]
    Attachment(#[from] AttachmentFileError),
    #[error("invalid {0} identifier")]
    InvalidId(&'static str),
    #[error("job does not belong to the local operator")]
    ScopeMismatch,
    #[error("job is not in the required state")]
    InvalidJobState,
    #[error("idempotency key must contain 1-128 non-control characters")]
    InvalidIdempotencyKey,
}

impl DesktopDataError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Data(DataError::Conflict(_))
            | Self::Store(StoreError::Conflict(_))
            | Self::Attachment(AttachmentFileError::AlreadyExists) => "conflict",
            Self::Data(DataError::NotFound) | Self::Store(StoreError::NotFound { .. }) => {
                "not_found"
            }
            Self::Data(
                DataError::InvalidFileName
                | DataError::EmptyUpload
                | DataError::UnsupportedUpload(_)
                | DataError::UploadTooLarge(_)
                | DataError::ArtifactTooLarge(_)
                | DataError::ScopeMismatch
                | DataError::PreviewHasErrors
                | DataError::Plan(_)
                | DataError::Directory(_),
            )
            | Self::Attachment(AttachmentFileError::TooLarge)
            | Self::Store(StoreError::Validation(_))
            | Self::InvalidId(_)
            | Self::ScopeMismatch
            | Self::InvalidJobState
            | Self::InvalidIdempotencyKey => "validation",
            Self::Data(
                DataError::ChecksumMismatch(_)
                | DataError::CorruptState(_)
                | DataError::Store(_)
                | DataError::Import(_)
                | DataError::Snapshot(_)
                | DataError::Json(_)
                | DataError::Io(_)
                | DataError::Attachment(_),
            )
            | Self::Attachment(
                AttachmentFileError::UnsafePath
                | AttachmentFileError::Missing
                | AttachmentFileError::Integrity
                | AttachmentFileError::Io(_),
            )
            | Self::Store(StoreError::Database(_) | StoreError::Serialization(_)) => {
                "storage_error"
            }
        }
    }

    pub(crate) fn safe_message(&self) -> String {
        match self.code() {
            "storage_error" => "本地数据文件不可用或校验失败，请查看诊断日志".to_owned(),
            _ => self.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopDataState {
    store: SqliteStore,
    files: DataFiles,
    attachments: AttachmentFiles,
}

impl DesktopDataState {
    pub(crate) async fn initialize(
        database_path: impl AsRef<Path>,
        app_data_dir: impl AsRef<Path>,
    ) -> Result<Self, DesktopDataError> {
        let store = SqliteStore::connect_path(database_path).await?;
        store.migrate().await?;
        let attachments = AttachmentFiles::new(app_data_dir.as_ref().join("attachments"));
        attachments.initialize().await?;
        Ok(Self {
            store,
            files: DataFiles::new(app_data_dir.as_ref().join("data")),
            attachments,
        })
    }

    async fn audit(&self, reason: &'static str) -> Result<AuditContext, DesktopDataError> {
        let operator = self.store.get_user(LOCAL_USER_ID).await?;
        Ok(AuditContext {
            actor: Actor::human(LOCAL_USER_ID, operator.display_name),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some(reason.to_owned()),
        })
    }

    pub(crate) async fn list_attachments(
        &self,
        input: AttachmentScopeInput,
    ) -> Result<Vec<AttachmentView>, DesktopDataError> {
        let entity_id = parse_id("attachment target", &input.entity_id)?;
        let requested_project_id = input
            .project_id
            .as_deref()
            .map(|value| parse_id("project", value))
            .transpose()?;
        let effective_project = self
            .authorize_attachment_target(input.entity_type, entity_id, requested_project_id)
            .await?;
        let mut attachments = self
            .store
            .list_attachments(LOCAL_LAB_ID, input.entity_type.as_str(), entity_id)
            .await?;
        if let Some(project_id) = effective_project {
            attachments.retain(|attachment| attachment.project_id == Some(project_id));
        }
        Ok(attachments.iter().map(AttachmentView::from).collect())
    }

    pub(crate) async fn upload_attachment(
        &self,
        input: UploadAttachmentInput,
    ) -> Result<AttachmentView, DesktopDataError> {
        let entity_id = parse_id("attachment target", &input.entity_id)?;
        let requested_project_id = input
            .project_id
            .as_deref()
            .map(|value| parse_id("project", value))
            .transpose()?;
        let effective_project = self
            .authorize_attachment_target(input.entity_type, entity_id, requested_project_id)
            .await?;
        let file_name = validate_attachment_file_name(input.file_name)?;
        let media_type = validate_attachment_media_type(input.media_type)?;
        if input.bytes.len() as u64 > self.attachments.max_bytes() {
            return Err(AttachmentFileError::TooLarge.into());
        }
        let version = self
            .store
            .list_attachments(LOCAL_LAB_ID, input.entity_type.as_str(), entity_id)
            .await?
            .into_iter()
            .filter(|attachment| attachment.file_name == file_name)
            .map(|attachment| attachment.version)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|version| *version <= MAX_ATTACHMENT_VERSION)
            .ok_or_else(|| {
                StoreError::Validation("attachment version limit was reached".to_owned())
            })?;
        let audit = self.audit("upload_attachment").await?;
        let id = Uuid::new_v4();
        let object = self.attachments.write_bytes(id, &input.bytes).await?;
        let attachment = Attachment {
            id,
            lab_id: LOCAL_LAB_ID,
            project_id: effective_project,
            entity_type: input.entity_type.as_str().to_owned(),
            entity_id,
            file_name,
            media_type,
            relative_path: object.relative_path.clone(),
            size_bytes: object.size_bytes,
            sha256: object.sha256.clone(),
            version,
            meta: RecordMeta::new(Utc::now()),
        };
        self.commit_attachment(&attachment, &object, &audit).await?;
        Ok(AttachmentView::from(&attachment))
    }

    pub(crate) async fn download_attachment(
        &self,
        id: &str,
    ) -> Result<AttachmentDownloadView, DesktopDataError> {
        let id = parse_id("attachment", id)?;
        let attachment = self.store.get_attachment(id).await?;
        if attachment.lab_id != LOCAL_LAB_ID {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let entity_type = AttachmentTargetInput::from_stored(&attachment.entity_type)
            .ok_or(DesktopDataError::ScopeMismatch)?;
        let effective_project = self
            .authorize_attachment_target(entity_type, attachment.entity_id, attachment.project_id)
            .await?;
        if effective_project != attachment.project_id {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let bytes = self.attachments.read_verified_bytes(&attachment).await?;
        Ok(AttachmentDownloadView {
            metadata: AttachmentView::from(&attachment),
            bytes,
        })
    }

    async fn commit_attachment(
        &self,
        attachment: &Attachment,
        object: &StoredAttachmentObject,
        audit: &AuditContext,
    ) -> Result<(), DesktopDataError> {
        if let Err(error) = self.store.create_attachment(attachment, audit).await {
            self.attachments.remove_installed_object(object).await?;
            return Err(error.into());
        }
        Ok(())
    }

    async fn authorize_attachment_target(
        &self,
        entity_type: AttachmentTargetInput,
        entity_id: Uuid,
        requested_project_id: Option<Uuid>,
    ) -> Result<Option<Uuid>, DesktopDataError> {
        match entity_type {
            AttachmentTargetInput::Project => {
                if requested_project_id.is_some_and(|project_id| project_id != entity_id) {
                    return Err(DesktopDataError::ScopeMismatch);
                }
                let project = self.store.get_project(entity_id).await?;
                ensure_local_lab(project.lab_id)?;
                Ok(Some(entity_id))
            }
            AttachmentTargetInput::Animal => {
                if let Some(project_id) = requested_project_id {
                    let project = self.store.get_project(project_id).await?;
                    ensure_local_lab(project.lab_id)?;
                }
                let animal = self.store.get_animal(entity_id).await?;
                ensure_local_lab(animal.lab_id)?;
                if let Some(project_id) = requested_project_id {
                    let visible = self
                        .store
                        .list_animals(&AnimalFilter {
                            lab_id: LOCAL_LAB_ID,
                            project_id: Some(project_id),
                            ..AnimalFilter::default()
                        })
                        .await?
                        .into_iter()
                        .any(|candidate| candidate.id == entity_id);
                    if !visible {
                        return Err(StoreError::NotFound {
                            entity: "animal",
                            id: entity_id,
                        }
                        .into());
                    }
                }
                Ok(requested_project_id)
            }
            AttachmentTargetInput::Experiment => {
                let experiment = self.store.get_experiment(entity_id).await?;
                ensure_local_lab(experiment.lab_id)?;
                ensure_requested_project(requested_project_id, experiment.project_id)?;
                Ok(Some(experiment.project_id))
            }
            AttachmentTargetInput::Measurement => {
                let measurement = self.store.get_measurement(entity_id).await?;
                ensure_local_lab(measurement.lab_id)?;
                ensure_requested_project(requested_project_id, measurement.project_id)?;
                Ok(Some(measurement.project_id))
            }
            AttachmentTargetInput::Sample => {
                let sample = self.store.get_sample(entity_id).await?;
                ensure_local_lab(sample.lab_id)?;
                ensure_requested_project(requested_project_id, sample.project_id)?;
                Ok(Some(sample.project_id))
            }
        }
    }

    pub(crate) async fn preview_import(
        &self,
        input: PreviewDataImportInput,
    ) -> Result<AnimalImportPreviewResponse, DesktopDataError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let (project_id, experiment_id) = match input.import_kind {
            ImportKind::Animal => {
                if input.experiment_id.is_some() {
                    return Err(DesktopDataError::ScopeMismatch);
                }
                (None, None)
            }
            ImportKind::Measurement => {
                let experiment_id = parse_id(
                    "experiment",
                    input
                        .experiment_id
                        .as_deref()
                        .ok_or(DesktopDataError::InvalidId("experiment"))?,
                )?;
                let experiment = self.store.get_experiment(experiment_id).await?;
                if experiment.lab_id != LOCAL_LAB_ID {
                    return Err(DesktopDataError::ScopeMismatch);
                }
                (Some(experiment.project_id), Some(experiment_id))
            }
        };
        if let Some(existing) = self
            .store
            .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &input.idempotency_key)
            .await?
        {
            ensure_job_scope(&existing, JobKind::Import)?;
            if existing.project_id != project_id {
                return Err(DesktopDataError::ScopeMismatch);
            }
            if existing.status == JobStatus::AwaitingConfirmation {
                return match input.import_kind {
                    ImportKind::Animal => {
                        Ok((&self.files.read_pending_import(existing.id).await?).into())
                    }
                    ImportKind::Measurement => {
                        let pending = self
                            .files
                            .read_pending_measurement_import(existing.id)
                            .await?;
                        if Some(pending.experiment_id) != experiment_id {
                            return Err(DesktopDataError::ScopeMismatch);
                        }
                        Ok((&pending).into())
                    }
                };
            }
            return Err(DesktopDataError::InvalidJobState);
        }

        let now = Utc::now();
        let mut job = Job {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id,
            created_by: LOCAL_USER_ID,
            kind: JobKind::Import,
            status: JobStatus::Parsing,
            idempotency_key: input.idempotency_key,
            progress_current: 0,
            progress_total: Some(3),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        let audit = self.audit("preview_data_import").await?;
        self.store.create_job(&job, &audit).await?;

        let operation = async {
            self.files
                .write_upload_bytes(job.id, &input.file_name, &input.bytes)
                .await?;
            match input.import_kind {
                ImportKind::Animal => {
                    let pending = self.files.preview_animal_import(&job, &self.store).await?;
                    Ok::<_, DesktopDataError>((&pending).into())
                }
                ImportKind::Measurement => {
                    let pending = self
                        .files
                        .preview_measurement_import(
                            &job,
                            experiment_id.expect("measurement selection has an experiment"),
                            &self.store,
                        )
                        .await?;
                    Ok::<_, DesktopDataError>((&pending).into())
                }
            }
        }
        .await;
        match operation {
            Ok(preview) => {
                transition_job(
                    &self.store,
                    &mut job,
                    JobStatus::AwaitingConfirmation,
                    2,
                    Some(serde_json::to_value(&preview).map_err(DataError::from)?),
                    None,
                    &audit,
                )
                .await?;
                Ok(preview)
            }
            Err(error) => {
                let _ = transition_job(
                    &self.store,
                    &mut job,
                    JobStatus::Failed,
                    0,
                    None,
                    Some(json!({ "code": error.code() })),
                    &audit,
                )
                .await;
                Err(error)
            }
        }
    }

    pub(crate) async fn remap_import(
        &self,
        input: RemapDataImportInput,
    ) -> Result<AnimalImportPreviewResponse, DesktopDataError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let previous_id = parse_id("job", &input.job_id)?;
        let mut previous = self.store.get_job(previous_id).await?;
        ensure_job_scope(&previous, JobKind::Import)?;

        if let Some(existing) = self
            .store
            .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &input.idempotency_key)
            .await?
        {
            return exact_local_remap_replay(&existing, &previous, &input.mapping);
        }
        if previous.status != JobStatus::AwaitingConfirmation || previous.cancellation_requested {
            return Err(DesktopDataError::InvalidJobState);
        }

        let experiment_id = if previous.project_id.is_some() {
            let pending = self
                .files
                .read_pending_measurement_import(previous.id)
                .await?;
            if pending.job_id != previous.id
                || pending.lab_id != previous.lab_id
                || pending.created_by != previous.created_by
                || Some(pending.project_id) != previous.project_id
            {
                return Err(DesktopDataError::ScopeMismatch);
            }
            Some(pending.experiment_id)
        } else {
            let pending = self.files.read_pending_import(previous.id).await?;
            if pending.job_id != previous.id
                || pending.lab_id != previous.lab_id
                || pending.created_by != previous.created_by
                || pending.project_id != previous.project_id
            {
                return Err(DesktopDataError::ScopeMismatch);
            }
            None
        };

        let now = Utc::now();
        let mut replacement = Job {
            id: Uuid::new_v4(),
            lab_id: previous.lab_id,
            project_id: previous.project_id,
            created_by: previous.created_by,
            kind: JobKind::Import,
            status: JobStatus::Parsing,
            idempotency_key: input.idempotency_key,
            progress_current: 0,
            progress_total: Some(3),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        let audit = self.audit("remap_data_import").await?;
        if let Err(error) = self.store.create_job(&replacement, &audit).await {
            if matches!(error, StoreError::Conflict(_))
                && let Some(existing) = self
                    .store
                    .find_job_by_idempotency(
                        LOCAL_LAB_ID,
                        LOCAL_USER_ID,
                        &replacement.idempotency_key,
                    )
                    .await?
            {
                return exact_local_remap_replay(&existing, &previous, &input.mapping);
            }
            return Err(error.into());
        }

        let canonical_mapping = input.mapping;
        let operation = async {
            self.files.copy_upload(previous.id, replacement.id).await?;
            match experiment_id {
                Some(experiment_id) => Ok::<_, DesktopDataError>(
                    (&self
                        .files
                        .preview_measurement_import_with_mapping(
                            &replacement,
                            experiment_id,
                            &self.store,
                            Some(MeasurementFieldMapping {
                                columns: canonical_mapping.columns.clone(),
                            }),
                        )
                        .await?)
                        .into(),
                ),
                None => Ok((&self
                    .files
                    .preview_animal_import_with_mapping(
                        &replacement,
                        &self.store,
                        Some(canonical_mapping.clone()),
                    )
                    .await?)
                    .into()),
            }
        }
        .await;
        let preview: AnimalImportPreviewResponse = match operation {
            Ok(preview) => preview,
            Err(error) => {
                self.discard_import_replacement(
                    replacement.id,
                    JobStatus::Failed,
                    error.code(),
                    &audit,
                )
                .await;
                return Err(error);
            }
        };

        let result = ImportRemapJobResult {
            source_job_id: previous.id,
            mapping: canonical_mapping,
            preview: preview.clone(),
        };
        let result = match serde_json::to_value(result) {
            Ok(result) => result,
            Err(error) => {
                self.discard_import_replacement(
                    replacement.id,
                    JobStatus::Failed,
                    "storage_error",
                    &audit,
                )
                .await;
                return Err(DataError::Json(error).into());
            }
        };
        if let Err(error) = transition_job(
            &self.store,
            &mut replacement,
            JobStatus::AwaitingConfirmation,
            2,
            Some(result),
            None,
            &audit,
        )
        .await
        {
            self.discard_import_replacement(
                replacement.id,
                JobStatus::Failed,
                "storage_error",
                &audit,
            )
            .await;
            return Err(error);
        }

        previous.cancellation_requested = true;
        let previous_progress = previous.progress_current;
        let previous_result = previous.result.clone();
        if let Err(error) = transition_job(
            &self.store,
            &mut previous,
            JobStatus::Cancelled,
            previous_progress,
            previous_result,
            None,
            &audit,
        )
        .await
        {
            self.discard_import_replacement(
                replacement.id,
                JobStatus::Cancelled,
                "conflict",
                &audit,
            )
            .await;
            return Err(error);
        }
        let _ = self.files.clear_pending_import(previous.id).await;
        let _ = self.files.clear_upload(previous.id).await;
        Ok(preview)
    }

    async fn discard_import_replacement(
        &self,
        job_id: Uuid,
        status: JobStatus,
        code: &'static str,
        audit: &AuditContext,
    ) {
        if let Ok(mut job) = self.store.get_job(job_id).await {
            job.cancellation_requested = true;
            let progress = job.progress_current;
            let result = job.result.clone();
            let _ = transition_job(
                &self.store,
                &mut job,
                status,
                progress,
                result,
                Some(json!({ "code": code })),
                audit,
            )
            .await;
        }
        let _ = self.files.clear_pending_import(job_id).await;
        let _ = self.files.clear_upload(job_id).await;
    }

    pub(crate) async fn confirm_import(
        &self,
        input: ConfirmDataImportInput,
    ) -> Result<ImportReceiptView, DesktopDataError> {
        let job_id = parse_id("job", &input.job_id)?;
        let mut job = self.store.get_job(job_id).await?;
        ensure_job_scope(&job, JobKind::Import)?;
        if job.status == JobStatus::Completed {
            let value = job
                .result
                .clone()
                .ok_or(DesktopDataError::InvalidJobState)?;
            let result = serde_json::from_value::<muriarc_core::ImportCommitResult>(value)
                .map_err(DataError::from)?;
            if !result
                .preview_hash
                .eq_ignore_ascii_case(input.preview_hash.trim())
            {
                return Err(DataError::Conflict(
                    "confirmed preview hash does not match the completed import".to_owned(),
                )
                .into());
            }
            let mut view = ImportReceiptView::from_result(job.id, result);
            view.replayed = true;
            return Ok(view);
        }
        if job.status != JobStatus::AwaitingConfirmation || job.cancellation_requested {
            return Err(DesktopDataError::InvalidJobState);
        }
        let audit = self.audit("confirm_data_import").await?;
        let receipt = if job.project_id.is_some() {
            self.files
                .confirm_measurement_import(
                    &job,
                    &input.preview_hash,
                    &self.store,
                    &audit,
                    Utc::now(),
                )
                .await?
        } else {
            self.files
                .confirm_animal_import(&job, &input.preview_hash, &self.store, &audit, Utc::now())
                .await?
        };
        transition_job(
            &self.store,
            &mut job,
            JobStatus::Completed,
            3,
            Some(serde_json::to_value(&receipt).map_err(DataError::from)?),
            None,
            &audit,
        )
        .await?;
        let _ = self.files.clear_pending_import(job.id).await;
        let _ = self.files.clear_upload(job.id).await;
        Ok(ImportReceiptView::from_result(job.id, receipt))
    }

    pub(crate) async fn cancel_import(
        &self,
        input: CancelDataImportInput,
    ) -> Result<(), DesktopDataError> {
        let job_id = parse_id("job", &input.job_id)?;
        let mut job = self.store.get_job(job_id).await?;
        ensure_job_scope(&job, JobKind::Import)?;
        if matches!(job.status, JobStatus::Completed | JobStatus::Writing) {
            return Err(DesktopDataError::InvalidJobState);
        }
        if job.status != JobStatus::Cancelled {
            job.cancellation_requested = true;
            let audit = self.audit("cancel_data_import").await?;
            let progress_current = job.progress_current;
            let result = job.result.clone();
            transition_job(
                &self.store,
                &mut job,
                JobStatus::Cancelled,
                progress_current,
                result,
                None,
                &audit,
            )
            .await?;
        }
        self.files.clear_pending_import(job.id).await?;
        self.files.clear_upload(job.id).await?;
        Ok(())
    }

    pub(crate) async fn create_export(
        &self,
        input: CreateDataExportInput,
    ) -> Result<DataArtifactView, DesktopDataError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let format = input.format;
        self.create_artifact_job(JobKind::Export, input.idempotency_key, move |job, state| {
            Box::pin(async move {
                let bytes = export_animals(
                    &state.store,
                    LOCAL_LAB_ID,
                    format,
                    &AnimalExportFilter::default(),
                )
                .await?;
                let file_name = format!(
                    "muriarc-animals-{}.{}",
                    job.meta.created_at.format("%Y%m%d-%H%M%S"),
                    format.extension()
                );
                let metadata = artifact_metadata(
                    job.id,
                    ArtifactKind::Export,
                    file_name,
                    format.media_type().to_owned(),
                    &bytes,
                    job.meta.created_at,
                )?;
                Ok((metadata, bytes))
            })
        })
        .await
    }

    pub(crate) async fn create_snapshot(
        &self,
        input: CreateDataSnapshotInput,
    ) -> Result<DataArtifactView, DesktopDataError> {
        validate_idempotency_key(&input.idempotency_key)?;
        self.create_artifact_job(JobKind::Snapshot, input.idempotency_key, |job, state| {
            Box::pin(async move {
                let origin_instance_id = state.files.instance_id().await?;
                let bytes = build_lab_snapshot(
                    &state.store,
                    state.attachments.root(),
                    job.id,
                    origin_instance_id,
                    LOCAL_LAB_ID,
                    Some(LOCAL_USER_ID),
                    job.meta.created_at,
                )
                .await?;
                let metadata = artifact_metadata(
                    job.id,
                    ArtifactKind::Snapshot,
                    format!(
                        "muriarc-snapshot-{}.muriarc.zip",
                        job.meta.created_at.format("%Y%m%d-%H%M%S")
                    ),
                    "application/vnd.muriarc.snapshot+zip".to_owned(),
                    &bytes,
                    job.meta.created_at,
                )?;
                Ok((metadata, bytes))
            })
        })
        .await
    }

    async fn create_artifact_job<F>(
        &self,
        kind: JobKind,
        idempotency_key: String,
        build: F,
    ) -> Result<DataArtifactView, DesktopDataError>
    where
        F: for<'a> FnOnce(
            Job,
            &'a DesktopDataState,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(ArtifactMetadata, Vec<u8>), DataError>>
                    + Send
                    + 'a,
            >,
        >,
    {
        if let Some(existing) = self
            .store
            .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &idempotency_key)
            .await?
        {
            ensure_job_scope(&existing, kind)?;
            if existing.status != JobStatus::Completed {
                return Err(DesktopDataError::InvalidJobState);
            }
            let (metadata, bytes) = self.files.read_artifact_bytes(existing.id).await?;
            return Ok(DataArtifactView::from_parts(metadata, Some(bytes)));
        }
        let now = Utc::now();
        let mut job = Job {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            created_by: LOCAL_USER_ID,
            kind,
            status: JobStatus::Writing,
            idempotency_key,
            progress_current: 0,
            progress_total: Some(1),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        let audit = self
            .audit(match kind {
                JobKind::Export => "create_data_export",
                JobKind::Snapshot => "create_data_snapshot",
                JobKind::Import | JobKind::BulkOperation => "create_data_artifact",
            })
            .await?;
        self.store.create_job(&job, &audit).await?;
        match build(job.clone(), self).await {
            Ok((metadata, bytes)) => {
                self.files.write_artifact(&metadata, &bytes).await?;
                transition_job(
                    &self.store,
                    &mut job,
                    JobStatus::Completed,
                    1,
                    Some(serde_json::to_value(&metadata).map_err(DataError::from)?),
                    None,
                    &audit,
                )
                .await?;
                Ok(DataArtifactView::from_parts(metadata, Some(bytes)))
            }
            Err(error) => {
                let _ = transition_job(
                    &self.store,
                    &mut job,
                    JobStatus::Failed,
                    0,
                    None,
                    Some(json!({ "code": "artifact_failed" })),
                    &audit,
                )
                .await;
                Err(error.into())
            }
        }
    }

    pub(crate) async fn read_artifact(
        &self,
        job_id: &str,
    ) -> Result<DataArtifactView, DesktopDataError> {
        let id = parse_id("job", job_id)?;
        let job = self.store.get_job(id).await?;
        if job.lab_id != LOCAL_LAB_ID
            || job.created_by != LOCAL_USER_ID
            || !matches!(job.kind, JobKind::Export | JobKind::Snapshot)
            || job.status != JobStatus::Completed
        {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let (metadata, bytes) = self.files.read_artifact_bytes(id).await?;
        Ok(DataArtifactView::from_parts(metadata, Some(bytes)))
    }

    pub(crate) fn store_ref(&self) -> &SqliteStore {
        &self.store
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> &SqliteStore {
        self.store_ref()
    }

    pub(crate) fn files_ref(&self) -> &DataFiles {
        &self.files
    }
}

fn exact_local_remap_replay(
    replacement: &Job,
    source: &Job,
    mapping: &FieldMapping,
) -> Result<AnimalImportPreviewResponse, DesktopDataError> {
    if replacement.lab_id != source.lab_id
        || replacement.created_by != source.created_by
        || replacement.project_id != source.project_id
        || replacement.kind != JobKind::Import
        || replacement.status != JobStatus::AwaitingConfirmation
        || replacement.cancellation_requested
    {
        return Err(DataError::Conflict(
            "idempotency key belongs to a different remap request".to_owned(),
        )
        .into());
    }
    let result = replacement
        .result
        .clone()
        .and_then(|value| serde_json::from_value::<ImportRemapJobResult>(value).ok())
        .ok_or_else(|| DataError::Conflict("remap replay state is unavailable".to_owned()))?;
    if result.source_job_id != source.id || result.mapping != *mapping {
        return Err(DataError::Conflict(
            "idempotency key belongs to a different remap request".to_owned(),
        )
        .into());
    }
    Ok(result.preview)
}

async fn transition_job(
    store: &SqliteStore,
    job: &mut Job,
    status: JobStatus,
    progress_current: i64,
    result: Option<Value>,
    error_report: Option<Value>,
    audit: &AuditContext,
) -> Result<(), DesktopDataError> {
    let expected_revision = job.meta.revision;
    job.status = status;
    job.progress_current = progress_current;
    job.result = result;
    job.error_report = error_report;
    job.meta.touch(Utc::now());
    store.update_job(job, expected_revision, audit).await?;
    Ok(())
}

fn ensure_job_scope(job: &Job, kind: JobKind) -> Result<(), DesktopDataError> {
    if job.lab_id == LOCAL_LAB_ID && job.created_by == LOCAL_USER_ID && job.kind == kind {
        Ok(())
    } else {
        Err(DesktopDataError::ScopeMismatch)
    }
}

fn validate_idempotency_key(value: &str) -> Result<(), DesktopDataError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 128
        || value.chars().any(char::is_control)
    {
        Err(DesktopDataError::InvalidIdempotencyKey)
    } else {
        Ok(())
    }
}

fn parse_id(field: &'static str, value: &str) -> Result<Uuid, DesktopDataError> {
    Uuid::parse_str(value).map_err(|_| DesktopDataError::InvalidId(field))
}

fn ensure_local_lab(lab_id: Uuid) -> Result<(), DesktopDataError> {
    if lab_id == LOCAL_LAB_ID {
        Ok(())
    } else {
        Err(DesktopDataError::ScopeMismatch)
    }
}

fn ensure_requested_project(requested: Option<Uuid>, actual: Uuid) -> Result<(), DesktopDataError> {
    if requested.is_none_or(|project_id| project_id == actual) {
        Ok(())
    } else {
        Err(DesktopDataError::ScopeMismatch)
    }
}

fn validate_attachment_file_name(value: String) -> Result<String, DesktopDataError> {
    let value = value.trim().to_owned();
    if value.is_empty()
        || value.len() > MAX_ATTACHMENT_FILE_NAME_BYTES
        || matches!(value.as_str(), "." | "..")
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(StoreError::Validation(
            "attachment file name must be a plain name of at most 255 bytes".to_owned(),
        )
        .into());
    }
    Ok(value)
}

fn validate_attachment_media_type(
    value: Option<String>,
) -> Result<Option<String>, DesktopDataError> {
    let value = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| {
        value.len() > MAX_ATTACHMENT_MEDIA_TYPE_BYTES
            || !value.is_ascii()
            || value.chars().any(char::is_control)
    }) {
        return Err(StoreError::Validation(
            "attachment media type must be printable ASCII of at most 127 bytes".to_owned(),
        )
        .into());
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttachmentTargetInput {
    Project,
    Animal,
    Experiment,
    Measurement,
    Sample,
}

impl AttachmentTargetInput {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Animal => "animal",
            Self::Experiment => "experiment",
            Self::Measurement => "measurement",
            Self::Sample => "sample",
        }
    }

    fn from_stored(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "animal" => Some(Self::Animal),
            "experiment" => Some(Self::Experiment),
            "measurement" => Some(Self::Measurement),
            "sample" => Some(Self::Sample),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AttachmentScopeInput {
    pub entity_type: AttachmentTargetInput,
    pub entity_id: String,
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UploadAttachmentInput {
    pub entity_type: AttachmentTargetInput,
    pub entity_id: String,
    pub project_id: Option<String>,
    pub file_name: String,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentView {
    pub id: String,
    pub project_id: Option<String>,
    pub entity_type: String,
    pub entity_id: String,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    pub version: i32,
    pub content_href: String,
    pub created_at: String,
}

impl From<&Attachment> for AttachmentView {
    fn from(attachment: &Attachment) -> Self {
        Self {
            id: attachment.id.to_string(),
            project_id: attachment.project_id.map(|id| id.to_string()),
            entity_type: attachment.entity_type.clone(),
            entity_id: attachment.entity_id.to_string(),
            file_name: attachment.file_name.clone(),
            media_type: attachment.media_type.clone(),
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256.clone(),
            version: attachment.version,
            content_href: format!("muriarc-ipc://attachments/{}", attachment.id),
            created_at: attachment.meta.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentDownloadView {
    pub metadata: AttachmentView,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreviewDataImportInput {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub idempotency_key: String,
    #[serde(default)]
    pub import_kind: ImportKind,
    pub experiment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RemapDataImportInput {
    pub job_id: String,
    pub mapping: FieldMapping,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfirmDataImportInput {
    pub job_id: String,
    pub preview_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CancelDataImportInput {
    pub job_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateDataExportInput {
    pub format: ExportFormat,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateDataSnapshotInput {
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportReceiptView {
    pub job_id: String,
    pub preview_hash: String,
    pub committed_at: String,
    pub replayed: bool,
    pub counts: ImportCountsView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCountsView {
    animals: usize,
    animal_events: usize,
    genotypes: usize,
    pedigrees: usize,
    measurements: usize,
}

impl ImportReceiptView {
    fn from_result(job_id: Uuid, result: muriarc_core::ImportCommitResult) -> Self {
        Self {
            job_id: job_id.to_string(),
            preview_hash: result.preview_hash,
            committed_at: result.committed_at.to_rfc3339(),
            replayed: result.replayed,
            counts: ImportCountsView {
                animals: result.counts.animals,
                animal_events: result.counts.animal_events,
                genotypes: result.counts.genotypes,
                pedigrees: result.counts.pedigrees,
                measurements: result.counts.measurements,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataArtifactView {
    job_id: String,
    kind: &'static str,
    file_name: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<Vec<u8>>,
}

impl DataArtifactView {
    fn from_parts(metadata: ArtifactMetadata, bytes: Option<Vec<u8>>) -> Self {
        Self {
            job_id: metadata.job_id.to_string(),
            kind: match metadata.kind {
                ArtifactKind::Export => "export",
                ArtifactKind::Snapshot => "snapshot",
            },
            file_name: metadata.file_name,
            media_type: metadata.media_type,
            size_bytes: metadata.size_bytes,
            sha256: metadata.sha256,
            bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use muriarc_core::{
        Actor, Animal, AnimalFilter, AuditContext, AuditFilter, EntityType, Experiment,
        ExperimentTemplateVersion, FieldValueType, JobStatus, MeasurementFilter, MuriArcStore,
        Participation, Project, ProvenanceFilter, ProvenanceSource, RecordStatus, Sex,
        TemplateField, WriteSource,
    };
    use muriarc_snapshot::verify_bundle;
    use tempfile::tempdir;

    use super::*;
    use crate::application::DesktopState;

    #[tokio::test]
    async fn preview_confirm_export_and_snapshot_form_a_real_local_vertical_flow() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();

        let preview = state
            .preview_import(PreviewDataImportInput {
                file_name: "animals.csv".to_owned(),
                bytes: b"display_id,sex\nM-IMPORT-1,female\n".to_vec(),
                idempotency_key: "desktop-import-1".to_owned(),
                import_kind: ImportKind::Animal,
                experiment_id: None,
            })
            .await
            .unwrap();
        assert_eq!(preview.import_kind, ImportKind::Animal);
        assert!(preview.can_confirm);
        let receipt = state
            .confirm_import(ConfirmDataImportInput {
                job_id: preview.job_id.to_string(),
                preview_hash: preview.preview_hash,
            })
            .await
            .unwrap();
        assert_eq!(receipt.counts.animals, 1);
        let replayed = state
            .confirm_import(ConfirmDataImportInput {
                job_id: receipt.job_id.clone(),
                preview_hash: receipt.preview_hash.clone(),
            })
            .await
            .unwrap();
        assert!(replayed.replayed);
        assert_eq!(replayed.counts.animals, 1);
        assert_eq!(
            state
                .store()
                .list_animals(&AnimalFilter {
                    lab_id: LOCAL_LAB_ID,
                    ..AnimalFilter::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            state
                .store()
                .get_job(Uuid::parse_str(&receipt.job_id).unwrap())
                .await
                .unwrap()
                .status,
            JobStatus::Completed
        );

        let exported = state
            .create_export(CreateDataExportInput {
                format: ExportFormat::Csv,
                idempotency_key: "desktop-export-1".to_owned(),
            })
            .await
            .unwrap();
        assert!(
            String::from_utf8(exported.bytes.unwrap())
                .unwrap()
                .contains("M-IMPORT-1")
        );

        let imported_animal = state
            .store()
            .list_animals(&AnimalFilter {
                lab_id: LOCAL_LAB_ID,
                ..AnimalFilter::default()
            })
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let attachment = state
            .upload_attachment(UploadAttachmentInput {
                entity_type: AttachmentTargetInput::Animal,
                entity_id: imported_animal.id.to_string(),
                project_id: None,
                file_name: "observation.txt".to_owned(),
                media_type: Some("text/plain".to_owned()),
                bytes: b"snapshot attachment".to_vec(),
            })
            .await
            .unwrap();

        let snapshot = state
            .create_snapshot(CreateDataSnapshotInput {
                idempotency_key: "desktop-snapshot-1".to_owned(),
            })
            .await
            .unwrap();
        let manifest = verify_bundle(Cursor::new(snapshot.bytes.unwrap())).unwrap();
        assert_eq!(manifest.lab_id, LOCAL_LAB_ID);
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.path == "data/animal.jsonl")
        );
        assert!(manifest.entries.iter().any(|entry| {
            entry.path == format!("attachments/{}/v1/content", attachment.id)
                && entry.size_bytes == b"snapshot attachment".len() as u64
                && entry.sha256 == attachment.sha256
        }));
    }

    #[tokio::test]
    async fn local_artifact_read_rejects_same_length_tampering_without_returning_bytes() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let exported = state
            .create_export(CreateDataExportInput {
                format: ExportFormat::Csv,
                idempotency_key: "desktop-tampered-export".to_owned(),
            })
            .await
            .unwrap();
        let job_id = Uuid::parse_str(&exported.job_id).unwrap();
        assert!(exported.size_bytes > 0);
        let mut tampered = exported.bytes.unwrap();
        tampered[0] ^= 0xff;
        tokio::fs::write(
            temp.path()
                .join("data")
                .join("artifacts")
                .join(format!("{job_id}.bin")),
            &tampered,
        )
        .await
        .unwrap();

        let error = state
            .read_artifact(&job_id.to_string())
            .await
            .expect_err("tampered artifact must not produce a DataArtifactView");
        assert!(matches!(
            error,
            DesktopDataError::Data(DataError::ChecksumMismatch("artifact"))
        ));
        assert_eq!(error.code(), "storage_error");
    }

    #[tokio::test]
    async fn local_attachments_are_versioned_verified_scoped_and_atomic() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let now = Utc::now();
        let audit = state.audit("attachment_test_fixture").await.unwrap();
        let animal = Animal::new_mouse(LOCAL_LAB_ID, "M-ATTACH-1", Sex::Female, now).unwrap();
        state.store().create_animal(&animal, &audit).await.unwrap();

        let empty = state
            .upload_attachment(UploadAttachmentInput {
                entity_type: AttachmentTargetInput::Animal,
                entity_id: animal.id.to_string(),
                project_id: None,
                file_name: "empty.bin".to_owned(),
                media_type: None,
                bytes: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(empty.size_bytes, 0);
        assert_eq!(
            empty.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(
            state
                .download_attachment(&empty.id)
                .await
                .unwrap()
                .bytes
                .is_empty()
        );

        let first = state
            .upload_attachment(UploadAttachmentInput {
                entity_type: AttachmentTargetInput::Animal,
                entity_id: animal.id.to_string(),
                project_id: None,
                file_name: "result.txt".to_owned(),
                media_type: Some("text/plain".to_owned()),
                bytes: b"version one".to_vec(),
            })
            .await
            .unwrap();
        let second = state
            .upload_attachment(UploadAttachmentInput {
                entity_type: AttachmentTargetInput::Animal,
                entity_id: animal.id.to_string(),
                project_id: None,
                file_name: "result.txt".to_owned(),
                media_type: Some("text/plain".to_owned()),
                bytes: b"version two".to_vec(),
            })
            .await
            .unwrap();
        assert_eq!((first.version, second.version), (1, 2));
        assert_eq!(
            state
                .list_attachments(AttachmentScopeInput {
                    entity_type: AttachmentTargetInput::Animal,
                    entity_id: animal.id.to_string(),
                    project_id: None,
                })
                .await
                .unwrap()
                .len(),
            3
        );

        let first_id = Uuid::parse_str(&first.id).unwrap();
        let provenance = state
            .store()
            .list_provenance(&ProvenanceFilter {
                lab_id: LOCAL_LAB_ID,
                entity_type: Some(EntityType::Attachment),
                entity_id: Some(first_id),
                ..ProvenanceFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(provenance[0].source, ProvenanceSource::Human);
        assert_eq!(provenance[0].actor_user_id, Some(LOCAL_USER_ID));
        assert_eq!(
            state
                .store()
                .list_audit_entries(&AuditFilter {
                    lab_id: LOCAL_LAB_ID,
                    project_id: None,
                    entity_id: Some(first_id),
                })
                .await
                .unwrap()
                .len(),
            1
        );

        let before_unknown = state
            .store()
            .list_lab_attachments(LOCAL_LAB_ID)
            .await
            .unwrap()
            .len();
        let unknown = state
            .upload_attachment(UploadAttachmentInput {
                entity_type: AttachmentTargetInput::Animal,
                entity_id: Uuid::new_v4().to_string(),
                project_id: None,
                file_name: "unknown.bin".to_owned(),
                media_type: None,
                bytes: b"must not be written".to_vec(),
            })
            .await;
        assert!(matches!(
            unknown,
            Err(DesktopDataError::Store(StoreError::NotFound { .. }))
        ));
        assert_eq!(
            state
                .store()
                .list_lab_attachments(LOCAL_LAB_ID)
                .await
                .unwrap()
                .len(),
            before_unknown
        );

        let stored = state.store().get_attachment(first_id).await.unwrap();
        tokio::fs::write(
            state.attachments.root().join(&stored.relative_path),
            b"polluted content",
        )
        .await
        .unwrap();
        assert!(matches!(
            state.download_attachment(&first.id).await,
            Err(DesktopDataError::Attachment(AttachmentFileError::Integrity))
        ));
        let snapshot_error = state
            .create_snapshot(CreateDataSnapshotInput {
                idempotency_key: "snapshot-with-polluted-attachment".to_owned(),
            })
            .await
            .err()
            .unwrap();
        assert_eq!(snapshot_error.code(), "storage_error");

        let orphan_id = Uuid::new_v4();
        let orphan = state
            .attachments
            .write_bytes(orphan_id, b"orphan candidate")
            .await
            .unwrap();
        let invalid_metadata = Attachment {
            id: orphan_id,
            lab_id: LOCAL_LAB_ID,
            project_id: Some(Uuid::new_v4()),
            entity_type: "animal".to_owned(),
            entity_id: animal.id,
            file_name: "orphan.bin".to_owned(),
            media_type: None,
            relative_path: orphan.relative_path.clone(),
            size_bytes: orphan.size_bytes,
            sha256: orphan.sha256.clone(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        assert!(
            state
                .commit_attachment(&invalid_metadata, &orphan, &audit)
                .await
                .is_err()
        );
        assert!(!orphan.absolute_path.exists());
    }

    #[tokio::test]
    async fn manual_remap_replaces_the_pending_job_and_only_new_preview_confirms() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let original = state
            .preview_import(PreviewDataImportInput {
                file_name: "animals.csv".to_owned(),
                bytes: b"custom_code,gender\nM-REMAP-1,F\n".to_vec(),
                idempotency_key: "desktop-remap-source".to_owned(),
                import_kind: ImportKind::Animal,
                experiment_id: None,
            })
            .await
            .unwrap();
        assert!(!original.can_confirm);
        let mapping = FieldMapping {
            columns: std::collections::BTreeMap::from([
                ("display_id".to_owned(), "custom_code".to_owned()),
                ("sex".to_owned(), "gender".to_owned()),
            ]),
        };
        let remapped = state
            .remap_import(RemapDataImportInput {
                job_id: original.job_id.to_string(),
                mapping: mapping.clone(),
                idempotency_key: "desktop-remap-replacement".to_owned(),
            })
            .await
            .unwrap();
        assert!(remapped.can_confirm, "{:?}", remapped.issues);
        assert_ne!(remapped.job_id, original.job_id);
        assert_ne!(remapped.preview_hash, original.preview_hash);
        let old_job = state.store().get_job(original.job_id).await.unwrap();
        assert_eq!(old_job.status, JobStatus::Cancelled);
        assert!(old_job.cancellation_requested);

        let replayed = state
            .remap_import(RemapDataImportInput {
                job_id: original.job_id.to_string(),
                mapping: mapping.clone(),
                idempotency_key: "desktop-remap-replacement".to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(replayed.job_id, remapped.job_id);
        assert_eq!(replayed.preview_hash, remapped.preview_hash);
        assert!(
            state
                .remap_import(RemapDataImportInput {
                    job_id: original.job_id.to_string(),
                    mapping: FieldMapping {
                        columns: std::collections::BTreeMap::from([(
                            "display_id".to_owned(),
                            "gender".to_owned(),
                        )]),
                    },
                    idempotency_key: "desktop-remap-replacement".to_owned(),
                })
                .await
                .is_err()
        );
        assert!(
            state
                .confirm_import(ConfirmDataImportInput {
                    job_id: original.job_id.to_string(),
                    preview_hash: original.preview_hash,
                })
                .await
                .is_err()
        );
        let receipt = state
            .confirm_import(ConfirmDataImportInput {
                job_id: remapped.job_id.to_string(),
                preview_hash: remapped.preview_hash,
            })
            .await
            .unwrap();
        assert_eq!(receipt.counts.animals, 1);
    }

    #[tokio::test]
    async fn failed_remap_keeps_the_original_pending_job_confirmable() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let original = state
            .preview_import(PreviewDataImportInput {
                file_name: "animals.csv".to_owned(),
                bytes: b"display_id\nM-SAFE-1\n".to_vec(),
                idempotency_key: "desktop-remap-safe-source".to_owned(),
                import_kind: ImportKind::Animal,
                experiment_id: None,
            })
            .await
            .unwrap();
        state.files.clear_upload(original.job_id).await.unwrap();
        assert!(
            state
                .remap_import(RemapDataImportInput {
                    job_id: original.job_id.to_string(),
                    mapping: original.mapping.clone(),
                    idempotency_key: "desktop-remap-failing-replacement".to_owned(),
                })
                .await
                .is_err()
        );
        let preserved = state.store().get_job(original.job_id).await.unwrap();
        assert_eq!(preserved.status, JobStatus::AwaitingConfirmation);
        assert!(!preserved.cancellation_requested);
        assert!(
            state
                .files
                .read_pending_import(original.job_id)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_blocking_preview_never_writes_animals() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let preview = state
            .preview_import(PreviewDataImportInput {
                file_name: "animals.csv".to_owned(),
                bytes: b"display_id,birth_date\nM-BAD,not-a-date\n".to_vec(),
                idempotency_key: "desktop-import-bad".to_owned(),
                import_kind: ImportKind::Animal,
                experiment_id: None,
            })
            .await
            .unwrap();
        assert_eq!(preview.import_kind, ImportKind::Animal);
        assert!(!preview.can_confirm);
        assert!(
            state
                .confirm_import(ConfirmDataImportInput {
                    job_id: preview.job_id.to_string(),
                    preview_hash: preview.preview_hash,
                })
                .await
                .is_err()
        );
        assert!(
            state
                .store()
                .list_animals(&AnimalFilter {
                    lab_id: LOCAL_LAB_ID,
                    ..AnimalFilter::default()
                })
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn measurement_import_requires_an_experiment_and_commits_drafts() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("muriarc.sqlite3");
        let _domain = DesktopState::initialize(&database).await.unwrap();
        let state = DesktopDataState::initialize(&database, temp.path())
            .await
            .unwrap();
        let now = Utc::now();
        let audit = AuditContext {
            actor: Actor::human(LOCAL_USER_ID, "本地操作者"),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("measurement import fixture".to_owned()),
        };
        let project = Project::new(LOCAL_LAB_ID, "DEMO", now).unwrap();
        state
            .store()
            .create_project(&project, &audit)
            .await
            .unwrap();
        let mut template = ExperimentTemplateVersion::draft(
            LOCAL_LAB_ID,
            "demo-measurements",
            1,
            "DEMO measurements",
            now,
        )
        .unwrap();
        template
            .replace_fields(
                vec![TemplateField {
                    key: "body_weight".to_owned(),
                    label: "体重".to_owned(),
                    value_type: FieldValueType::Number,
                    unit: Some("g".to_owned()),
                    required: true,
                    categories: Vec::new(),
                    minimum: Some(0.0),
                    maximum: None,
                    display_order: 0,
                    ai_writable: true,
                }],
                now,
            )
            .unwrap();
        state
            .store()
            .create_template_version(&template, &audit)
            .await
            .unwrap();
        let published = state
            .store()
            .publish_template_version(
                template.id,
                template.meta.revision,
                LOCAL_USER_ID,
                now,
                &audit,
            )
            .await
            .unwrap();
        let mut experiment = Experiment::new(LOCAL_LAB_ID, project.id, "DEMO-001", now).unwrap();
        experiment.template_version_id = Some(published.id);
        state
            .store()
            .create_experiment(&experiment, &audit)
            .await
            .unwrap();
        let animal = Animal::new_mouse(LOCAL_LAB_ID, "M-MEASURE-1", Sex::Female, now).unwrap();
        state.store().create_animal(&animal, &audit).await.unwrap();
        let participation = Participation::enroll(experiment.id, animal.id, now);
        state
            .store()
            .create_participation(&participation, &audit)
            .await
            .unwrap();

        let original = state
            .preview_import(PreviewDataImportInput {
                file_name: "measurements.csv".to_owned(),
                bytes: b"mouse,metric,kind,result,result_unit,when\nM-MEASURE-1,body_weight,number,23.5,g,2026-07-19T08:30:00Z\n".to_vec(),
                idempotency_key: "desktop-measurement-import-1".to_owned(),
                import_kind: ImportKind::Measurement,
                experiment_id: Some(experiment.id.to_string()),
            })
            .await
            .unwrap();
        assert!(!original.can_confirm);
        let preview = state
            .remap_import(RemapDataImportInput {
                job_id: original.job_id.to_string(),
                mapping: FieldMapping {
                    columns: std::collections::BTreeMap::from([
                        ("display_id".to_owned(), "mouse".to_owned()),
                        ("measurement_key".to_owned(), "metric".to_owned()),
                        ("value_type".to_owned(), "kind".to_owned()),
                        ("value".to_owned(), "result".to_owned()),
                        ("unit".to_owned(), "result_unit".to_owned()),
                        ("measured_at".to_owned(), "when".to_owned()),
                    ]),
                },
                idempotency_key: "desktop-measurement-remap-1".to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(preview.import_kind, ImportKind::Measurement);
        assert!(preview.can_confirm, "{:?}", preview.issues);
        assert_eq!(preview.experiment_id, Some(experiment.id));
        let receipt = state
            .confirm_import(ConfirmDataImportInput {
                job_id: preview.job_id.to_string(),
                preview_hash: preview.preview_hash,
            })
            .await
            .unwrap();
        assert_eq!(receipt.counts.measurements, 1);
        assert_eq!(receipt.counts.animals, 0);
        let measurements = state
            .store()
            .list_measurements(&MeasurementFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal.id),
            })
            .await
            .unwrap();
        assert_eq!(measurements.len(), 1);
        assert_eq!(measurements[0].status, RecordStatus::Draft);
        assert_eq!(measurements[0].unit.as_deref(), Some("g"));
    }
}
