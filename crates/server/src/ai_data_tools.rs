use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use muriarc_ai::{
    AiDataAccessContext, AiDataApplyResult, AiDataToolBackend, AiExportFormat, AiExportResource,
    Citation, DomainToolOutput, DomainToolRequest, DraftKind, ExportCreateArguments, FieldChange,
    ImportCommitDraftArguments, ImportCommitDraftPayload, ImportPreviewArguments, ProposalActor,
    ToolExecutionError, ToolName, WriteDraft, valid_sha256,
};
use muriarc_core::{
    Actor, ActorType, AuditContext, EntityType, ImportCommitResult, Job, JobKind, JobStatus,
    MuriArcStore, RecordMeta, WriteSource,
};
use muriarc_data::{
    AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata, DataError, DataFiles,
    ExportFormat, artifact_metadata, export_animals_scoped,
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
}

impl ServerAiDataTools {
    pub(crate) fn new(
        store: Arc<dyn MuriArcStore>,
        jobs: Arc<dyn JobRepository>,
        files: Arc<DataFiles>,
    ) -> Self {
        Self { store, jobs, files }
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
        match job.project_id {
            Some(project_id) if !access.can_import_project(project_id) => {
                return Err(rejected("import_job_not_found"));
            }
            None if !access.can_import_lab() => {
                return Err(rejected("import_job_not_found"));
            }
            _ => {}
        }
        if let Some(project_id) = job.project_id {
            let project = self
                .store
                .get_project(project_id)
                .await
                .map_err(|_| rejected("import_job_not_found"))?;
            if project.lab_id != access.lab_id() {
                return Err(rejected("import_job_not_found"));
            }
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

        let now = Utc::now();
        let expires_at = job.meta.created_at + IMPORT_PREVIEW_TTL;
        let payload = ImportCommitDraftPayload {
            operation: ImportCommitDraftPayload::OPERATION.to_owned(),
            job_id: job.id,
            preview_hash: preview_hash.clone(),
            expected_revision: job.meta.revision,
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
        let receipt: ImportCommitResult = serde_json::from_value(value)
            .map_err(|_| rejected("import_completed_result_invalid"))?;
        if !receipt
            .preview_hash
            .eq_ignore_ascii_case(binding.preview_hash.trim())
        {
            return Err(rejected("import_preview_hash_conflict"));
        }
        Ok(Some(AiDataApplyResult {
            job_id: job.id,
            result: serde_json::to_value(receipt).map_err(|_| ToolExecutionError::Unavailable)?,
        }))
    }
}

#[async_trait]
impl AiDataToolBackend for ServerAiDataTools {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
        let mut tools = Vec::new();
        if access.can_import_anything() {
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
        audit: &AuditContext,
    ) -> Result<AiDataApplyResult, ToolExecutionError> {
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(access.user_id())
            || draft.kind() != DraftKind::BulkImport
            || draft.tool() != ToolName::ImportCommitDraft
            || draft.status() != muriarc_ai::DraftStatus::Approved
        {
            return Err(rejected("invalid_import_approval"));
        }
        let binding: ImportCommitDraftPayload = serde_json::from_value(draft.payload().clone())
            .map_err(|_| rejected("invalid_import_binding"))?;
        binding.validate()?;
        let mut job = self.import_job(access, binding.job_id).await?;
        if draft.project_id() != job.project_id {
            return Err(rejected("import_job_not_found"));
        }
        if let Some(result) = self.completed_import_replay(&job, draft, &binding).await? {
            return Ok(result);
        }
        if job.meta.revision != binding.expected_revision {
            return Err(rejected("import_revision_conflict"));
        }
        let (preview_hash, _) = self.pending_preview(&job).await?;
        if !preview_hash.eq_ignore_ascii_case(binding.preview_hash.trim()) {
            return Err(rejected("import_preview_hash_conflict"));
        }

        let receipt = if job.project_id.is_some() {
            self.files
                .confirm_measurement_import(
                    &job,
                    &binding.preview_hash,
                    self.store.as_ref(),
                    audit,
                    Utc::now(),
                )
                .await
        } else {
            self.files
                .confirm_animal_import(
                    &job,
                    &binding.preview_hash,
                    self.store.as_ref(),
                    audit,
                    Utc::now(),
                )
                .await
        }
        .map_err(map_data_error)?;

        let mut stored_result =
            serde_json::to_value(&receipt).map_err(|_| ToolExecutionError::Unavailable)?;
        let object = stored_result
            .as_object_mut()
            .ok_or(ToolExecutionError::Unavailable)?;
        object.insert("_ai_draft_id".to_owned(), json!(draft.id()));
        object.insert(
            "_ai_expected_revision".to_owned(),
            json!(binding.expected_revision),
        );
        let completed_progress = job.progress_total.unwrap_or(3);
        let transition = transition_job(
            self.jobs.as_ref(),
            &mut job,
            JobStatus::Completed,
            completed_progress,
            Some(stored_result),
            None,
            audit,
        )
        .await;
        if transition.is_err() {
            // A domain import is idempotent. If the job transition raced with
            // an exact retry of this same draft, accept only the marked,
            // completed state; every other revision remains a hard conflict.
            let current = self.jobs.get(job.id).await.map_err(map_job_lookup)?;
            if let Some(result) = self
                .completed_import_replay(&current, draft, &binding)
                .await?
            {
                return Ok(result);
            }
            return Err(ToolExecutionError::Unavailable);
        }
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

fn export_output(
    project_id: Uuid,
    project_revision: i64,
    job: &Job,
    artifact: ArtifactMetadata,
) -> Result<DomainToolOutput, ToolExecutionError> {
    if artifact.job_id != job.id || artifact.kind != ArtifactKind::Export {
        return Err(ToolExecutionError::Unavailable);
    }
    Ok(DomainToolOutput::read(
        json!({
            "job_id": job.id,
            "project_id": project_id,
            "status": job.status,
            "artifact": artifact,
            "download_url": format!("/api/v1/data/artifacts/{}", job.id),
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
        DataError::Conflict(_) => rejected("import_conflict"),
        DataError::PreviewHasErrors | DataError::Plan(_) => rejected("import_preview_blocked"),
        _ => ToolExecutionError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_ai::{ApprovalDecision, DomainToolOutput, HumanApprover};
    use muriarc_core::{Lab, Project, Sex, User};
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
            let backend = ServerAiDataTools::new(store.clone(), jobs.clone(), files);
            Self {
                _temp: temp,
                store,
                jobs,
                backend,
                lab_id: lab.id,
                user_id: user.id,
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
    async fn preview_and_reinforced_apply_are_owner_bound_and_idempotent() {
        let fixture = Fixture::new().await;
        let access = fixture.access();
        let job = fixture.pending_animal_import(Utc::now()).await;
        let pending = fixture
            .backend
            .files
            .read_pending_import(job.id)
            .await
            .unwrap();

        let preview = fixture
            .backend
            .execute(
                &access,
                fixture.request(ToolName::ImportPreview, json!({"job_id": job.id})),
            )
            .await
            .unwrap();
        let DomainToolOutput::Read { data, citations } = preview else {
            panic!("preview must be read-only");
        };
        assert_eq!(data["job_revision"], job.meta.revision);
        assert_eq!(data["preview_hash"], pending.preview_hash);
        assert_eq!(citations[0].entity_id, job.id);

        let output = fixture
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
            .unwrap();
        let DomainToolOutput::WriteDraft { mut draft, .. } = output else {
            panic!("commit tool must only create a draft");
        };
        assert_eq!(draft.kind(), DraftKind::BulkImport);
        draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                HumanApprover {
                    user_id: fixture.user_id,
                    display_name: "AI Data".to_owned(),
                },
                Some("I reviewed this import preview".to_owned()),
                true,
                Utc::now(),
            )
            .unwrap();
        let applied = fixture
            .backend
            .apply_import_draft(&access, &draft, &fixture.human_audit())
            .await
            .unwrap();
        assert_eq!(applied.job_id, job.id);
        assert_eq!(
            fixture.jobs.get(job.id).await.unwrap().status,
            JobStatus::Completed
        );
        let replayed = fixture
            .backend
            .apply_import_draft(&access, &draft, &fixture.human_audit())
            .await
            .unwrap();
        assert_eq!(replayed.job_id, job.id);
        assert_eq!(
            fixture
                .store
                .list_animals(&muriarc_core::AnimalFilter {
                    lab_id: fixture.lab_id,
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            2,
            "the exact replay must not duplicate the imported animal"
        );
    }

    #[tokio::test]
    async fn import_rejects_unknown_owner_expiry_hash_and_revision_without_leaking_jobs() {
        let fixture = Fixture::new().await;
        let access = fixture.access();
        let job = fixture.pending_animal_import(Utc::now()).await;
        let pending = fixture
            .backend
            .files
            .read_pending_import(job.id)
            .await
            .unwrap();

        let unknown = fixture
            .backend
            .execute(
                &access,
                fixture.request(ToolName::ImportPreview, json!({"job_id": Uuid::new_v4()})),
            )
            .await;
        assert!(matches!(unknown, Err(ToolExecutionError::Rejected { .. })));

        let other_user = AiDataAccessContext::new(
            fixture.lab_id,
            Uuid::new_v4(),
            [fixture.project_id],
            [fixture.project_id],
            true,
        );
        let other_owner = fixture
            .backend
            .execute(
                &other_user,
                DomainToolRequest {
                    user_id: other_user.user_id(),
                    ..fixture.request(ToolName::ImportPreview, json!({"job_id": job.id}))
                },
            )
            .await;
        assert!(matches!(
            other_owner,
            Err(ToolExecutionError::Rejected { ref code }) if code == "import_job_not_found"
        ));

        for arguments in [
            json!({
                "job_id": job.id,
                "preview_hash": "b".repeat(64),
                "expected_revision": job.meta.revision,
            }),
            json!({
                "job_id": job.id,
                "preview_hash": pending.preview_hash,
                "expected_revision": job.meta.revision + 1,
            }),
        ] {
            let result = fixture
                .backend
                .execute(
                    &access,
                    fixture.request(ToolName::ImportCommitDraft, arguments),
                )
                .await;
            assert!(matches!(result, Err(ToolExecutionError::Rejected { .. })));
        }

        let expired = fixture
            .pending_animal_import(Utc::now() - Duration::hours(25))
            .await;
        let result = fixture
            .backend
            .execute(
                &access,
                fixture.request(ToolName::ImportPreview, json!({"job_id": expired.id})),
            )
            .await;
        assert!(matches!(
            result,
            Err(ToolExecutionError::Rejected { ref code }) if code == "import_preview_expired"
        ));
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
        assert!(
            data["download_url"]
                .as_str()
                .unwrap()
                .starts_with("/api/v1/data/artifacts/")
        );
        assert!(
            citations
                .iter()
                .any(|citation| citation.entity_id == job_id)
        );
    }
}
