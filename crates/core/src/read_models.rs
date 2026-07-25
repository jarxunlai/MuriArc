use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Animal, GenotypingBatchStatus, GenotypingRecord, GenotypingState};

/// Project membership attached to an animal through an experiment
/// participation. This is deliberately a compact read model rather than a
/// second source of truth for projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalProjectRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatestAnimalWeight {
    /// Internal optimistic-read identity. API projections may omit these
    /// fields, but AI grouping must retain them until apply-time validation.
    #[serde(default, skip_serializing)]
    pub measurement_id: Uuid,
    #[serde(default, skip_serializing)]
    pub revision: i64,
    pub value: f64,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
}

/// Bounded, list-friendly animal projection shared by SQLite and PostgreSQL.
///
/// Store adapters populate this projection with a fixed number of batched
/// queries. HTTP and desktop callers must not enrich a page one animal at a
/// time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimalOverview {
    pub animal: Animal,
    pub genotype_labels: Vec<String>,
    pub projects: Vec<AnimalProjectRef>,
    pub latest_weight: Option<LatestAnimalWeight>,
}

/// One current Genetics v2 fact enriched for bounded list/query surfaces.
///
/// "Current" is resolved by the Store before applying the optional state
/// filter: for every animal and genotype definition, only the latest
/// non-deleted, non-voided record is eligible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingEvidenceAttachment {
    pub id: Uuid,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub version: i32,
    pub revision: i64,
}

/// Safe, read-only evidence projection for one batch-created genotype fact.
///
/// Storage paths, content hashes, the original result-table attachment and
/// human account identifiers are deliberately excluded. Callers may use the
/// opaque attachment id with an authorized download endpoint, but the model
/// only receives bounded metadata about gel images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingBatchEvidence {
    pub id: Uuid,
    pub batch_number: String,
    pub status: GenotypingBatchStatus,
    pub assessed_at: DateTime<Utc>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub revision: i64,
    pub gel_attachments: Vec<GenotypingEvidenceAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentGenotypingRecordOverview {
    pub record: GenotypingRecord,
    pub animal_display_id: String,
    pub genotype_definition_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_batch: Option<GenotypingBatchEvidence>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentGenotypingRecordFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub animal_id: Option<Uuid>,
    pub state: Option<GenotypingState>,
}
