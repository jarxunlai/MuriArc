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
    LOCAL_LAB_ID, LOCAL_USER_ID, MuriArcStore, RecordMeta, StoreError, WriteSource,
};
use muriarc_data::{
    AnimalImportPreviewResponse, ArtifactKind, ArtifactMetadata, DataError, ExportFormat,
    artifact_metadata, export_animals_scoped,
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
                .data
                .store_ref()
                .get_project(project_id)
                .await
                .map_err(|_| rejected("import_job_not_found"))?;
            if project.lab_id != LOCAL_LAB_ID {
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
impl AiDataToolBackend for DesktopAiDataTools {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
        if self.ensure_local_access(access).is_err() {
            return Vec::new();
        }
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
        self.ensure_local_access(access)?;
        if request.user_id != LOCAL_USER_ID {
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
        self.ensure_local_access(access)?;
        if audit.actor.actor_type != ActorType::Human
            || audit.actor.user_id != Some(LOCAL_USER_ID)
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

        // The normal DataFiles confirmation is the single owner of parse/plan/
        // transaction rules. This adapter only revalidates the immutable binding.
        let receipt = if job.project_id.is_some() {
            self.data
                .files_ref()
                .confirm_measurement_import(
                    &job,
                    &binding.preview_hash,
                    self.data.store_ref(),
                    audit,
                    Utc::now(),
                )
                .await
        } else {
            self.data
                .files_ref()
                .confirm_animal_import(
                    &job,
                    &binding.preview_hash,
                    self.data.store_ref(),
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
        if transition_job(
            self.data.store_ref(),
            &mut job,
            JobStatus::Completed,
            completed_progress,
            Some(stored_result),
            None,
            audit,
        )
        .await
        .is_err()
        {
            let current = self
                .data
                .store_ref()
                .get_job(job.id)
                .await
                .map_err(map_import_job_lookup)?;
            if let Some(result) = self
                .completed_import_replay(&current, draft, &binding)
                .await?
            {
                return Ok(result);
            }
            return Err(ToolExecutionError::Unavailable);
        }
        let _ = self.data.files_ref().clear_pending_import(job.id).await;
        let _ = self.data.files_ref().clear_upload(job.id).await;
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
            "artifact": artifact,
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
        DataError::Conflict(_) => rejected("import_conflict"),
        DataError::PreviewHasErrors | DataError::Plan(_) => rejected("import_preview_blocked"),
        _ => ToolExecutionError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_ai::{ApprovalDecision, ApprovalRequirement, DomainToolOutput, HumanApprover};
    use muriarc_core::{Project, StoreError};
    use serde_json::json;
    use tempfile::TempDir;

    use muriarc_data::ImportKind;

    use crate::{
        application::DesktopState,
        data::{DesktopDataState, PreviewDataImportInput},
    };

    struct Fixture {
        _temp: TempDir,
        data: DesktopDataState,
        backend: DesktopAiDataTools,
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
            let backend = DesktopAiDataTools::new(data.clone());
            Self {
                _temp: temp,
                data,
                backend,
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
    async fn existing_pending_import_is_previewable_drafted_and_applied_through_normal_data_flow() {
        let fixture = Fixture::new().await;
        let preview = fixture.pending_import().await;
        let job = fixture
            .data
            .store_ref()
            .get_job(preview.job_id)
            .await
            .unwrap();

        let output = fixture
            .backend
            .execute(
                &fixture.access(),
                fixture.request(ToolName::ImportPreview, json!({"job_id": preview.job_id})),
            )
            .await
            .unwrap();
        let DomainToolOutput::Read { data, .. } = output else {
            panic!("import_preview must stay read-only")
        };
        assert_eq!(data["preview_hash"], preview.preview_hash);
        assert_eq!(data["job_revision"], job.meta.revision);

        let output = fixture
            .backend
            .execute(
                &fixture.access(),
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
            .unwrap();
        let DomainToolOutput::WriteDraft { mut draft, .. } = output else {
            panic!("import_commit_draft must not commit")
        };
        assert_eq!(
            draft.requirement(),
            ApprovalRequirement::ReinforcedConfirmation
        );
        assert!(!draft.payload().to_string().contains("path"));
        assert!(!draft.payload().to_string().contains("url"));
        assert!(!draft.payload().to_string().contains("bytes"));

        draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                HumanApprover {
                    user_id: LOCAL_USER_ID,
                    display_name: "Local researcher".to_owned(),
                },
                Some("I reviewed the import diff".to_owned()),
                true,
                Utc::now(),
            )
            .unwrap();
        let result = fixture
            .backend
            .apply_import_draft(&fixture.access(), &draft, &human_audit("attempt_apply"))
            .await
            .unwrap();
        assert_eq!(result.job_id, preview.job_id);
        let completed = fixture
            .data
            .store_ref()
            .get_job(preview.job_id)
            .await
            .unwrap();
        assert_eq!(completed.status, JobStatus::Completed);
        assert_eq!(completed.meta.revision, job.meta.revision + 1);
        let animals = fixture
            .data
            .store_ref()
            .list_animals(&muriarc_core::AnimalFilter {
                lab_id: LOCAL_LAB_ID,
                query: Some("AI-LOCAL-001".to_owned()),
                ..muriarc_core::AnimalFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(animals.len(), 1);

        let replayed = fixture
            .backend
            .apply_import_draft(&fixture.access(), &draft, &human_audit("exact_replay"))
            .await
            .unwrap();
        assert_eq!(replayed.job_id, preview.job_id);
        let animals_after_replay = fixture
            .data
            .store_ref()
            .list_animals(&muriarc_core::AnimalFilter {
                lab_id: LOCAL_LAB_ID,
                query: Some("AI-LOCAL-001".to_owned()),
                ..muriarc_core::AnimalFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(animals_after_replay.len(), 1);
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
        let serialized = data.to_string().to_ascii_lowercase();
        assert!(!serialized.contains("download_url"));
        assert!(!serialized.contains("relative_path"));
        assert!(!serialized.contains("\"bytes\""));
        let job_id = Uuid::parse_str(data["job_id"].as_str().unwrap()).unwrap();
        let job = fixture.data.store_ref().get_job(job_id).await.unwrap();
        assert_eq!(job.project_id, Some(fixture.project_id));
        assert_eq!(job.kind, JobKind::Export);
        assert_eq!(job.status, JobStatus::Completed);

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
