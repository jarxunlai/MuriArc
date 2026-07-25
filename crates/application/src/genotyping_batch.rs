use chrono::{DateTime, Utc};
use muriarc_core::{
    AuditContext, GenotypingBatch, GenotypingBatchCommit, GenotypingBatchPreview,
    GenotypingBatchReceipt, GenotypingBatchRecordInput, GenotypingBatchStatus, GenotypingRecord,
    MuriArcStore,
};
use uuid::Uuid;

use crate::genetics::{MAX_GENOTYPING_METHOD_BYTES, MAX_GENOTYPING_NOTES_BYTES};
use crate::validation::{
    normalized_optional_bytes, normalized_required, normalized_required_bytes,
};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_GENOTYPING_BATCH_NUMBER_CHARS: usize = 128;
pub const MAX_GENOTYPING_BATCH_NOTES_BYTES: usize = 8_000;
pub const MAX_GENOTYPING_BATCH_CANCEL_REASON_BYTES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGenotypingBatchCommand {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub batch_number: String,
    pub genotype_definition_id: Uuid,
    pub assessed_at: DateTime<Utc>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub created_by: Option<Uuid>,
    pub now: DateTime<Utc>,
}

pub async fn create_genotyping_batch(
    store: &dyn MuriArcStore,
    command: CreateGenotypingBatchCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypingBatch> {
    let definition = store
        .get_genotype_definition(command.genotype_definition_id)
        .await?;
    if definition.lab_id != command.lab_id {
        return Err(ApplicationError::Validation(
            "genotyping batch definition belongs to a different lab".to_owned(),
        ));
    }
    if definition.meta.deleted_at.is_some() {
        return Err(ApplicationError::Validation(
            "genotyping batch definition is archived".to_owned(),
        ));
    }
    let batch_number = normalized_required(
        "genotyping_batch.batch_number",
        command.batch_number,
        MAX_GENOTYPING_BATCH_NUMBER_CHARS,
    )?;
    let mut batch = GenotypingBatch::new(
        command.lab_id,
        command.project_id,
        batch_number,
        command.genotype_definition_id,
        command.assessed_at,
        command.created_by,
        command.now,
    )?;
    batch.method = normalized_optional_bytes(
        "genotyping_batch.method",
        command.method,
        MAX_GENOTYPING_METHOD_BYTES,
    )?;
    batch.notes = normalized_optional_bytes(
        "genotyping_batch.notes",
        command.notes,
        MAX_GENOTYPING_BATCH_NOTES_BYTES,
    )?;
    batch.validate()?;
    store.create_genotyping_batch(&batch, audit).await?;
    Ok(batch)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetGenotypingBatchPreviewCommand {
    pub batch_id: Uuid,
    pub expected_revision: i64,
    pub source_attachment_id: Uuid,
    pub preview_hash: String,
    pub row_count: i64,
    pub now: DateTime<Utc>,
}

pub async fn set_genotyping_batch_preview(
    store: &dyn MuriArcStore,
    command: SetGenotypingBatchPreviewCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypingBatch> {
    let preview = GenotypingBatchPreview {
        source_attachment_id: command.source_attachment_id,
        preview_hash: command.preview_hash,
        row_count: command.row_count,
    };
    preview.validate()?;
    Ok(store
        .set_genotyping_batch_preview(
            command.batch_id,
            command.expected_revision,
            &preview,
            command.now,
            audit,
        )
        .await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitGenotypingBatchCommand {
    pub batch_id: Uuid,
    pub expected_revision: i64,
    pub preview_hash: String,
    pub rows: Vec<GenotypingBatchRecordInput>,
    pub now: DateTime<Utc>,
}

pub async fn commit_genotyping_batch(
    store: &dyn MuriArcStore,
    command: CommitGenotypingBatchCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypingBatchReceipt> {
    let batch = store.get_genotyping_batch(command.batch_id).await?;
    if batch.status != GenotypingBatchStatus::Draft {
        return Err(ApplicationError::Validation(
            "only a draft genotyping batch can be committed".to_owned(),
        ));
    }
    if batch.meta.revision != command.expected_revision {
        return Err(ApplicationError::Store(muriarc_core::StoreError::Conflict(
            "genotyping batch revision does not match".to_owned(),
        )));
    }
    let mut records = Vec::with_capacity(command.rows.len());
    for row in command.rows {
        let mut record = GenotypingRecord::new(
            batch.lab_id,
            row.animal_id,
            batch.genotype_definition_id,
            row.state,
            Some(batch.assessed_at),
            command.now,
        )?;
        record.project_id = batch.project_id;
        record.method = batch.method.clone();
        record.notes = normalized_optional_bytes(
            "genotyping_record.notes",
            row.notes,
            MAX_GENOTYPING_NOTES_BYTES,
        )?;
        record.validate()?;
        records.push(record);
    }
    Ok(store
        .commit_genotyping_batch(
            &GenotypingBatchCommit {
                batch_id: batch.id,
                expected_revision: command.expected_revision,
                preview_hash: command.preview_hash,
                records,
                committed_at: command.now,
            },
            audit,
        )
        .await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelGenotypingBatchCommand {
    pub batch_id: Uuid,
    pub expected_revision: i64,
    pub reason: String,
    pub now: DateTime<Utc>,
}

pub async fn cancel_genotyping_batch(
    store: &dyn MuriArcStore,
    command: CancelGenotypingBatchCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypingBatch> {
    let reason = normalized_required_bytes(
        "genotyping_batch.cancel_reason",
        command.reason,
        MAX_GENOTYPING_BATCH_CANCEL_REASON_BYTES,
    )?;
    Ok(store
        .cancel_genotyping_batch(
            command.batch_id,
            command.expected_revision,
            &reason,
            command.now,
            audit,
        )
        .await?)
}
