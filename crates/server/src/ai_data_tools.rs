use std::{path::Path, sync::Arc};

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
    ImportSourceArchive, Job, JobKind, JobStatus, MuriArcStore, ProvenanceFilter, RecordMeta,
    StoreError, ToolRunStatus, WriteSource, canonical_import_receipt, completed_ai_import_tool_run,
};
use muriarc_data::{
    AiSourceImportValidationError, AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata,
    AttachmentFiles, DataError, DataFiles, ExportFormat, ImportConfirmOptions, ImportKind,
    ai_source_import_idempotency_key, artifact_metadata, export_animals_scoped,
    validate_ai_source_import,
};
use muriarc_importer::AnimalExportFilter;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{JobRepository, JobRepositoryError};

const IMPORT_PREVIEW_TTL: Duration = Duration::hours(24);
const MAX_PREVIEW_ISSUES: usize = 50;

/// Server adapter for the three bounded AI data tools.
///
/// This adapter receives only opaque job/project identifiers. Upload bytes,
/// paths, URLs and snapshot creation are deliberately absent from its API.
#[derive(Clone)]
pub(crate) struct ServerAiDataTools {
    store: Arc<dyn MuriArcStore>,
    jobs: Arc<dyn JobRepository>,
    files: Arc<DataFiles>,
    attachments: Option<AttachmentFiles>,
}

impl ServerAiDataTools {
    pub(crate) fn new(
        store: Arc<dyn MuriArcStore>,
        jobs: Arc<dyn JobRepository>,
        files: Arc<DataFiles>,
    ) -> Self {
        Self {
            store,
            jobs,
            files,
            attachments: None,
        }
    }

    pub(crate) fn with_attachment_root(mut self, root: &Path) -> Self {
        self.attachments = Some(AttachmentFiles::new(root));
        self
    }

