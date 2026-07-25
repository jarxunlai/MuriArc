use std::io::Cursor;

use chrono::{DateTime, Utc};
use muriarc_application::{
    ApplicationError, CancelGenotypingBatchCommand, CommitGenotypingBatchCommand,
    CreateGenotypingBatchCommand, SetGenotypingBatchPreviewCommand,
    cancel_genotyping_batch as cancel_use_case, commit_genotyping_batch as commit_use_case,
    create_genotyping_batch as create_use_case,
    set_genotyping_batch_preview as set_preview_use_case,
};
use muriarc_core::{
    Actor, AnimalFilter, Attachment, AuditContext, GenotypingBatch, GenotypingBatchFilter,
    GenotypingBatchReceipt, GenotypingBatchStatus, GenotypingRecord, LOCAL_LAB_ID, LOCAL_USER_ID,
    MuriArcStore, StoreError, WriteSource,
};
use muriarc_importer::{
    AnimalDirectory, GenotypingFieldMapping, GenotypingImportPreview, genotyping_template_csv,
    preview_genotyping, read_csv, read_xlsx,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::{DesktopDataError, DesktopDataState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListGenotypingBatchesInput {
    project_id: Option<Uuid>,
    status: Option<GenotypingBatchStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateGenotypingBatchInput {
    project_id: Option<Uuid>,
    batch_number: String,
    genotype_definition_id: Uuid,
    assessed_at: DateTime<Utc>,
    method: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GenotypingBatchPreviewInput {
    batch_id: Uuid,
    expected_revision: i64,
    source_attachment_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitGenotypingBatchInput {
    batch_id: Uuid,
    expected_revision: i64,
    preview_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CancelGenotypingBatchInput {
    batch_id: Uuid,
    expected_revision: i64,
    reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GenotypingBatchPreviewView {
    pub batch: GenotypingBatch,
    pub preview: GenotypingImportPreview,
}

#[derive(Debug, Serialize)]
pub(crate) struct GenotypingBatchDetailView {
    pub batch: GenotypingBatch,
    pub records: Vec<GenotypingRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GenotypingBatchTemplateView {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl DesktopDataState {
    async fn genotyping_batch_audit(
        &self,
        reason: &'static str,
    ) -> Result<AuditContext, DesktopDataError> {
        let operator = self.store_ref().get_user(LOCAL_USER_ID).await?;
        Ok(AuditContext {
            actor: Actor::human(LOCAL_USER_ID, operator.display_name),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some(reason.to_owned()),
        })
    }

    pub(crate) async fn list_genotyping_batches(
        &self,
        input: ListGenotypingBatchesInput,
    ) -> Result<Vec<GenotypingBatch>, DesktopDataError> {
        Ok(self
            .store_ref()
            .list_genotyping_batches(&GenotypingBatchFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: input.project_id,
                status: input.status,
            })
            .await?)
    }

    pub(crate) async fn get_genotyping_batch(
        &self,
        batch_id: Uuid,
    ) -> Result<GenotypingBatchDetailView, DesktopDataError> {
        let batch = self.store_ref().get_genotyping_batch(batch_id).await?;
        ensure_local_batch(&batch)?;
        let records = self
            .store_ref()
            .list_genotyping_batch_records(batch_id)
            .await?;
        Ok(GenotypingBatchDetailView { batch, records })
    }

    pub(crate) async fn get_genotyping_batch_for_record(
        &self,
        record_id: Uuid,
    ) -> Result<Option<GenotypingBatch>, DesktopDataError> {
        let record = self.store_ref().get_genotyping_record(record_id).await?;
        if record.lab_id != LOCAL_LAB_ID {
            return Err(DesktopDataError::ScopeMismatch);
        }
        let batch = self
            .store_ref()
            .find_genotyping_batch_for_record(record_id)
            .await?;
        if let Some(batch) = batch.as_ref() {
            ensure_local_batch(batch)?;
            if batch.project_id != record.project_id {
                return Err(DesktopDataError::ScopeMismatch);
            }
        }
        Ok(batch)
    }

    pub(crate) async fn create_genotyping_batch(
        &self,
        input: CreateGenotypingBatchInput,
    ) -> Result<GenotypingBatch, DesktopDataError> {
        let audit = self
            .genotyping_batch_audit("create_genotyping_batch")
            .await?;
        create_use_case(
            self.store_ref(),
            CreateGenotypingBatchCommand {
                lab_id: LOCAL_LAB_ID,
                project_id: input.project_id,
                batch_number: input.batch_number,
                genotype_definition_id: input.genotype_definition_id,
                assessed_at: input.assessed_at,
                method: input.method,
                notes: input.notes,
                created_by: Some(LOCAL_USER_ID),
                now: Utc::now(),
            },
            &audit,
        )
        .await
        .map_err(application_error)
    }

    pub(crate) async fn preview_genotyping_batch(
        &self,
        input: GenotypingBatchPreviewInput,
    ) -> Result<GenotypingBatchPreviewView, DesktopDataError> {
        let mut batch = self
            .store_ref()
            .get_genotyping_batch(input.batch_id)
            .await?;
        ensure_local_batch(&batch)?;
        if batch.status != GenotypingBatchStatus::Draft {
            return Err(
                StoreError::Conflict("genotyping batch is no longer a draft".to_owned()).into(),
            );
        }
        let source = self
            .genotyping_source_attachment(&batch, input.source_attachment_id)
            .await?;
        let preview = self.parse_genotyping_preview(&batch, &source).await?;
        if preview.can_confirm() {
            let row_count = i64::try_from(preview.accepted_rows.len()).map_err(|_| {
                StoreError::Validation("genotyping preview has too many rows".to_owned())
            })?;
            let audit = self
                .genotyping_batch_audit("preview_genotyping_batch")
                .await?;
            batch = set_preview_use_case(
                self.store_ref(),
                SetGenotypingBatchPreviewCommand {
                    batch_id: batch.id,
                    expected_revision: input.expected_revision,
                    source_attachment_id: source.id,
                    preview_hash: preview.preview_hash.clone(),
                    row_count,
                    now: Utc::now(),
                },
                &audit,
            )
            .await
            .map_err(application_error)?;
        }
        Ok(GenotypingBatchPreviewView { batch, preview })
    }

    pub(crate) async fn commit_genotyping_batch(
        &self,
        input: CommitGenotypingBatchInput,
    ) -> Result<GenotypingBatchReceipt, DesktopDataError> {
        let batch = self
            .store_ref()
            .get_genotyping_batch(input.batch_id)
            .await?;
        ensure_local_batch(&batch)?;
        let source_id = batch.source_attachment_id.ok_or_else(|| {
            StoreError::Conflict("genotyping batch has no confirmed preview".to_owned())
        })?;
        let source = self.genotyping_source_attachment(&batch, source_id).await?;
        let preview = self.parse_genotyping_preview(&batch, &source).await?;
        if !preview.can_confirm()
            || preview.preview_hash != input.preview_hash
            || batch.preview_hash.as_deref() != Some(input.preview_hash.as_str())
        {
            return Err(StoreError::Conflict(
                "genotyping source changed or no longer matches the confirmed preview".to_owned(),
            )
            .into());
        }
        let rows = preview
            .accepted_rows
            .into_iter()
            .map(|row| muriarc_core::GenotypingBatchRecordInput {
                animal_id: row.animal_id,
                state: row.state,
                notes: row.notes,
            })
            .collect();
        let audit = self
            .genotyping_batch_audit("commit_genotyping_batch")
            .await?;
        commit_use_case(
            self.store_ref(),
            CommitGenotypingBatchCommand {
                batch_id: batch.id,
                expected_revision: input.expected_revision,
                preview_hash: input.preview_hash,
                rows,
                now: Utc::now(),
            },
            &audit,
        )
        .await
        .map_err(application_error)
    }

    pub(crate) async fn cancel_genotyping_batch(
        &self,
        input: CancelGenotypingBatchInput,
    ) -> Result<GenotypingBatch, DesktopDataError> {
        let batch = self
            .store_ref()
            .get_genotyping_batch(input.batch_id)
            .await?;
        ensure_local_batch(&batch)?;
        let audit = self
            .genotyping_batch_audit("cancel_genotyping_batch")
            .await?;
        cancel_use_case(
            self.store_ref(),
            CancelGenotypingBatchCommand {
                batch_id: input.batch_id,
                expected_revision: input.expected_revision,
                reason: input.reason,
                now: Utc::now(),
            },
            &audit,
        )
        .await
        .map_err(application_error)
    }

    pub(crate) fn genotyping_batch_template(&self) -> GenotypingBatchTemplateView {
        GenotypingBatchTemplateView {
            file_name: "muriarc-genotyping-batch-template.csv".to_owned(),
            media_type: "text/csv;charset=utf-8".to_owned(),
            bytes: genotyping_template_csv(),
        }
    }

    async fn genotyping_source_attachment(
        &self,
        batch: &GenotypingBatch,
        attachment_id: Uuid,
    ) -> Result<Attachment, DesktopDataError> {
        let attachment = self.store_ref().get_attachment(attachment_id).await?;
        if attachment.lab_id != LOCAL_LAB_ID
            || attachment.project_id != batch.project_id
            || attachment.entity_type != "genotyping_batch"
            || attachment.entity_id != batch.id
            || attachment.meta.deleted_at.is_some()
            || attachment
                .media_type
                .as_deref()
                .is_some_and(|value| value.starts_with("image/"))
        {
            return Err(DesktopDataError::ScopeMismatch);
        }
        Ok(attachment)
    }

    async fn parse_genotyping_preview(
        &self,
        batch: &GenotypingBatch,
        source: &Attachment,
    ) -> Result<GenotypingImportPreview, DesktopDataError> {
        let bytes = self.attachments_ref().read_verified_bytes(source).await?;
        let extension = source
            .file_name
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        let table = match extension.as_str() {
            "csv" => read_csv(Cursor::new(bytes)),
            "xlsx" => read_xlsx(Cursor::new(bytes)),
            _ => {
                return Err(StoreError::Validation(
                    "genotyping result table must be a CSV or XLSX file".to_owned(),
                )
                .into());
            }
        }
        .map_err(|error| {
            StoreError::Validation(format!(
                "genotyping result table could not be parsed: {error}"
            ))
        })?;
        let animals = self
            .store_ref()
            .list_animals(&AnimalFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: batch.project_id,
                ..AnimalFilter::default()
            })
            .await?;
        let directory = AnimalDirectory::from_entries(
            animals
                .into_iter()
                .map(|animal| (animal.display_id, animal.id)),
        )
        .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mapping = GenotypingFieldMapping::infer(&table.headers);
        Ok(preview_genotyping(&table, &mapping, &directory))
    }
}

fn ensure_local_batch(batch: &GenotypingBatch) -> Result<(), DesktopDataError> {
    if batch.lab_id == LOCAL_LAB_ID {
        Ok(())
    } else {
        Err(DesktopDataError::ScopeMismatch)
    }
}

fn application_error(error: ApplicationError) -> DesktopDataError {
    match error {
        ApplicationError::Store(error) => error.into(),
        ApplicationError::Domain(error) => StoreError::Validation(error.to_string()).into(),
        ApplicationError::TooLong { field, max } => {
            StoreError::Validation(format!("{field} must not exceed {max} characters")).into()
        }
        ApplicationError::TooManyBytes { field, max } => {
            StoreError::Validation(format!("{field} must not exceed {max} bytes")).into()
        }
        ApplicationError::Validation(message) => StoreError::Validation(message).into(),
    }
}
