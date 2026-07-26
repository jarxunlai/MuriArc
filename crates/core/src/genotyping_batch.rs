use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, GenotypingRecord, GenotypingState, RecordMeta};

pub const MAX_GENOTYPING_BATCH_RECORDS: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenotypingBatchStatus {
    Draft,
    Committed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatch {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub batch_number: String,
    pub genotype_definition_id: Uuid,
    pub assessed_at: DateTime<Utc>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub status: GenotypingBatchStatus,
    pub created_by: Option<Uuid>,
    pub source_attachment_id: Option<Uuid>,
    pub preview_hash: Option<String>,
    pub preview_row_count: Option<i64>,
    pub committed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub meta: RecordMeta,
}

impl GenotypingBatch {
    pub fn new(
        lab_id: Uuid,
        project_id: Option<Uuid>,
        batch_number: impl Into<String>,
        genotype_definition_id: Uuid,
        assessed_at: DateTime<Utc>,
        created_by: Option<Uuid>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let batch = Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            batch_number: batch_number.into().trim().to_owned(),
            genotype_definition_id,
            assessed_at,
            method: None,
            notes: None,
            status: GenotypingBatchStatus::Draft,
            created_by,
            source_attachment_id: None,
            preview_hash: None,
            preview_row_count: None,
            committed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            meta: RecordMeta::new(now),
        };
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let preview_valid = match (
            self.source_attachment_id,
            self.preview_hash.as_deref(),
            self.preview_row_count,
        ) {
            (None, None, None) => true,
            (Some(source_id), Some(hash), Some(row_count)) => {
                !source_id.is_nil()
                    && hash.len() == 64
                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && row_count > 0
                    && usize::try_from(row_count)
                        .is_ok_and(|count| count <= MAX_GENOTYPING_BATCH_RECORDS)
            }
            _ => false,
        };
        let lifecycle_valid = match self.status {
            GenotypingBatchStatus::Draft => {
                self.committed_at.is_none()
                    && self.cancelled_at.is_none()
                    && self.cancel_reason.is_none()
            }
            GenotypingBatchStatus::Committed => {
                self.committed_at.is_some()
                    && self.cancelled_at.is_none()
                    && self.cancel_reason.is_none()
                    && self.source_attachment_id.is_some()
            }
            GenotypingBatchStatus::Cancelled => {
                self.committed_at.is_none()
                    && self.cancelled_at.is_some()
                    && self
                        .cancel_reason
                        .as_deref()
                        .is_some_and(|reason| !reason.trim().is_empty())
            }
        };
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.project_id.is_some_and(|id| id.is_nil())
            || self.genotype_definition_id.is_nil()
            || self.created_by.is_some_and(|id| id.is_nil())
            || self.batch_number.trim().is_empty()
            || !preview_valid
            || !lifecycle_valid
        {
            Err(DomainError::InvalidGenotypingBatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatchFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub status: Option<GenotypingBatchStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatchPreview {
    pub source_attachment_id: Uuid,
    pub preview_hash: String,
    pub row_count: i64,
}

impl GenotypingBatchPreview {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.source_attachment_id.is_nil()
            || self.preview_hash.len() != 64
            || !self
                .preview_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.row_count <= 0
            || !usize::try_from(self.row_count)
                .is_ok_and(|count| count <= MAX_GENOTYPING_BATCH_RECORDS)
        {
            Err(DomainError::InvalidGenotypingBatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatchRecordInput {
    pub animal_id: Uuid,
    pub state: GenotypingState,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenotypingBatchCommit {
    pub batch_id: Uuid,
    pub expected_revision: i64,
    pub preview_hash: String,
    pub records: Vec<GenotypingRecord>,
    pub committed_at: DateTime<Utc>,
}

impl GenotypingBatchCommit {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.batch_id.is_nil()
            || self.expected_revision < 1
            || self.preview_hash.len() != 64
            || !self
                .preview_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.records.is_empty()
            || self.records.len() > MAX_GENOTYPING_BATCH_RECORDS
            || self.records.iter().any(|record| record.validate().is_err())
        {
            return Err(DomainError::InvalidGenotypingBatch);
        }
        let mut ids = BTreeSet::new();
        let mut animals = BTreeSet::new();
        for record in &self.records {
            if !ids.insert(record.id) || !animals.insert(record.animal_id) {
                return Err(DomainError::InvalidGenotypingBatch);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatchReceipt {
    pub batch: GenotypingBatch,
    pub records: Vec<GenotypingRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-25T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn batch_requires_consistent_preview_and_lifecycle_fields() {
        let mut batch = GenotypingBatch::new(
            Uuid::new_v4(),
            None,
            "PCR-20260725-01",
            Uuid::new_v4(),
            now(),
            Some(Uuid::new_v4()),
            now(),
        )
        .unwrap();
        batch.source_attachment_id = Some(Uuid::new_v4());
        assert_eq!(batch.validate(), Err(DomainError::InvalidGenotypingBatch));

        batch.preview_hash = Some("a".repeat(64));
        batch.preview_row_count = Some(12);
        assert!(batch.validate().is_ok());

        batch.status = GenotypingBatchStatus::Committed;
        assert_eq!(batch.validate(), Err(DomainError::InvalidGenotypingBatch));
        batch.committed_at = Some(now());
        assert!(batch.validate().is_ok());
    }

    #[test]
    fn commit_rejects_duplicate_animals() {
        let lab_id = Uuid::new_v4();
        let animal_id = Uuid::new_v4();
        let definition_id = Uuid::new_v4();
        let record = GenotypingRecord::new(
            lab_id,
            animal_id,
            definition_id,
            GenotypingState::Confirmed,
            Some(now()),
            now(),
        )
        .unwrap();
        let command = GenotypingBatchCommit {
            batch_id: Uuid::new_v4(),
            expected_revision: 1,
            preview_hash: "b".repeat(64),
            records: vec![
                record.clone(),
                GenotypingRecord {
                    id: Uuid::new_v4(),
                    ..record
                },
            ],
            committed_at: now(),
        };
        assert_eq!(command.validate(), Err(DomainError::InvalidGenotypingBatch));
    }
}
