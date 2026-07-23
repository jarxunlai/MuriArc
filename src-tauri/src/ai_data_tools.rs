use async_trait::async_trait;
use chrono::{Duration, Utc};
use muriarc_ai::{
    AiDataAccessContext, AiDataApplyResult, AiDataToolBackend, AiExportArtifactView,
    AiExportFormat, AiExportResource, AiSourceImportKind, Citation, DomainToolOutput,
    DomainToolRequest, DraftKind, ExportCreateArguments, FieldChange, ImportCommitDraftArguments,
    ImportCommitDraftPayload, ImportDraftPreviewSummary, ImportPreviewArguments, ProposalActor,
    SOURCE_IMPORT_JOB_BINDING_KEY, SourceImportJobBinding, SourceImportPreviewArguments,
    ToolExecutionError, ToolName, WriteDraft, valid_sha256,
};
use muriarc_core::{
    Actor, ActorType, AiConversationSourceStatus, AiImportResolution,
    ApprovalDecision as StoredApprovalDecision, AuditContext, EntityType, ImportCommitResult,
    ImportSourceArchive, Job, JobKind, JobStatus, LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore,
    ProvenanceFilter, RecordMeta, StoreError, ToolRunStatus, WriteSource, canonical_import_receipt,
    completed_ai_import_tool_run,
};
use muriarc_data::{
    AiSourceImportValidationError, AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata,
    DataError, ExportFormat, ImportConfirmOptions, ImportKind, ai_source_import_idempotency_key,
    artifact_metadata, export_animals_scoped, validate_ai_source_import,
};
use muriarc_importer::AnimalExportFilter;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::data::DesktopDataState;

const IMPORT_PREVIEW_TTL: Duration = Duration::hours(24);
const MAX_PREVIEW_ISSUES: usize = 50;

/// Bounded local adapter for AI-assisted import previews/drafts and project exports.
///
/// It intentionally has no API for uploads, arbitrary files, URLs, snapshot creation,
/// raw bytes or SQL. Upload parsing and import confirmation remain owned by the ordinary
/// `DataFiles` workflow.
#[derive(Clone)]
pub(crate) struct DesktopAiDataTools {
    data: DesktopDataState,
}

impl DesktopAiDataTools {
    pub(crate) fn new(data: DesktopDataState) -> Self {
        Self { data }
    }

    fn ensure_local_access(&self, access: &AiDataAccessContext) -> Result<(), ToolExecutionError> {
        if access.lab_id() == LOCAL_LAB_ID && access.user_id() == LOCAL_USER_ID {
            Ok(())
        } else {
            Err(rejected("data_tool_forbidden"))
        }
    }

    async fn execute_source_import_preview(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        self.ensure_local_access(access)?;
        let arguments: SourceImportPreviewArguments = parse_arguments(request.arguments)?;
        let conversation_id = access
            .conversation_id()
            .filter(|value| !value.is_nil())
            .ok_or_else(|| rejected("source_import_unavailable"))?;
        if arguments.source_id.is_nil() {
            return Err(rejected("invalid_arguments"));
        }

        let (import_kind, project_id, experiment_id) = match arguments.import_kind {
            AiSourceImportKind::Animal => return Err(rejected("source_import_forbidden")),
            AiSourceImportKind::Measurement => {
                let experiment_id = arguments
                    .experiment_id
                    .filter(|value| !value.is_nil())
                    .ok_or_else(|| rejected("invalid_arguments"))?;
                let experiment = self
                    .data
                    .store_ref()
                    .get_experiment(experiment_id)
                    .await
                    .map_err(|_| rejected("source_import_forbidden"))?;
                if experiment.lab_id != LOCAL_LAB_ID
                    || access.conversation_project_id() != Some(experiment.project_id)
                    || !access.can_import_project(experiment.project_id)
                {
                    return Err(rejected("source_import_forbidden"));
                }
                (
                    ImportKind::Measurement,
                    Some(experiment.project_id),
                    Some(experiment_id),
                )
            }
        };

        let source = self
            .data
            .store_ref()
            .get_ai_conversation_source(arguments.source_id)
            .await
            .map_err(|_| rejected("source_import_source_unavailable"))?;
        let attachment = self
            .data
            .store_ref()
            .get_attachment(source.attachment_id)
            .await
            .map_err(|_| rejected("source_import_source_unavailable"))?;
        let bytes = self
            .data
            .attachments_ref()
            .read_verified_bytes(&attachment)
            .await
            .map_err(|_| rejected("source_import_invalid_material"))?;
        if bytes.len() as u64 > self.data.files_ref().max_upload_bytes() {
            return Err(rejected("source_import_invalid_file"));
        }
        let validated = validate_ai_source_import(
            &source,
            &attachment,
            &bytes,
            LOCAL_LAB_ID,
            LOCAL_USER_ID,
            conversation_id,
            project_id,
            import_kind,
            Utc::now(),
        )
        .map_err(map_source_validation_error)?;

        let idempotency_key =
            ai_source_import_idempotency_key(source.id, import_kind, experiment_id);
        let binding = SourceImportJobBinding::new(
            source.id,
            source.meta.revision,
            source.project_id,
            attachment.id,
            attachment.meta.revision,
            conversation_id,
        );
        if let Some(existing) = self
            .data
            .store_ref()
            .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &idempotency_key)
            .await
            .map_err(map_store_error)?
        {
            ensure_source_import_job(
                &existing,
                access,
                project_id,
                &idempotency_key,
                &binding,
                false,
            )?;
            return self.source_import_output(&existing, &binding).await;
        }