    async fn execute_source_import_preview(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
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
                    .store
                    .get_experiment(experiment_id)
                    .await
                    .map_err(|_| rejected("source_import_forbidden"))?;
                if experiment.lab_id != access.lab_id()
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
            .store
            .get_ai_conversation_source(arguments.source_id)
            .await
            .map_err(|_| rejected("source_import_source_unavailable"))?;
        let attachment = self
            .store
            .get_attachment(source.attachment_id)
            .await
            .map_err(|_| rejected("source_import_source_unavailable"))?;
        let attachment_files = self
            .attachments
            .as_ref()
            .ok_or_else(|| rejected("source_import_unavailable"))?;
        let bytes = attachment_files
            .read_verified_bytes(&attachment)
            .await
            .map_err(|_| rejected("source_import_invalid_material"))?;
        if bytes.len() as u64 > self.files.max_upload_bytes() {
            return Err(rejected("source_import_invalid_file"));
        }
        let validated = validate_ai_source_import(
            &source,
            &attachment,
            &bytes,
            access.lab_id(),
            access.user_id(),
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
        let audit = ai_audit(access, request.tool_run_id, "ai_source_import_preview");
        let requested = Job {
            id: Uuid::new_v4(),
            lab_id: access.lab_id(),
            project_id,
            created_by: access.user_id(),
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
        let outcome = self
            .jobs
            .create(requested, audit.clone())
            .await
            .map_err(map_job_error)?;
        let mut job = outcome.job;
        ensure_source_import_job(
            &job,
            access,
            project_id,
            &idempotency_key,
            &binding,
            outcome.created,
        )?;
        if !outcome.created {
            return self.source_import_output(&job, &binding).await;
        }

        let operation = async {
            self.files
                .write_upload_bytes(job.id, &validated.file_name, &bytes)
                .await?;
            let preview: AnimalImportPreviewResponse = match import_kind {
                ImportKind::Animal => (&self
                    .files
                    .preview_animal_import(&job, self.store.as_ref())
                    .await?)
                    .into(),
                ImportKind::Measurement => (&self
                    .files
                    .preview_measurement_import(
                        &job,
                        experiment_id.expect("measurement source import has an experiment"),
                        self.store.as_ref(),
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
                    self.jobs.as_ref(),
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
                    self.jobs.as_ref(),
                    &mut job,
                    JobStatus::Failed,
                    0,
                    None,
                    Some(json!({"code": code})),
                    &audit,
                )
                .await;
                let _ = self.files.clear_pending_import(job.id).await;
                let _ = self.files.clear_upload(job.id).await;
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
        let _ = self.files.clear_pending_import(job_id).await;
        let _ = self.files.clear_upload(job_id).await;
        if let Ok(mut current) = self.jobs.get(job_id).await
            && !matches!(
                current.status,
                JobStatus::AwaitingConfirmation | JobStatus::Completed | JobStatus::Failed
            )
        {
            let _ = transition_job(
                self.jobs.as_ref(),
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
        let job = self.jobs.get(job_id).await.map_err(map_job_lookup)?;
        if job.lab_id != access.lab_id()
            || job.created_by != access.user_id()
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
            .store
            .get_project(project_id)
            .await
            .map_err(|_| rejected("import_job_not_found"))?;
        if project.lab_id != access.lab_id() {
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
                    .files
                    .read_pending_measurement_import(job.id)
                    .await
                    .map_err(map_data_error)?;
                if pending.lab_id != job.lab_id
                    || pending.created_by != job.created_by
                    || pending.project_id != project_id
                    || pending.job_id != job.id
                {
                    return Err(rejected("import_job_not_found"));
                }
                (&pending).into()
            }
            None => {
                let pending = self
                    .files
                    .read_pending_import(job.id)
                    .await
                    .map_err(map_data_error)?;
                if pending.lab_id != job.lab_id
                    || pending.created_by != job.created_by
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
        let expires_at = job.meta.created_at + IMPORT_PREVIEW_TTL;
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
            expires_at,
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
            .store
            .get_project(arguments.project_id)
            .await
            .map_err(|_| rejected("project_export_forbidden"))?;
        if project.lab_id != access.lab_id() {
            return Err(rejected("project_export_forbidden"));
        }
        let format = match arguments.format {
            AiExportFormat::Csv => ExportFormat::Csv,
            AiExportFormat::Xlsx => ExportFormat::Xlsx,
        };
        let now = Utc::now();
        let audit = ai_audit(access, request.tool_run_id, "ai_project_export");
        let requested = Job {
            id: Uuid::new_v4(),
            lab_id: access.lab_id(),
            project_id: Some(project.id),
            created_by: access.user_id(),
            kind: JobKind::Export,
            status: JobStatus::Writing,
            idempotency_key: format!("ai-export:{}", request.tool_run_id),
            progress_current: 0,
            progress_total: Some(1),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        let outcome = self
            .jobs
            .create(requested, audit.clone())
            .await
            .map_err(map_job_error)?;
        let mut job = outcome.job;
        if job.lab_id != access.lab_id()
            || job.created_by != access.user_id()
            || job.project_id != Some(project.id)
            || job.kind != JobKind::Export
        {
            return Err(rejected("export_job_conflict"));
        }
        if !outcome.created {
            if job.status != JobStatus::Completed {
                return Err(rejected("export_job_conflict"));
            }
            let artifact = self
                .files
                .artifact_metadata(job.id)
                .await
                .map_err(map_data_error)?;
            return export_output(project.id, project.meta.revision, &job, artifact);
        }

        let operation = async {
            let bytes = export_animals_scoped(
                self.store.as_ref(),
                job.lab_id,
                job.project_id,
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
            self.files.write_artifact(&metadata, &bytes).await?;
            Ok::<_, DataError>(metadata)
        }
        .await;

        match operation {
            Ok(artifact) => {
                transition_job(
                    self.jobs.as_ref(),
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
                .await?;
                export_output(project.id, project.meta.revision, &job, artifact)
            }
            Err(_) => {
                let _ = transition_job(
                    self.jobs.as_ref(),
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
            .store
            .get_tool_run(expected_tool_run.id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        let stored_approval = self
            .store
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
            .store
            .get_ai_conversation_source(binding.source_id)
            .await
            .map_err(|_| rejected("import_revision_conflict"))?;
        let attachment = self
            .store
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
                .store
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
impl AiDataToolBackend for ServerAiDataTools {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
        let mut tools = Vec::new();
        if access
            .conversation_project_id()
            .is_some_and(|project_id| access.can_import_project(project_id))
        {
            if self.attachments.is_some() && access.conversation_id().is_some() {
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
        if request.user_id != access.user_id() {
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
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(access.user_id())
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

        let source_binding = optional_source_import_binding(&job)?;
        let confirm_options = ImportConfirmOptions {
            source_archive: source_binding.as_ref().map(import_source_archive),
            ai_resolution: Some(resolution.clone()),
        };
        let receipt = if job.project_id.is_some() {
            self.files
                .confirm_measurement_import_with_options(
                    &job,
                    &binding.preview_hash,
                    self.store.as_ref(),
                    audit,
                    Utc::now(),
                    confirm_options,
                )
                .await
        } else {
            self.files
                .confirm_animal_import_with_options(
                    &job,
                    &binding.preview_hash,
                    self.store.as_ref(),
                    audit,
                    Utc::now(),
                    confirm_options,
                )
                .await
        }
        .map_err(map_data_error)?;
        if let Err(error) = self.files.clear_pending_import(job.id).await {
            tracing::warn!(job_id = %job.id, error = %error, "AI import pending cleanup failed");
        }
        if let Err(error) = self.files.clear_upload(job.id).await {
            tracing::warn!(job_id = %job.id, error = %error, "AI import upload cleanup failed");
        }
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
    // Keep model-visible output transport-neutral. The human UI can resolve the
    // ordinary artifact download action from the stable job identifier.
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

async fn transition_job(
    jobs: &dyn JobRepository,
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
    jobs.update(job.clone(), expected_revision, audit.clone())
        .await
        .map_err(map_job_error)
}

fn ai_audit(access: &AiDataAccessContext, tool_run_id: Uuid, reason: &'static str) -> AuditContext {
    AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(access.user_id()),
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

fn map_job_lookup(error: JobRepositoryError) -> ToolExecutionError {
    match error {
        JobRepositoryError::NotFound(_) => rejected("import_job_not_found"),
        JobRepositoryError::IdempotencyConflict | JobRepositoryError::Unavailable => {
            ToolExecutionError::Unavailable
        }
    }
}

fn map_job_error(error: JobRepositoryError) -> ToolExecutionError {
    match error {
        JobRepositoryError::IdempotencyConflict => rejected("job_revision_conflict"),
        JobRepositoryError::NotFound(_) => rejected("job_not_found"),
        JobRepositoryError::Unavailable => ToolExecutionError::Unavailable,
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
        AiConversation, AiConversationSource, AiConversationSourceKind, AiConversationSourceStatus,
        AiModelProfile, AiModelProfileBinding, AiModelProfileStore, AiModelProfileVersion,
        AiOperationStore, AiProviderProtocol, AiProviderTransport, Attachment, Lab, Project, Sex,
        User, WorkspaceStore,
    };
    use muriarc_store_sqlite::SqliteStore;
    use tempfile::TempDir;

    use crate::StoreJobRepository;

    struct Fixture {
        _temp: TempDir,
        store: Arc<SqliteStore>,
        jobs: Arc<StoreJobRepository>,
        backend: ServerAiDataTools,
        lab_id: Uuid,
        user_id: Uuid,
        conversation_id: Uuid,
        project_id: Uuid,
        other_project_id: Uuid,
    }

    impl Fixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(SqliteStore::in_memory().await.unwrap());
            store.migrate().await.unwrap();
            let now = Utc::now();
            let bootstrap = AuditContext::system(WriteSource::Migration);
            let lab = Lab::new("AI data backend", now).unwrap();
            store.create_lab(&lab, &bootstrap).await.unwrap();
            let user = User::new(lab.id, "ai-data@example.test", "AI Data", now).unwrap();
            store.create_user(&user, &bootstrap).await.unwrap();
            let model_profile = AiModelProfile {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                user_id: user.id,
                name: "AI data fixture model".to_owned(),
                current_version: 1,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            let model_version = AiModelProfileVersion {
                profile_id: model_profile.id,
                version: 1,
                protocol: AiProviderProtocol::OpenaiChatCompletions,
                transport: AiProviderTransport::OpenAiCompatible,
                base_url: "https://provider.example.test/v1".to_owned(),
                normalized_base_url: "https://provider.example.test/v1".to_owned(),
                model_id: "ai-data-fixture-model".to_owned(),
                supports_vision: false,
                context_window_tokens: 16_384,
                max_input_tokens: 8_192,
                max_output_tokens: 2_048,
                history_token_budget: 4_096,
                history_turns: 20,
                temperature: 0.0,
                timeout_ms: 30_000,
                created_at: now,
            };
            store
                .create_ai_model_profile(&model_profile, &model_version, &bootstrap)
                .await
                .unwrap();
            let conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id: None,
                user_id: user.id,
                title: "Source import".to_owned(),
                model_profile: Some(AiModelProfileBinding {
                    profile_id: model_profile.id,
                    profile_version: 1,
                }),
                legacy_read_only: false,
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            store
                .create_ai_conversation(&conversation, &bootstrap)
                .await
                .unwrap();
            let project = Project::new(lab.id, "Allowed project", now).unwrap();
            let other_project = Project::new(lab.id, "Other project", now).unwrap();
            store.create_project(&project, &bootstrap).await.unwrap();
            store
                .create_project(&other_project, &bootstrap)
                .await
                .unwrap();
            // A project-scoped export may be empty; a lab animal is included
            // only after normal Participation links it to an experiment.
            let animal =
                muriarc_core::Animal::new_mouse(lab.id, "M-AI-EXPORT", Sex::Female, now).unwrap();
            store.create_animal(&animal, &bootstrap).await.unwrap();

            let jobs = Arc::new(StoreJobRepository::new(store.clone()));
            let files = Arc::new(DataFiles::new(temp.path().join("data")));
            let backend = ServerAiDataTools::new(store.clone(), jobs.clone(), files)
                .with_attachment_root(&temp.path().join("attachments"));
            Self {
                _temp: temp,
                store,
                jobs,
                backend,
                lab_id: lab.id,
                user_id: user.id,
                conversation_id: conversation.id,
                project_id: project.id,
                other_project_id: other_project.id,
            }
        }

        fn human_audit(&self) -> AuditContext {
            AuditContext {
                actor: Actor::human(self.user_id, "AI Data"),
                source: WriteSource::Web,
                request_id: Some(Uuid::new_v4().to_string()),
                reason: Some("approved AI import".to_owned()),
            }
        }

        fn access(&self) -> AiDataAccessContext {
            AiDataAccessContext::new(
                self.lab_id,
                self.user_id,
                [self.project_id],
                [self.project_id],
                true,
            )
            .with_conversation(self.conversation_id, None)
        }

        async fn upload_csv_source(&self, bytes: &[u8]) -> AiConversationSource {
            let now = Utc::now();
            let source_id = Uuid::new_v4();
            let attachment_id = Uuid::new_v4();
            let object = self
                .backend
                .attachments
                .as_ref()
                .unwrap()
                .write_bytes(attachment_id, bytes)
                .await
                .unwrap();
            let attachment = Attachment {
                id: attachment_id,
                lab_id: self.lab_id,
                project_id: None,
                entity_type: "ai_conversation_source".to_owned(),
                entity_id: source_id,
                file_name: "animals.csv".to_owned(),
                media_type: Some("text/csv".to_owned()),
                relative_path: object.relative_path,
                size_bytes: object.size_bytes,
                sha256: object.sha256,
                version: 1,
                meta: RecordMeta::new(now),
            };
            let source = AiConversationSource {
                id: source_id,
                lab_id: self.lab_id,
                user_id: self.user_id,
                conversation_id: Some(self.conversation_id),
                project_id: None,
                attachment_id,
                kind: AiConversationSourceKind::DelimitedText,
                status: AiConversationSourceStatus::Ready,
                last_activity_at: now,
                expires_at: now + Duration::hours(1),
                archived_at: None,
                error_code: None,
                meta: RecordMeta::new(now),
            };
            self.store
                .create_ai_conversation_source(&attachment, &source, &self.human_audit())
                .await
                .unwrap();
            source
        }

        async fn pending_animal_import(&self, created_at: chrono::DateTime<Utc>) -> Job {
            let mut job = Job {
                id: Uuid::new_v4(),
                lab_id: self.lab_id,
                project_id: None,
                created_by: self.user_id,
                kind: JobKind::Import,
                status: JobStatus::Parsing,
                idempotency_key: format!("test-import-{}", Uuid::new_v4()),
                progress_current: 0,
                progress_total: Some(3),
                result: None,
                error_report: None,
                cancellation_requested: false,
                meta: RecordMeta::new(created_at),
            };
            self.jobs
                .create(job.clone(), self.human_audit())
                .await
                .unwrap();
            let display_id = format!("M-{}", &job.id.simple().to_string()[..12]);
            self.backend
                .files
                .write_upload_bytes(
                    job.id,
                    "animals.csv",
                    format!("display_id,sex\n{display_id},female\n").as_bytes(),
                )
                .await
                .unwrap();
            let pending = self
                .backend
                .files
                .preview_animal_import(&job, self.store.as_ref())
                .await
                .unwrap();
            let expected_revision = job.meta.revision;
            job.status = JobStatus::AwaitingConfirmation;
            job.progress_current = 2;
            job.result =
                Some(serde_json::to_value(AnimalImportPreviewResponse::from(&pending)).unwrap());
            job.meta.touch(Utc::now());
            self.jobs
                .update(job.clone(), expected_revision, self.human_audit())
                .await
                .unwrap();
            job
        }

        fn request(&self, tool: ToolName, arguments: Value) -> DomainToolRequest {
            DomainToolRequest {
                tool_run_id: Uuid::new_v4(),
                provider_call_id: Uuid::new_v4().to_string(),
                user_id: self.user_id,
                tool,
                arguments,
            }
        }
    }

    #[tokio::test]
    async fn lab_wide_animal_imports_are_not_exposed_or_staged_by_ai() {
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
                "lab-wide AI access advertised {forbidden:?}"
            );
        }
        let source = fixture
            .upload_csv_source(b"display_id,sex\nAI-SOURCE-001,female\n")
            .await;
        let source_error = fixture
            .backend
            .execute(
                &access,
                fixture.request(
                    ToolName::SourceImportPreview,
                    json!({
                        "source_id": source.id,
                        "import_kind": "animal",
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(source_error, rejected("source_import_forbidden"));
        assert_eq!(
            fixture
                .store
                .get_ai_conversation_source(source.id)
                .await
                .unwrap()
                .status,
            AiConversationSourceStatus::Ready
        );
        assert!(fixture.jobs.list(fixture.lab_id).await.unwrap().is_empty());

        let job = fixture.pending_animal_import(Utc::now()).await;
        let pending = fixture
            .backend
            .files
            .read_pending_import(job.id)
            .await
            .unwrap();
        let preview_error = fixture
            .backend
            .execute(
                &access,
                fixture.request(ToolName::ImportPreview, json!({"job_id": job.id})),
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
                        "job_id": job.id,
                        "preview_hash": pending.preview_hash,
                        "expected_revision": job.meta.revision,
                    }),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(commit_error, rejected("import_job_not_found"));
        assert_eq!(
            fixture.jobs.get(job.id).await.unwrap().status,
            JobStatus::AwaitingConfirmation
        );
    }

    #[tokio::test]
    async fn export_is_project_scoped_and_creates_only_a_downloadable_artifact_job() {
        let fixture = Fixture::new().await;
        let access = fixture.access();
        let forbidden = fixture
            .backend
            .execute(
                &access,
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": fixture.other_project_id,
                        "resource": "animals",
                        "format": "csv",
                    }),
                ),
            )
            .await;
        assert!(matches!(
            forbidden,
            Err(ToolExecutionError::Rejected { ref code }) if code == "project_export_forbidden"
        ));

        let exported = fixture
            .backend
            .execute(
                &access,
                fixture.request(
                    ToolName::ExportCreate,
                    json!({
                        "project_id": fixture.project_id,
                        "resource": "animals",
                        "format": "xlsx",
                    }),
                ),
            )
            .await
            .unwrap();
        let DomainToolOutput::Read { data, citations } = exported else {
            panic!("export creates an artifact immediately, not a write draft");
        };
        let job_id = serde_json::from_value::<Uuid>(data["job_id"].clone()).unwrap();
        let job = fixture.jobs.get(job_id).await.unwrap();
        assert_eq!(job.kind, JobKind::Export);
        assert_eq!(job.project_id, Some(fixture.project_id));
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(data["artifact"]["kind"], "export");
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
        assert!(
            citations
                .iter()
                .any(|citation| citation.entity_id == job_id)
        );

        let artifact = fixture
            .backend
            .files
            .artifact_metadata(job_id)
            .await
            .unwrap();
        let mut wrong_project = job.clone();
        wrong_project.project_id = Some(fixture.other_project_id);
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
    }
}
