use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use muriarc_core::{
    CageKind, GenotypingState, MeasurementValue, ObservationPolicy, ObservationSubjectType,
    ObservationValueData, ObservationValueType, ParticipationStatus, ProcedureStatus, RecordStatus,
    Sex, TemplateField, TemplateStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Dataset {
    pub schema_version: u32,
    pub dataset_id: String,
    pub synthetic: bool,
    pub fixed_timeline: FixedTimeline,
    pub projects: Vec<ProjectSpec>,
    pub cages: Vec<CageSpec>,
    pub animals: Vec<AnimalSpec>,
    pub genetics: GeneticsSpec,
    pub breeding: BreedingSpec,
    pub templates: Vec<TemplateSpec>,
    pub experiments: Vec<ExperimentSpec>,
    pub cohorts: Vec<CohortSpec>,
    pub participations: Vec<ParticipationSpec>,
    pub procedures: Vec<ProcedureSpec>,
    pub events: Vec<EventSpec>,
    pub observation_definitions: Vec<ObservationDefinitionSpec>,
    pub observations: Vec<ObservationSpec>,
    pub measurements: Vec<MeasurementSpec>,
    pub samples: Vec<SampleSpec>,
    pub attachments: Vec<AttachmentSpec>,
    pub expected_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FixedTimeline {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectSpec {
    pub key: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CageSpec {
    pub key: String,
    pub section: String,
    pub display_id: String,
    pub location: String,
    pub kind: CageKind,
    pub capacity: i32,
    pub sort_order: i32,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AnimalSpec {
    pub key: String,
    pub display_id: String,
    pub sex: Sex,
    pub strain: String,
    pub birth_date: NaiveDate,
    pub legacy_id: Option<String>,
    pub cage: String,
    pub project: String,
    pub source: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub cohort: Option<String>,
    #[serde(default)]
    pub temporary_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeneticsSpec {
    pub locus: LocusSpec,
    pub alleles: Vec<AlleleSpec>,
    pub definitions: Vec<GenotypeDefinitionSpec>,
    pub records: Vec<GenotypingRecordSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocusSpec {
    pub key: String,
    pub symbol: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AlleleSpec {
    pub key: String,
    pub symbol: String,
    pub description: String,
    pub is_wild_type: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenotypeDefinitionSpec {
    pub key: String,
    pub name: String,
    pub description: String,
    pub alleles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GenotypingRecordSpec {
    pub key: String,
    pub animal: String,
    pub definition: String,
    pub state: GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: String,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BreedingSpec {
    pub line: BreedingLineSpec,
    pub colonies: Vec<ColonySpec>,
    pub pairs: Vec<BreedingPairSpec>,
    pub mating: MatingSpec,
    pub litter: LitterSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BreedingLineSpec {
    pub key: String,
    pub name: String,
    pub description: String,
    pub definitions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ColonySpec {
    pub key: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BreedingPairSpec {
    pub key: String,
    pub colony: String,
    pub name: String,
    pub male: String,
    pub females: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub status: String,
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MatingSpec {
    pub key: String,
    pub pair: String,
    pub male: String,
    pub female: String,
    pub occurred_at: DateTime<Utc>,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LitterSpec {
    pub key: String,
    pub pair: String,
    pub mating: String,
    pub born_on: NaiveDate,
    pub size_total: i32,
    pub notes: String,
    pub drafts: Vec<DraftSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftSpec {
    pub temporary_label: String,
    pub sex: Sex,
    pub animal: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TemplateSpec {
    pub key: String,
    pub project: String,
    pub template_key: String,
    pub version: i32,
    pub name: String,
    pub description: String,
    pub status: TemplateStatus,
    pub fields: Vec<TemplateField>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExperimentSpec {
    pub key: String,
    pub project: String,
    pub name: String,
    pub description: String,
    pub template: Option<String>,
    pub status: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohortSpec {
    pub key: String,
    pub experiment: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ParticipationSpec {
    pub key: String,
    pub experiment: String,
    pub animal: String,
    pub cohort: String,
    pub enrolled_at: DateTime<Utc>,
    pub status: ParticipationStatus,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProcedureSpec {
    pub key: String,
    pub experiment: String,
    pub animal: Option<String>,
    pub name: String,
    pub scheduled_at: DateTime<Utc>,
    pub performed_at: Option<DateTime<Utc>>,
    pub status: ProcedureStatus,
    pub details: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EventSpec {
    pub key: String,
    pub experiment: String,
    pub event_key: String,
    pub label: String,
    pub occurred_at: DateTime<Utc>,
    pub details: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObservationDefinitionSpec {
    pub key: String,
    pub experiment: String,
    pub definition_key: String,
    pub label: String,
    pub value_type: ObservationValueType,
    pub unit: Option<String>,
    pub categories: Vec<String>,
    pub policy: ObservationPolicy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObservationSpec {
    pub key: String,
    pub experiment: String,
    pub event: String,
    pub definition: String,
    pub subject_type: ObservationSubjectType,
    pub animal: String,
    pub context: Value,
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub notes: String,
    pub revision: Option<ObservationRevisionSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObservationRevisionSpec {
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MeasurementSpec {
    pub key: String,
    pub project: String,
    pub experiment: String,
    pub animal: String,
    pub measurement_key: String,
    pub label: String,
    pub value: MeasurementValue,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
    pub status: RecordStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SampleSpec {
    pub key: String,
    pub project: String,
    pub experiment: String,
    pub animal: String,
    pub sample_type: String,
    pub quantity: f64,
    pub unit: String,
    pub location: String,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AttachmentSpec {
    pub key: String,
    pub project: String,
    pub target_type: String,
    pub target: String,
    pub file: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FixtureManifest {
    pub schema_version: u32,
    pub dataset_id: String,
    pub dataset_version: String,
    pub synthetic: bool,
    pub fixed_timeline: bool,
    pub public_api_only: bool,
    pub baseline_policy: String,
    pub sandbox_policy: String,
    pub expected_counts: BTreeMap<String, usize>,
    pub files: BTreeMap<String, String>,
    pub known_public_api_limits: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureIds {
    pub projects: BTreeMap<String, Uuid>,
    pub cages: BTreeMap<String, Uuid>,
    pub animals: BTreeMap<String, Uuid>,
    pub assignments: BTreeMap<String, Uuid>,
    pub loci: BTreeMap<String, Uuid>,
    pub alleles: BTreeMap<String, Uuid>,
    pub genotype_definitions: BTreeMap<String, Uuid>,
    pub genotyping_records: BTreeMap<String, Uuid>,
    pub breeding_lines: BTreeMap<String, Uuid>,
    pub colonies: BTreeMap<String, Uuid>,
    pub breeding_pairs: BTreeMap<String, Uuid>,
    pub mating_events: BTreeMap<String, Uuid>,
    pub litters: BTreeMap<String, Uuid>,
    pub animal_drafts: BTreeMap<String, Uuid>,
    pub templates: BTreeMap<String, Uuid>,
    pub experiments: BTreeMap<String, Uuid>,
    pub cohorts: BTreeMap<String, Uuid>,
    pub participations: BTreeMap<String, Uuid>,
    pub procedures: BTreeMap<String, Uuid>,
    pub events: BTreeMap<String, Uuid>,
    pub observation_definitions: BTreeMap<String, Uuid>,
    pub observations: BTreeMap<String, Uuid>,
    pub measurements: BTreeMap<String, Uuid>,
    pub samples: BTreeMap<String, Uuid>,
    pub attachments: BTreeMap<String, Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeedReceipt {
    pub schema_version: u32,
    pub status: String,
    pub dataset_id: String,
    pub dataset_version: String,
    pub dataset_sha256: String,
    pub manifest_sha256: String,
    pub source_commit: String,
    pub application_version: String,
    pub data_epoch: String,
    pub backend: String,
    pub generation_id: Uuid,
    pub expected_counts: BTreeMap<String, usize>,
    pub attachment_files: BTreeMap<String, String>,
    pub ids: FixtureIds,
}