        let audit = ai_audit(request.tool_run_id, "ai_source_import_preview");
        let mut job = Job {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id,
            created_by: LOCAL_USER_ID,
            kind: JobKind::Import,
            status: JobStatus::Parsing,
            idempotency_key: idempotency_key.clone(),
            progress_current: 0,
            progress_total: Some(3),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(Utc::now()),
        };
        if let Err(error) = self.data.store_ref().create_job(&job, &audit).await {
            if matches!(error, StoreError::Conflict(_))
                && let Some(existing) = self
                    .data
                    .store_ref()
                    .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &idempotency_key)
                    .await
                    .map_err(map_store_error)?
            {
                ensure_source_import_job(
                    &existing,
                    access,
                    project_id,
                    &idempotency_key,
                    &binding,
                    false,
                )?;
                return self.source_import_output(&existing, &binding).await;
            }
            return Err(map_store_error(error));
        }
        ensure_source_import_job(&job, access, project_id, &idempotency_key, &binding, true)?;

        let operation = async {
            self.data
                .files_ref()
                .write_upload_bytes(job.id, &validated.file_name, &bytes)
                .await?;
            let preview: AnimalImportPreviewResponse = match import_kind {
                ImportKind::Animal => (&self
                    .data
                    .files_ref()
                    .preview_animal_import(&job, self.data.store_ref())
                    .await?)
                    .into(),
                ImportKind::Measurement => (&self
                    .data
                    .files_ref()
                    .preview_measurement_import(
                        &job,
                        experiment_id.expect("measurement source import has an experiment"),
                        self.data.store_ref(),
                    )
                    .await?)
                    .into(),
            };
            Ok::<_, DataError>(preview)
        }
        .await;

        match operation {
            Ok(preview) => {
                let result = source_import_job_result(&preview, &binding)?;
                if transition_job(
                    self.data.store_ref(),
                    &mut job,
                    JobStatus::AwaitingConfirmation,
                    2,
                    Some(result),
                    None,
                    &audit,
                )
                .await
                .is_err()
                {
                    self.cleanup_failed_source_import(job.id, &audit, "storage_error")
                        .await;
                    return Err(ToolExecutionError::Unavailable);
                }
                self.source_import_output(&job, &binding).await
            }
            Err(error) => {
                let code = source_import_data_error_code(&error);
                let _ = transition_job(
                    self.data.store_ref(),
                    &mut job,
                    JobStatus::Failed,
                    0,
                    None,
                    Some(json!({"code": code})),
                    &audit,
                )
                .await;
                let _ = self.data.files_ref().clear_pending_import(job.id).await;
                let _ = self.data.files_ref().clear_upload(job.id).await;
                Err(map_source_import_data_error(error))
            }
        }
    }

    async fn source_import_output(
        &self,
        job: &Job,
        expected_binding: &SourceImportJobBinding,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let stored_binding = source_import_binding(job)?;
        if &stored_binding != expected_binding {
            return Err(rejected("source_import_job_conflict"));
        }
        let (_, mut preview) = self.pending_preview(job).await?;
        let preview_object = preview
            .as_object_mut()
            .ok_or(ToolExecutionError::Unavailable)?;
        preview_object.insert("source_id".to_owned(), json!(stored_binding.source_id));
        Ok(DomainToolOutput::read(
            preview,
            vec![
                Citation::new(
                    EntityType::AiConversationSource,
                    stored_binding.source_id,
                    Some(stored_binding.source_revision),
                ),
                Citation::new(EntityType::Job, job.id, Some(job.meta.revision)),
            ],
        ))
    }

    async fn cleanup_failed_source_import(
        &self,
        job_id: Uuid,
        audit: &AuditContext,
        code: &'static str,
    ) {
        let _ = self.data.files_ref().clear_pending_import(job_id).await;
        let _ = self.data.files_ref().clear_upload(job_id).await;
        if let Ok(mut current) = self.data.store_ref().get_job(job_id).await
            && !matches!(
                current.status,
                JobStatus::AwaitingConfirmation | JobStatus::Completed | JobStatus::Failed
            )
        {
            let _ = transition_job(
                self.data.store_ref(),
                &mut current,
                JobStatus::Failed,
                0,
                None,
                Some(json!({"code": code})),
                audit,
            )
            .await;
        }
    }

    async fn import_job(
        &self,
        access: &AiDataAccessContext,
        job_id: Uuid,
    ) -> Result<Job, ToolExecutionError> {
        self.ensure_local_access(access)?;
        let job = self
            .data
            .store_ref()
            .get_job(job_id)
            .await
            .map_err(map_import_job_lookup)?;
        if job.lab_id != LOCAL_LAB_ID
            || job.created_by != LOCAL_USER_ID
            || job.kind != JobKind::Import
        {
            return Err(rejected("import_job_not_found"));
        }
        let project_id = job
            .project_id
            .filter(|project_id| {
                access.conversation_project_id() == Some(*project_id)
                    && access.can_import_project(*project_id)
            })
            .ok_or_else(|| rejected("import_job_not_found"))?;
        let project = self
            .data
            .store_ref()
            .get_project(project_id)
            .await
            .map_err(|_| rejected("import_job_not_found"))?;
        if project.lab_id != LOCAL_LAB_ID {
            return Err(rejected("import_job_not_found"));
        }
        Ok(job)
    }

    fn ensure_pending_and_fresh(&self, job: &Job) -> Result<(), ToolExecutionError> {
        if job.status != JobStatus::AwaitingConfirmation || job.cancellation_requested {
            return Err(rejected("import_not_awaiting_confirmation"));
        }
        let now = Utc::now();
        let expires_at = job.meta.created_at + IMPORT_PREVIEW_TTL;
        if job.meta.created_at > now || now >= expires_at {
            return Err(rejected("import_preview_expired"));
        }
        Ok(())
    }

    async fn pending_preview(&self, job: &Job) -> Result<(String, Value), ToolExecutionError> {
        self.ensure_pending_and_fresh(job)?;
        let expires_at = job.meta.created_at + IMPORT_PREVIEW_TTL;
        let response: AnimalImportPreviewResponse = match job.project_id {
            Some(project_id) => {
                let pending = self
                    .data
                    .files_ref()
                    .read_pending_measurement_import(job.id)
                    .await
                    .map_err(map_data_error)?;
                if pending.lab_id != LOCAL_LAB_ID
                    || pending.created_by != LOCAL_USER_ID
                    || pending.project_id != project_id
                    || pending.job_id != job.id
                {
                    return Err(rejected("import_job_not_found"));
                }
                (&pending).into()
            }
            None => {
                let pending = self
                    .data
                    .files_ref()
                    .read_pending_import(job.id)
                    .await
                    .map_err(map_data_error)?;
                if pending.lab_id != LOCAL_LAB_ID
                    || pending.created_by != LOCAL_USER_ID
                    || pending.project_id.is_some()
                    || pending.job_id != job.id
                {
                    return Err(rejected("import_job_not_found"));
                }
                (&pending).into()
            }
        };
        let preview_hash = response.preview_hash.clone();
        let issue_count = response.issues.len();
        let preview_row_count = response.preview_rows.len();
        let issues = response
            .issues
            .into_iter()
            .take(MAX_PREVIEW_ISSUES)
            .collect::<Vec<_>>();
        Ok((
            preview_hash.clone(),
            json!({
                "job_id": job.id,
                "job_revision": job.meta.revision,
                "project_id": job.project_id,
                "import_kind": response.import_kind,
                "experiment_id": response.experiment_id,
                "file_name": response.file_name,
                "sheet_name": response.sheet_name,
                "headers": response.headers,
                "mapping": response.mapping,
                "preview_hash": preview_hash,
                "total_rows": response.total_rows,
                "accepted_rows": response.accepted_rows,
                "preview_rows": response.preview_rows,
                "preview_rows_truncated": response.accepted_rows > preview_row_count,
                "can_confirm": response.can_confirm,
                "issue_count": issue_count,
                "issues": issues,
                "issues_truncated": issue_count > MAX_PREVIEW_ISSUES,
                "expires_at": expires_at,
            }),
        ))
    }

    async fn execute_import_preview(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ImportPreviewArguments = parse_arguments(request.arguments)?;
        let job = self.import_job(access, arguments.job_id).await?;
        let (_, preview) = self.pending_preview(&job).await?;
        Ok(DomainToolOutput::read(
            preview,
            vec![Citation::new(
                EntityType::Job,
                job.id,
                Some(job.meta.revision),
            )],
        ))
    }

    async fn execute_import_commit_draft(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ImportCommitDraftArguments = parse_arguments(request.arguments)?;
        if arguments.expected_revision < 1 || !valid_sha256(&arguments.preview_hash) {
            return Err(rejected("invalid_import_binding"));
        }
        let job = self.import_job(access, arguments.job_id).await?;
        if job.meta.revision != arguments.expected_revision {
            return Err(rejected("import_revision_conflict"));
        }
        let (preview_hash, preview) = self.pending_preview(&job).await?;
        if !preview_hash.eq_ignore_ascii_case(arguments.preview_hash.trim()) {
            return Err(rejected("import_preview_hash_conflict"));
        }
        if preview.get("can_confirm").and_then(Value::as_bool) != Some(true) {
            return Err(rejected("import_preview_blocked"));
        }
        let preview_summary = ImportDraftPreviewSummary::from_public_preview(&preview)?;
        if Some(preview_summary.project_id) != job.project_id {
            return Err(rejected("invalid_import_preview"));
        }

        let now = Utc::now();
        let payload = ImportCommitDraftPayload {
            operation: ImportCommitDraftPayload::OPERATION.to_owned(),
            job_id: job.id,
            preview_hash: preview_hash.clone(),
            expected_revision: job.meta.revision,
            preview: preview_summary,
        };
        let draft = WriteDraft::new(
            DraftKind::BulkImport,
            ToolName::ImportCommitDraft,
            ProposalActor::Ai {
                user_id: request.user_id,
                tool_run_id: request.tool_run_id,
            },
            job.project_id,
            vec![FieldChange {
                path: format!("/data/imports/{}", job.id),
                before: Some(json!({
                    "status": "awaiting_confirmation",
                    "revision": job.meta.revision,
                    "preview_hash": preview_hash,
                })),
                after: Some(json!({"status": "completed"})),
            }],
            serde_json::to_value(payload).map_err(|_| rejected("invalid_import_binding"))?,
            now,
            job.meta.created_at + IMPORT_PREVIEW_TTL,
        )
        .map_err(|_| rejected("invalid_import_draft"))?;
        Ok(DomainToolOutput::write_draft(
            draft,
            vec![Citation::new(
                EntityType::Job,
                job.id,
                Some(job.meta.revision),
            )],
        ))
    }

    async fn execute_export(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ExportCreateArguments = parse_arguments(request.arguments)?;
        if arguments.resource != AiExportResource::Animals
            || !access.can_export_project(arguments.project_id)
        {
            return Err(rejected("project_export_forbidden"));
        }
        let project = self
            .data
            .store_ref()
            .get_project(arguments.project_id)
            .await
            .map_err(|_| rejected("project_export_forbidden"))?;
        if project.lab_id != LOCAL_LAB_ID {
            return Err(rejected("project_export_forbidden"));
        }
        let format = match arguments.format {
            AiExportFormat::Csv => ExportFormat::Csv,
            AiExportFormat::Xlsx => ExportFormat::Xlsx,
        };
        let idempotency_key = format!("ai-export:{}", request.tool_run_id);
        if let Some(job) = self
            .data
            .store_ref()
            .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &idempotency_key)
            .await
            .map_err(map_store_error)?
        {
            return self.export_replay(&project, job).await;
        }

        let now = Utc::now();
        let audit = ai_audit(request.tool_run_id, "ai_project_export");
        let mut job = Job {
            id: Uuid::new_v4(),
            lab_id: LOCAL_LAB_ID,
            project_id: Some(project.id),
            created_by: LOCAL_USER_ID,
            kind: JobKind::Export,
            status: JobStatus::Writing,
            idempotency_key,
            progress_current: 0,
            progress_total: Some(1),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        if let Err(error) = self.data.store_ref().create_job(&job, &audit).await {
            if matches!(error, StoreError::Conflict(_))
                && let Some(existing) = self
                    .data
                    .store_ref()
                    .find_job_by_idempotency(LOCAL_LAB_ID, LOCAL_USER_ID, &job.idempotency_key)
                    .await
                    .map_err(map_store_error)?
            {
                return self.export_replay(&project, existing).await;
            }
            return Err(map_store_error(error));
        }

        let operation = async {
            let bytes = export_animals_scoped(
                self.data.store_ref(),
                LOCAL_LAB_ID,
                Some(project.id),
                format,
                &AnimalExportFilter::default(),
            )
            .await?;
            let metadata = artifact_metadata(
                job.id,
                ArtifactKind::Export,
                format!(
                    "muriarc-animals-{}.{}",
                    job.meta.created_at.format("%Y%m%d-%H%M%S"),
                    format.extension()
                ),
                format.media_type().to_owned(),
                &bytes,
                job.meta.created_at,
            )?;
            self.data
                .files_ref()
                .write_artifact(&metadata, &bytes)
                .await?;
            Ok::<_, DataError>(metadata)
        }
        .await;

        match operation {
            Ok(artifact) => {
                if transition_job(
                    self.data.store_ref(),
                    &mut job,
                    JobStatus::Completed,
                    1,
                    Some(
                        serde_json::to_value(&artifact)
                            .map_err(|_| ToolExecutionError::Unavailable)?,
                    ),
                    None,
                    &audit,
                )
                .await
                .is_err()
                {
                    let _ = self.data.files_ref().clear_artifact(job.id).await;
                    return Err(ToolExecutionError::Unavailable);
                }
                export_output(project.id, project.meta.revision, &job, artifact)
            }
            Err(_) => {
                let _ = transition_job(
                    self.data.store_ref(),
                    &mut job,
                    JobStatus::Failed,
                    0,
                    None,
                    Some(json!({"code": "artifact_failed"})),
                    &audit,
                )
                .await;
                Err(ToolExecutionError::Unavailable)
            }
        }
    }

    async fn export_replay(
        &self,
        project: &muriarc_core::Project,
        job: Job,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        if job.lab_id != LOCAL_LAB_ID
            || job.created_by != LOCAL_USER_ID
            || job.project_id != Some(project.id)
            || job.kind != JobKind::Export
            || job.status != JobStatus::Completed
            || job.cancellation_requested
        {
            return Err(rejected("export_job_conflict"));
        }
        let artifact = self
            .data
            .files_ref()
            .artifact_metadata(job.id)
            .await
            .map_err(map_data_error)?;
        export_output(project.id, project.meta.revision, &job, artifact)
    }

    async fn completed_import_replay(
        &self,
        job: &Job,
        draft: &WriteDraft,
        binding: &ImportCommitDraftPayload,
        resolution: &AiImportResolution,
    ) -> Result<Option<AiDataApplyResult>, ToolExecutionError> {
        if job.status != JobStatus::Completed {
            return Ok(None);
        }
        if job.meta.revision != binding.expected_revision + 1 {
            return Err(rejected("import_revision_conflict"));
        }
        let value = job
            .result
            .clone()
            .ok_or_else(|| rejected("import_completed_result_invalid"))?;
        let recorded_draft = value
            .get("_ai_draft_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok());
        let recorded_revision = value.get("_ai_expected_revision").and_then(Value::as_i64);
        if recorded_draft != Some(draft.id())
            || recorded_revision != Some(binding.expected_revision)
        {
            return Err(rejected("import_revision_conflict"));
        }
        let mut receipt: ImportCommitResult = serde_json::from_value(value)
            .map_err(|_| rejected("import_completed_result_invalid"))?;
        if !receipt
            .preview_hash
            .eq_ignore_ascii_case(binding.preview_hash.trim())
        {
            return Err(rejected("import_preview_hash_conflict"));
        }
        let expected_tool_run =
            completed_ai_import_tool_run(resolution, job.id, &canonical_import_receipt(&receipt))
                .map_err(|_| rejected("import_revision_conflict"))?;
        let stored_tool_run = self
            .data
            .store_ref()
            .get_tool_run(expected_tool_run.id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        let stored_approval = self
            .data
            .store_ref()
            .get_approval(resolution.approval.id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        if stored_tool_run != expected_tool_run || stored_approval != resolution.approval {
            return Err(rejected("import_revision_conflict"));
        }
        self.validate_completed_import_source(job, &receipt).await?;
        receipt.replayed = true;
        Ok(Some(AiDataApplyResult {
            job_id: job.id,
            result: serde_json::to_value(receipt).map_err(|_| ToolExecutionError::Unavailable)?,
        }))
    }

    async fn validate_completed_import_source(
        &self,
        job: &Job,
        receipt: &ImportCommitResult,
    ) -> Result<(), ToolExecutionError> {
        let Some(binding) = optional_source_import_binding(job)? else {
            return Ok(());
        };
        let source = self
            .data
            .store_ref()
            .get_ai_conversation_source(binding.source_id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        let attachment = self
            .data
            .store_ref()
            .get_attachment(binding.attachment_id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        if source.lab_id != job.lab_id
            || source.user_id != job.created_by
            || source.conversation_id != Some(binding.conversation_id)
            || source.project_id != binding.source_project_id
            || source.attachment_id != binding.attachment_id
            || source.status != AiConversationSourceStatus::Archived
            || source.meta.revision != binding.source_revision + 1
            || attachment.lab_id != job.lab_id
            || attachment.project_id != binding.source_project_id
            || attachment.entity_type != "ai_conversation_source"
            || attachment.entity_id != binding.source_id
            || attachment.meta.revision != binding.attachment_revision + 1
        {
            return Err(rejected("import_revision_conflict"));
        }
        for (entity_type, entity_id) in [
            (EntityType::AiConversationSource, binding.source_id),
            (EntityType::Attachment, binding.attachment_id),
        ] {
            let provenance = self
                .data
                .store_ref()
                .list_provenance(&ProvenanceFilter {
                    lab_id: job.lab_id,
                    project_id: binding.source_project_id,
                    entity_type: Some(entity_type),
                    entity_id: Some(entity_id),
                    source: None,
                })
                .await
                .map_err(|_| rejected("import_revision_conflict"))?;
            if !provenance.iter().any(|record| {
                record.import_job_id == Some(job.id)
                    && record.import_commit_id == Some(receipt.commit_id)
            }) {
                return Err(rejected("import_revision_conflict"));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AiDataToolBackend for DesktopAiDataTools {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
        if self.ensure_local_access(access).is_err() {
            return Vec::new();
        }
        let mut tools = Vec::new();
        if access
            .conversation_project_id()
            .is_some_and(|project_id| access.can_import_project(project_id))
        {
            if access.conversation_id().is_some() {
                tools.push(ToolName::SourceImportPreview);
            }
            tools.extend([ToolName::ImportPreview, ToolName::ImportCommitDraft]);
        }
        if access.can_export_anything() {
            tools.push(ToolName::ExportCreate);
        }
        tools
    }

    async fn execute(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        self.ensure_local_access(access)?;
        if request.user_id != LOCAL_USER_ID {
            return Err(rejected("data_tool_forbidden"));
        }
        match request.tool {
            ToolName::SourceImportPreview => {
                self.execute_source_import_preview(access, request).await
            }
            ToolName::ImportPreview => self.execute_import_preview(access, request).await,
            ToolName::ImportCommitDraft => self.execute_import_commit_draft(access, request).await,
            ToolName::ExportCreate => self.execute_export(access, request).await,
            _ => Err(rejected("unsupported_tool")),
        }
    }

    async fn apply_import_draft(
        &self,
        access: &AiDataAccessContext,
        draft: &WriteDraft,
        resolution: &AiImportResolution,
        audit: &AuditContext,
    ) -> Result<AiDataApplyResult, ToolExecutionError> {
        self.ensure_local_access(access)?;
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(LOCAL_USER_ID)
            || draft.kind() != DraftKind::BulkImport
            || draft.tool() != ToolName::ImportCommitDraft
            || draft.status() != muriarc_ai::DraftStatus::Approved
            || resolution.approval.id != draft.id()
            || resolution.approval.tool_run_id != resolution.tool_run.id
            || resolution.approval.decision != StoredApprovalDecision::Approved
            || resolution.tool_run.status != ToolRunStatus::Completed
            || resolution.tool_run.lab_id != access.lab_id()
            || resolution.tool_run.project_id != draft.project_id()
            || resolution.tool_run.user_id != access.user_id()
            || resolution.tool_run.conversation_id != access.conversation_id()
            || resolution.tool_run.tool_name != ToolName::ImportCommitDraft.as_str()
        {
            return Err(rejected("invalid_import_approval"));
        }
        let binding: ImportCommitDraftPayload = serde_json::from_value(draft.payload().clone())
            .map_err(|_| rejected("invalid_import_binding"))?;
        binding.validate()?;
        if resolution.expected_job_revision != binding.expected_revision {
            return Err(rejected("invalid_import_approval"));
        }
        let job = self.import_job(access, binding.job_id).await?;
        if draft.project_id() != job.project_id {
            return Err(rejected("import_job_not_found"));
        }
        if let Some(result) = self
            .completed_import_replay(&job, draft, &binding, resolution)
            .await?
        {
            return Ok(result);
        }
        if job.meta.revision != binding.expected_revision {
            return Err(rejected("import_revision_conflict"));
        }
        let (preview_hash, preview) = self.pending_preview(&job).await?;
        if !preview_hash.eq_ignore_ascii_case(binding.preview_hash.trim()) {
            return Err(rejected("import_preview_hash_conflict"));
        }
        let current_preview = ImportDraftPreviewSummary::from_public_preview(&preview)?;
        if current_preview != binding.preview {
            return Err(rejected("import_preview_conflict"));
        }

        // The normal DataFiles confirmation is the single owner of parse/plan/
        // transaction rules. This adapter only revalidates the immutable binding.
        let source_binding = optional_source_import_binding(&job)?;
        let confirm_options = ImportConfirmOptions {
            source_archive: source_binding.as_ref().map(import_source_archive),
            ai_resolution: Some(resolution.clone()),
        };
        let receipt = if job.project_id.is_some() {
            self.data
                .files_ref()
                .confirm_measurement_import_with_options(
                    &job,
                    &binding.preview_hash,
                    self.data.store_ref(),
                    audit,
                    Utc::now(),
                    confirm_options,
                )
                .await
        } else {
            self.data
                .files_ref()
                .confirm_animal_import_with_options(
                    &job,
                    &binding.preview_hash,
                    self.data.store_ref(),
                    audit,
                    Utc::now(),
                    confirm_options,
                )
                .await
        }
        .map_err(map_data_error)?;
        let _ = self.data.files_ref().clear_pending_import(job.id).await;
        let _ = self.data.files_ref().clear_upload(job.id).await;
        Ok(AiDataApplyResult {
            job_id: job.id,
            result: serde_json::to_value(receipt).map_err(|_| ToolExecutionError::Unavailable)?,
        })
    }
}

fn source_import_job_result(
    preview: &AnimalImportPreviewResponse,
    binding: &SourceImportJobBinding,
) -> Result<Value, ToolExecutionError> {
    let mut result = serde_json::to_value(preview).map_err(|_| ToolExecutionError::Unavailable)?;
    result
        .as_object_mut()
        .ok_or(ToolExecutionError::Unavailable)?
        .insert(
            SOURCE_IMPORT_JOB_BINDING_KEY.to_owned(),
            serde_json::to_value(binding).map_err(|_| ToolExecutionError::Unavailable)?,
        );
    Ok(result)
}

fn source_import_binding(job: &Job) -> Result<SourceImportJobBinding, ToolExecutionError> {
    let binding = job
        .result
        .as_ref()
        .and_then(|value| value.get(SOURCE_IMPORT_JOB_BINDING_KEY))
        .cloned()
        .ok_or_else(|| rejected("source_import_job_conflict"))?;
    let binding: SourceImportJobBinding =
        serde_json::from_value(binding).map_err(|_| rejected("source_import_job_conflict"))?;
    if binding.validate() {
        Ok(binding)
    } else {
        Err(rejected("source_import_job_conflict"))
    }
}

fn optional_source_import_binding(
    job: &Job,
) -> Result<Option<SourceImportJobBinding>, ToolExecutionError> {
    let Some(value) = job
        .result
        .as_ref()
        .and_then(|value| value.get(SOURCE_IMPORT_JOB_BINDING_KEY))
        .cloned()
    else {
        return Ok(None);
    };
    let binding: SourceImportJobBinding =
        serde_json::from_value(value).map_err(|_| rejected("source_import_job_conflict"))?;
    if binding.validate() {
        Ok(Some(binding))
    } else {
        Err(rejected("source_import_job_conflict"))
    }
}

fn import_source_archive(binding: &SourceImportJobBinding) -> ImportSourceArchive {
    ImportSourceArchive {
        source_id: binding.source_id,
        expected_revision: binding.source_revision,
        attachment_id: binding.attachment_id,
        expected_attachment_revision: binding.attachment_revision,
        conversation_id: binding.conversation_id,
        project_id: binding.source_project_id,
    }
}

fn ensure_source_import_job(
    job: &Job,
    access: &AiDataAccessContext,
    project_id: Option<Uuid>,
    idempotency_key: &str,
    binding: &SourceImportJobBinding,
    created: bool,
) -> Result<(), ToolExecutionError> {
    if job.lab_id != access.lab_id()
        || job.created_by != access.user_id()
        || job.project_id != project_id
        || job.kind != JobKind::Import
        || job.idempotency_key != idempotency_key
        || job.cancellation_requested
    {
        return Err(rejected("source_import_job_conflict"));
    }
    if created {
        if job.status != JobStatus::Parsing || job.result.is_some() {
            return Err(rejected("source_import_job_conflict"));
        }
    } else if job.status != JobStatus::AwaitingConfirmation
        || source_import_binding(job)? != *binding
    {
        return Err(rejected("source_import_job_conflict"));
    }
    Ok(())
}

fn map_source_validation_error(error: AiSourceImportValidationError) -> ToolExecutionError {
    match error {
        AiSourceImportValidationError::SourceUnavailable
        | AiSourceImportValidationError::ScopeMismatch => {
            rejected("source_import_source_unavailable")
        }
        AiSourceImportValidationError::InvalidAttachment => {
            rejected("source_import_invalid_material")
        }
        AiSourceImportValidationError::UnsupportedFile => rejected("source_import_invalid_file"),
    }
}

fn source_import_data_error_code(error: &DataError) -> &'static str {
    match error {
        DataError::InvalidFileName
        | DataError::EmptyUpload
        | DataError::UnsupportedUpload(_)
        | DataError::UploadTooLarge(_)
        | DataError::Import(_)
        | DataError::Directory(_)
        | DataError::ChecksumMismatch(_)
        | DataError::CorruptState(_) => "source_import_invalid_file",
        DataError::NotFound
        | DataError::ScopeMismatch
        | DataError::PreviewHasErrors
        | DataError::Plan(_)
        | DataError::Conflict(_) => "source_import_conflict",
        DataError::ArtifactTooLarge(_)
        | DataError::Attachment(_)
        | DataError::Store(_)
        | DataError::Snapshot(_)
        | DataError::Json(_)
        | DataError::Io(_) => "storage_error",
    }
}

fn map_source_import_data_error(error: DataError) -> ToolExecutionError {
    match source_import_data_error_code(&error) {
        "source_import_invalid_file" => rejected("source_import_invalid_file"),
        "source_import_conflict" => rejected("source_import_conflict"),
        _ => ToolExecutionError::Unavailable,
    }
}

fn export_output(
    project_id: Uuid,
    project_revision: i64,
    job: &Job,
    artifact: ArtifactMetadata,
) -> Result<DomainToolOutput, ToolExecutionError> {
    if artifact.job_id != job.id
        || artifact.kind != ArtifactKind::Export
        || job.project_id != Some(project_id)
        || job.status != JobStatus::Completed
    {
        return Err(ToolExecutionError::Unavailable);
    }
    // Do not place a filesystem path, URL or bytes in model-visible output.
    // The human UI can use the ordinary read_data_artifact command with job_id.
    Ok(DomainToolOutput::read(
        json!({
            "job_id": job.id,
            "project_id": project_id,
            "status": job.status,
            "artifact": AiExportArtifactView {
                kind: "export".to_owned(),
                file_name: artifact.file_name,
                media_type: artifact.media_type,
                size_bytes: artifact.size_bytes,
                created_at: artifact.created_at,
            },
        }),
        vec![
            Citation::new(EntityType::Project, project_id, Some(project_revision)),
            Citation::new(EntityType::Job, job.id, Some(job.meta.revision)),
        ],
    ))
}

#[allow(clippy::too_many_arguments)]
async fn transition_job(
    store: &muriarc_store_sqlite::SqliteStore,
    job: &mut Job,
    status: JobStatus,
    progress_current: i64,
    result: Option<Value>,
    error_report: Option<Value>,
    audit: &AuditContext,
) -> Result<(), ToolExecutionError> {
    let expected_revision = job.meta.revision;
    job.status = status;
    job.progress_current = progress_current;
    job.result = result;
    job.error_report = error_report;
    job.meta.touch(Utc::now());
    store
        .update_job(job, expected_revision, audit)
        .await
        .map_err(map_store_error)
}

fn ai_audit(tool_run_id: Uuid, reason: &'static str) -> AuditContext {
    AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(LOCAL_USER_ID),
            display_name: "MuriArc AI data tool".to_owned(),
        },
        source: WriteSource::Ai,
        request_id: Some(tool_run_id.to_string()),
        reason: Some(reason.to_owned()),
    }
}

fn parse_arguments<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ToolExecutionError> {
    serde_json::from_value(value).map_err(|_| rejected("invalid_arguments"))
}

fn rejected(code: &str) -> ToolExecutionError {
    ToolExecutionError::Rejected {
        code: code.to_owned(),
    }
}

fn map_import_job_lookup(error: StoreError) -> ToolExecutionError {
    match error {
        StoreError::NotFound { .. } => rejected("import_job_not_found"),
        _ => ToolExecutionError::Unavailable,
    }
}

fn map_store_error(error: StoreError) -> ToolExecutionError {
    match error {
        StoreError::NotFound { .. } => rejected("job_not_found"),
        StoreError::Conflict(_) => rejected("job_revision_conflict"),
        _ => ToolExecutionError::Unavailable,
    }
}

fn map_data_error(error: DataError) -> ToolExecutionError {
    match error {
        DataError::NotFound | DataError::ScopeMismatch => rejected("import_job_not_found"),
        DataError::Conflict(_) | DataError::Store(StoreError::Conflict(_)) => {
            rejected("import_conflict")
        }
        DataError::Store(StoreError::NotFound { .. }) => rejected("import_job_not_found"),
        DataError::PreviewHasErrors | DataError::Plan(_) => rejected("import_preview_blocked"),
        _ => ToolExecutionError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_ai::DomainToolOutput;
    use muriarc_core::{
        AiConversation, AiConversationSourceStatus, AiOperationStore, Project, StoreError,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use muriarc_data::ImportKind;

    use crate::{
        ai_sources::UploadAiSourceInput,
        application::DesktopState,
        data::{DesktopDataState, PreviewDataImportInput},
    };

    struct Fixture {
        _temp: TempDir,
        data: DesktopDataState,
        backend: DesktopAiDataTools,
        conversation_id: Uuid,
        project_id: Uuid,
    }

    impl Fixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let database = temp.path().join("muriarc.sqlite3");
            DesktopState::initialize(&database).await.unwrap();
            let data = DesktopDataState::initialize(&database, temp.path())
                .await
                .unwrap();
            let project = Project::new(LOCAL_LAB_ID, "AI export project", Utc::now()).unwrap();
            data.store_ref()
                .create_project(&project, &human_audit("fixture_project"))
                .await
                .unwrap();
            let conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                user_id: LOCAL_USER_ID,
                title: "Source import".to_owned(),
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(Utc::now()),
            };
            data.store_ref()
                .create_ai_conversation(&conversation, &human_audit("fixture_conversation"))
                .await
                .unwrap();
            let backend = DesktopAiDataTools::new(data.clone());
            Self {
                _temp: temp,
                data,
                backend,
                conversation_id: conversation.id,
                project_id: project.id,
            }
        }

        fn access(&self) -> AiDataAccessContext {
            AiDataAccessContext::new(
                LOCAL_LAB_ID,
                LOCAL_USER_ID,
                [self.project_id],
                [self.project_id],
                true,
            )
            .with_conversation(self.conversation_id, None)
        }

        fn request(&self, tool: ToolName, arguments: Value) -> DomainToolRequest {
            DomainToolRequest {
                tool_run_id: Uuid::new_v4(),
                provider_call_id: Uuid::new_v4().to_string(),
                user_id: LOCAL_USER_ID,
                tool,
                arguments,
            }
        }

        async fn pending_import(&self) -> AnimalImportPreviewResponse {
            self.data
                .preview_import(PreviewDataImportInput {
                    file_name: "animals.csv".to_owned(),
                    bytes: b"display_id,sex\nAI-LOCAL-001,female\n".to_vec(),
                    idempotency_key: format!("desktop-ai-import-{}", Uuid::new_v4()),
                    import_kind: ImportKind::Animal,
                    experiment_id: None,
                })
                .await
                .unwrap()
        }

        async fn upload_csv_source(&self) -> Uuid {
            let source = self
                .data
                .upload_ai_source(UploadAiSourceInput {
                    file_name: "animals.csv".to_owned(),
                    media_type: Some("text/csv".to_owned()),
                    conversation_id: self.conversation_id.to_string(),
                    project_id: None,
                    bytes: b"display_id,sex\nAI-SOURCE-LOCAL-001,female\n".to_vec(),
                })
                .await
                .unwrap();
            Uuid::parse_str(&source.id).unwrap()
        }
    }

    fn human_audit(reason: &'static str) -> AuditContext {
        AuditContext {
            actor: Actor::human(LOCAL_USER_ID, "Local researcher"),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some(reason.to_owned()),
        }
    }

    #[tokio::test]
    async fn lab_wide_animal_imports_are_not_exposed_or_staged_by_desktop_ai() {
        let fixture = Fixture::new().await;
        let access = fixture.access();
        for forbidden in [
            ToolName::SourceImportPreview,
            ToolName::ImportPreview,
            ToolName::ImportCommitDraft,
        ] {
            assert!(
                !fixture
                    .backend
                    .supported_tools(&access)
                    .contains(&forbidden),
                "lab-wide desktop AI access advertised {forbidden:?}"
            );
        }

        let source_id = fixture.upload_csv_source().await;
        let source_error = fixture
            .backend
            .execute(
                &access,
                fixture.request(
                    ToolName::SourceImportPreview,
                    json!({
                        "source_id": source_id,
                        "import_kind": "animal",
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(source_error, rejected("source_import_forbidden"));
        assert_eq!(
            fixture
                .data
                .store_ref()
                .get_ai_conversation_source(source_id)
                .await
                .unwrap()
                .status,
            AiConversationSourceStatus::Ready
        );

        let preview = fixture.pending_import().await;
        let job = fixture
            .data
            .store_ref()
            .get_job(preview.job_id)
            .await
            .unwrap();
        let preview_error = fixture
            .backend
            .execute(
                &access,
                fixture.request(ToolName::ImportPreview, json!({"job_id": preview.job_id})),
            )
            .await
            .unwrap_err();
        assert_eq!(preview_error, rejected("import_job_not_found"));
        let commit_error = fixture
            .backend
            .execute(
                &access,
                fixture.request(
                    ToolName::ImportCommitDraft,
                    json!({
                        "job_id": preview.job_id,
                        "preview_hash": preview.preview_hash,
                        "expected_revision": job.meta.revision,
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(commit_error, rejected("import_job_not_found"));
        assert_eq!(
            fixture
                .data
                .store_ref()
                .get_job(preview.job_id)
                .await
                .unwrap()
                .status,
            JobStatus::AwaitingConfirmation
        );
    }

    #[tokio::test]
    async fn project_export_is_scoped_and_model_output_has_no_transport_or_content() {
        let fixture = Fixture::new().await;
        let output = fixture
            .backend
            .execute(
                &fixture.access(),
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": fixture.project_id,
                        "resource": "animals",
                        "format": "csv",
                    }),
                ),
            )
            .await
            .unwrap();
        let DomainToolOutput::Read { data, .. } = output else {
            panic!("export_create returns artifact metadata")
        };
        assert_eq!(data["project_id"], json!(fixture.project_id));
        let output_fields = data.as_object().unwrap();
        assert_eq!(output_fields.len(), 4);
        for field in ["job_id", "project_id", "status", "artifact"] {
            assert!(output_fields.contains_key(field), "missing field {field}");
        }
        let serialized = data.to_string().to_ascii_lowercase();
        assert!(!serialized.contains("download_url"));
        assert!(!serialized.contains("relative_path"));
        assert!(!serialized.contains("\"path\""));
        assert!(!serialized.contains("\"url\""));
        assert!(!serialized.contains("\"bytes\""));
        assert!(!serialized.contains("sha256"));
        let job_id = Uuid::parse_str(data["job_id"].as_str().unwrap()).unwrap();
        let job = fixture.data.store_ref().get_job(job_id).await.unwrap();
        assert_eq!(job.project_id, Some(fixture.project_id));
        assert_eq!(job.kind, JobKind::Export);
        assert_eq!(job.status, JobStatus::Completed);

        let artifact = fixture
            .data
            .files_ref()
            .artifact_metadata(job_id)
            .await
            .unwrap();
        let mut wrong_project = job.clone();
        wrong_project.project_id = Some(Uuid::new_v4());
        assert!(matches!(
            export_output(fixture.project_id, 1, &wrong_project, artifact.clone()),
            Err(ToolExecutionError::Unavailable)
        ));
        let mut incomplete = job;
        incomplete.status = JobStatus::Writing;
        assert!(matches!(
            export_output(fixture.project_id, 1, &incomplete, artifact),
            Err(ToolExecutionError::Unavailable)
        ));

        let forbidden_project = Uuid::new_v4();
        let error = fixture
            .backend
            .execute(
                &fixture.access(),
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": forbidden_project,
                        "resource": "animals",
                        "format": "xlsx",
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(error, rejected("project_export_forbidden"));
    }

    #[tokio::test]
    async fn identity_and_argument_shape_fail_closed() {
        let fixture = Fixture::new().await;
        let wrong = AiDataAccessContext::new(
            Uuid::new_v4(),
            LOCAL_USER_ID,
            [fixture.project_id],
            [fixture.project_id],
            true,
        );
        assert!(fixture.backend.supported_tools(&wrong).is_empty());
        let error = fixture
            .backend
            .execute(
                &wrong,
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": fixture.project_id,
                        "resource": "animals",
                        "format": "csv",
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(error, rejected("data_tool_forbidden"));

        let error = fixture
            .backend
            .execute(
                &fixture.access(),
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": fixture.project_id,
                        "resource": "animals",
                        "format": "csv",
                        "path": "C:/arbitrary.csv",
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(error, rejected("invalid_arguments"));

        let store_error = StoreError::Database("private database path".to_owned());
        assert_eq!(
            map_store_error(store_error),
            ToolExecutionError::Unavailable
        );
    }
}
