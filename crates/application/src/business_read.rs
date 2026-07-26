use std::{collections::BTreeSet, sync::Arc};

use chrono::{DateTime, Utc};
use muriarc_core::{
    ActorType, Allele, AnimalDraft, AnimalEvent, AnimalEventKind, AnimalFilter, AnimalOverview,
    AnimalStatus, Attachment, AuditAction, AuditEntry, AuditFilter, BreedingLine, BreedingPair,
    BreedingPairStatus, Cage, Cohort, Colony, CurrentGenotypingRecordFilter,
    CurrentGenotypingRecordOverview, EntityType, Experiment, ExperimentEvent, ExperimentFilter,
    ExperimentStatus, ExperimentTemplateVersion, FieldValueType, GeneLocus, GenotypeDefinition,
    GenotypingRecord, GenotypingState, Job, JobFilter, JobKind, JobStatus, Litter, MatingEvent,
    Measurement, MeasurementFilter, MeasurementValue, MuriArcStore, Observation,
    ObservationDefinition, ObservationFilter, ObservationSubjectType, ObservationValueData,
    ObservationValueRecord, Participation, ParticipationFilter, Pedigree, Procedure,
    ProcedureStatus, Project, ProjectAnimalAssignment, ProjectAnimalAssignmentFilter,
    ProjectStatus, Provenance, ProvenanceFilter, ProvenanceSource, RecordStatus, Sample,
    SampleFilter, StoreError, TemplateField, TemplateStatus, WriteSource,
    is_ai_managed_attachment_entity_type, is_private_ai_source_attachment_audit,
    is_private_ai_source_job_audit, protect_public_audit_entries,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 100;
const MAX_OFFSET: u32 = 10_000;

/// Read authorization resolved from the authenticated application context.
///
/// Model arguments never construct this value. Project membership narrows the
/// set; lab-registry access permits only the explicitly handled lab-level
/// animal, breeding, attachment, job, and shared-library projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessReadAccess {
    lab_id: Uuid,
    allowed_project_ids: BTreeSet<Uuid>,
    lab_registry_read: bool,
    activity_read: bool,
    audit_read: bool,
    current_user_id: Option<Uuid>,
}

impl BusinessReadAccess {
    pub fn new(lab_id: Uuid, allowed_project_ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            lab_id,
            allowed_project_ids: allowed_project_ids.into_iter().collect(),
            lab_registry_read: false,
            activity_read: false,
            audit_read: false,
            current_user_id: None,
        }
    }

    pub const fn with_lab_registry_read(mut self, allowed: bool) -> Self {
        self.lab_registry_read = allowed;
        self
    }

    pub const fn lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub fn allowed_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.allowed_project_ids
    }

    pub fn allows_project(&self, project_id: Uuid) -> bool {
        self.allowed_project_ids.contains(&project_id)
    }

    pub const fn can_read_lab_registry(&self) -> bool {
        self.lab_registry_read
    }

    /// Enables the safe activity projection only. This must be derived from a
    /// live `ReadActivity` permission and never from model arguments.
    pub const fn with_activity_read(mut self, allowed: bool) -> Self {
        self.activity_read = allowed;
        self
    }

    /// Enables the separately advertised audit tool. This must be derived from
    /// a live `ReadAudit` permission and defaults to false.
    pub const fn with_audit_read(mut self, allowed: bool) -> Self {
        self.audit_read = allowed;
        self
    }

    pub const fn can_read_activity(&self) -> bool {
        self.activity_read
    }

    pub const fn can_read_audit(&self) -> bool {
        self.audit_read
    }

    /// Binds actor-owned operational resources to the authenticated user.
    pub const fn with_current_user(mut self, user_id: Uuid) -> Self {
        self.current_user_id = Some(user_id);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessResource {
    Animals,
    GenotypingRecords,
    GeneLoci,
    Alleles,
    GenotypeDefinitions,
    GenotypingHistory,
    Projects,
    Cages,
    Experiments,
    Measurements,
    Samples,
    BreedingLines,
    Colonies,
    BreedingPairs,
    MatingEvents,
    Litters,
    Pedigrees,
    Cohorts,
    Procedures,
    ExperimentEvents,
    ObservationDefinitions,
    Observations,
    ObservationValues,
    Participations,
    AnimalDrafts,
    Attachments,
    Library,
    Jobs,
    Activity,
    Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadPageRequest {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

impl Default for ReadPageRequest {
    fn default() -> Self {
        Self {
            limit: Some(DEFAULT_LIMIT),
            offset: Some(0),
        }
    }
}

impl ReadPageRequest {
    fn checked(&self) -> Result<CheckedPage, BusinessReadError> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        let offset = self.offset.unwrap_or(0);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(BusinessReadError::Rejected("limit_out_of_range"));
        }
        if offset > MAX_OFFSET {
            return Err(BusinessReadError::Rejected("offset_out_of_range"));
        }
        Ok(CheckedPage { offset, limit })
    }
}

#[derive(Debug, Clone, Copy)]
struct CheckedPage {
    offset: u32,
    limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPage {
    pub offset: u32,
    pub limit: u32,
    pub returned: usize,
    pub complete: bool,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordScope {
    Lab,
    Project { project_id: Uuid },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadPermissionState {
    Granted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadEnvelope<T> {
    pub items: Vec<T>,
    pub page: ReadPage,
    pub record_scope: RecordScope,
    pub permission_state: ReadPermissionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessSourceRef {
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
}

impl BusinessSourceRef {
    pub const fn new(entity_type: EntityType, entity_id: Uuid, revision: Option<i64>) -> Self {
        Self {
            entity_type,
            entity_id,
            revision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessReadResult<T> {
    pub data: T,
    pub sources: Vec<BusinessSourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSearchRequest {
    pub resource: BusinessResource,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub animal_id: Option<Uuid>,
    #[serde(default)]
    pub experiment_id: Option<Uuid>,
    #[serde(default)]
    pub experiment_event_id: Option<Uuid>,
    #[serde(default)]
    pub breeding_line_id: Option<Uuid>,
    #[serde(default)]
    pub colony_id: Option<Uuid>,
    #[serde(default)]
    pub breeding_pair_id: Option<Uuid>,
    #[serde(default)]
    pub mating_event_id: Option<Uuid>,
    #[serde(default)]
    pub observation_id: Option<Uuid>,
    #[serde(default)]
    pub observation_subject_id: Option<Uuid>,
    #[serde(default)]
    pub locus_id: Option<Uuid>,
    #[serde(default)]
    pub cohort_id: Option<Uuid>,
    #[serde(default)]
    pub litter_id: Option<Uuid>,
    #[serde(default)]
    pub cage_id: Option<Uuid>,
    #[serde(default)]
    pub animal_status: Option<AnimalStatus>,
    #[serde(default)]
    pub project_status: Option<ProjectStatus>,
    #[serde(default)]
    pub experiment_status: Option<ExperimentStatus>,
    #[serde(default)]
    pub genotyping_state: Option<GenotypingState>,
    #[serde(default)]
    pub breeding_pair_status: Option<BreedingPairStatus>,
    #[serde(default)]
    pub procedure_status: Option<ProcedureStatus>,
    #[serde(default)]
    pub observation_subject_type: Option<ObservationSubjectType>,
    #[serde(default)]
    pub template_status: Option<TemplateStatus>,
    #[serde(default)]
    pub job_kind: Option<JobKind>,
    #[serde(default)]
    pub job_status: Option<JobStatus>,
    #[serde(default)]
    pub provenance_source: Option<ProvenanceSource>,
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default)]
    pub entity_id: Option<Uuid>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub measurement_key: Option<String>,
    #[serde(default)]
    pub sample_type: Option<String>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

impl ResourceSearchRequest {
    pub fn new(resource: BusinessResource) -> Self {
        Self {
            resource,
            project_id: None,
            animal_id: None,
            experiment_id: None,
            experiment_event_id: None,
            breeding_line_id: None,
            colony_id: None,
            breeding_pair_id: None,
            mating_event_id: None,
            observation_id: None,
            observation_subject_id: None,
            locus_id: None,
            cohort_id: None,
            litter_id: None,
            cage_id: None,
            animal_status: None,
            project_status: None,
            experiment_status: None,
            genotyping_state: None,
            breeding_pair_status: None,
            procedure_status: None,
            observation_subject_type: None,
            template_status: None,
            job_kind: None,
            job_status: None,
            provenance_source: None,
            entity_type: None,
            entity_id: None,
            query: None,
            measurement_key: None,
            sample_type: None,
            page: ReadPageRequest::default(),
        }
    }
}

/// Focused Genetics v2 query exposed as a first-class assistant tool.
///
/// It intentionally returns only the current effective record for each
/// animal/definition pair. Historical records remain available through the
/// explicitly animal-scoped `genotyping_history` resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenotypingQueryRequest {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub animal_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<GenotypingState>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "resource", content = "result", rename_all = "snake_case")]
pub enum ResourceSearchResult {
    Animals(ReadEnvelope<AnimalOverview>),
    GenotypingRecords(ReadEnvelope<CurrentGenotypingRecordOverview>),
    GeneLoci(ReadEnvelope<GeneLocus>),
    Alleles(ReadEnvelope<Allele>),
    GenotypeDefinitions(ReadEnvelope<GenotypeDefinition>),
    GenotypingHistory(ReadEnvelope<GenotypingRecord>),
    Projects(ReadEnvelope<Project>),
    Cages(ReadEnvelope<Cage>),
    Experiments(ReadEnvelope<Experiment>),
    Measurements(ReadEnvelope<MeasurementReadView>),
    Samples(ReadEnvelope<Sample>),
    BreedingLines(ReadEnvelope<BreedingLine>),
    Colonies(ReadEnvelope<Colony>),
    BreedingPairs(ReadEnvelope<BreedingPair>),
    MatingEvents(ReadEnvelope<MatingEvent>),
    Litters(ReadEnvelope<Litter>),
    Pedigrees(ReadEnvelope<Pedigree>),
    Cohorts(ReadEnvelope<Cohort>),
    Procedures(ReadEnvelope<Procedure>),
    ExperimentEvents(ReadEnvelope<ExperimentEvent>),
    ObservationDefinitions(ReadEnvelope<ObservationDefinition>),
    Observations(ReadEnvelope<Observation>),
    ObservationValues(ReadEnvelope<ObservationValueReadView>),
    Participations(ReadEnvelope<Participation>),
    AnimalDrafts(ReadEnvelope<AnimalDraft>),
    Attachments(ReadEnvelope<AttachmentReadView>),
    Library(ReadEnvelope<ExperimentTemplateReadView>),
    Jobs(ReadEnvelope<JobReadView>),
    Activity(ReadEnvelope<AuditReadView>),
    Provenance(ReadEnvelope<ProvenanceReadView>),
}

/// Project assignment fields needed for scope and lifecycle interpretation.
/// Human account identifiers and deleted-record internals never cross the
/// model boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAnimalAssignmentReadView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub animal_id: Uuid,
    pub reason: Option<String>,
    pub assigned_at: DateTime<Utc>,
    pub revision: i64,
}

impl From<ProjectAnimalAssignment> for ProjectAnimalAssignmentReadView {
    fn from(value: ProjectAnimalAssignment) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            animal_id: value.animal_id,
            reason: value.reason,
            assigned_at: value.meta.created_at,
            revision: value.meta.revision,
        }
    }
}

/// Animal timeline projection without the recorder account identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalEventReadView {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub kind: AnimalEventKind,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    pub notes: Option<String>,
}

impl From<AnimalEvent> for AnimalEventReadView {
    fn from(value: AnimalEvent) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            animal_id: value.animal_id,
            kind: value.kind,
            occurred_at: value.occurred_at,
            recorded_at: value.recorded_at,
            notes: value.notes,
        }
    }
}

/// Scientific measurement data without the signing account identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementReadView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub procedure_id: Option<Uuid>,
    pub key: String,
    pub label: String,
    pub value_type: FieldValueType,
    pub value: MeasurementValue,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
    pub status: RecordStatus,
    pub signed_at: Option<DateTime<Utc>>,
    pub revision: i64,
}

impl From<Measurement> for MeasurementReadView {
    fn from(value: Measurement) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            experiment_id: value.experiment_id,
            animal_id: value.animal_id,
            procedure_id: value.procedure_id,
            key: value.key,
            label: value.label,
            value_type: value.value_type,
            value: value.value,
            unit: value.unit,
            measured_at: value.measured_at,
            status: value.status,
            signed_at: value.signed_at,
            revision: value.meta.revision,
        }
    }
}

/// Versioned observation value without the recorder account identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueReadView {
    pub id: Uuid,
    pub observation_id: Uuid,
    pub version: i32,
    pub value: ObservationValueData,
    pub recorded_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub revision: i64,
}

impl From<ObservationValueRecord> for ObservationValueReadView {
    fn from(value: ObservationValueRecord) -> Self {
        Self {
            id: value.id,
            observation_id: value.observation_id,
            version: value.version,
            value: value.value,
            recorded_at: value.recorded_at,
            notes: value.notes,
            revision: value.meta.revision,
        }
    }
}

/// Shared experiment template data without the publishing account identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentTemplateReadView {
    pub id: Uuid,
    pub template_key: String,
    pub version: i32,
    pub name: String,
    pub description: Option<String>,
    pub status: TemplateStatus,
    pub fields: Vec<TemplateField>,
    pub published_at: Option<DateTime<Utc>>,
    pub revision: i64,
}

impl From<ExperimentTemplateVersion> for ExperimentTemplateReadView {
    fn from(value: ExperimentTemplateVersion) -> Self {
        Self {
            id: value.id,
            template_key: value.template_key,
            version: value.version,
            name: value.name,
            description: value.description,
            status: value.status,
            fields: value.fields,
            published_at: value.published_at,
            revision: value.meta.revision,
        }
    }
}

/// Provenance fields required to explain origin and confidence. Account,
/// import, tool-run, provider, model and request identifiers stay private.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceReadView {
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub source: ProvenanceSource,
    pub confidence: Option<f64>,
    pub recorded_at: DateTime<Utc>,
}

impl From<Provenance> for ProvenanceReadView {
    fn from(value: Provenance) -> Self {
        Self {
            entity_type: value.entity_type,
            entity_id: value.entity_id,
            source: value.source,
            confidence: value.confidence,
            recorded_at: value.recorded_at,
        }
    }
}

/// Attachment metadata safe for model consumption. Storage paths and digests
/// are deliberately omitted because neither is business data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentReadView {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub version: i32,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Attachment> for AttachmentReadView {
    fn from(value: Attachment) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            entity_type: value.entity_type,
            entity_id: value.entity_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            version: value.version,
            revision: value.meta.revision,
            created_at: value.meta.created_at,
            updated_at: value.meta.updated_at,
        }
    }
}

/// Job progress without idempotency keys, result payloads, error payloads, or
/// other operational details that may contain paths or imported values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReadView {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: JobKind,
    pub status: JobStatus,
    pub progress_current: i64,
    pub progress_total: Option<i64>,
    pub result_available: bool,
    pub error_report_available: bool,
    pub cancellation_requested: bool,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Job> for JobReadView {
    fn from(value: Job) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            kind: value.kind,
            status: value.status,
            progress_current: value.progress_current,
            progress_total: value.progress_total,
            result_available: value.result.is_some(),
            error_report_available: value.error_report.is_some(),
            cancellation_requested: value.cancellation_requested,
            revision: value.meta.revision,
            created_at: value.meta.created_at,
            updated_at: value.meta.updated_at,
        }
    }
}

/// Shared safe projection for activity and audit tools. Snapshot values,
/// operation parameters, free-form reasons, request identifiers, and actor user
/// identifiers never cross the model boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReadView {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub action: AuditAction,
    pub actor_type: ActorType,
    pub source: WriteSource,
    pub operation_code: String,
    pub operation_version: i32,
    pub entity_revision: Option<i64>,
    pub before_available: bool,
    pub after_available: bool,
    pub occurred_at: DateTime<Utc>,
}

impl From<AuditEntry> for AuditReadView {
    fn from(value: AuditEntry) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            entity_type: value.entity_type,
            entity_id: value.entity_id,
            action: value.action,
            actor_type: value.actor.actor_type,
            source: value.source,
            operation_code: value.operation_code,
            operation_version: value.operation_version,
            entity_revision: value.entity_revision,
            before_available: value.before.is_some(),
            after_available: value.after.is_some(),
            occurred_at: value.occurred_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditQueryRequest {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default)]
    pub entity_id: Option<Uuid>,
    #[serde(default)]
    pub action: Option<AuditAction>,
    #[serde(default)]
    pub source: Option<WriteSource>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityQueryRequest {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default)]
    pub entity_id: Option<Uuid>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceQueryRequest {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default)]
    pub entity_id: Option<Uuid>,
    #[serde(default)]
    pub source: Option<ProvenanceSource>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimalContextRequest {
    pub animal_id: Uuid,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimalContext {
    pub animal: muriarc_core::Animal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cage: Option<Cage>,
    pub assignments: Vec<ProjectAnimalAssignmentReadView>,
    pub current_genotyping_records: Vec<CurrentGenotypingRecordOverview>,
    pub genotyping_history_count: usize,
    pub events: ReadEnvelope<AnimalEventReadView>,
    pub participations: ReadEnvelope<Participation>,
    pub measurements: ReadEnvelope<MeasurementReadView>,
    pub samples: ReadEnvelope<Sample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectContextRequest {
    pub project_id: Uuid,
    #[serde(flatten)]
    pub page: ReadPageRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectContext {
    pub project: Project,
    pub animals: ReadEnvelope<AnimalOverview>,
    pub cages: ReadEnvelope<Cage>,
    pub experiments: ReadEnvelope<Experiment>,
    pub current_genotyping_records: ReadEnvelope<CurrentGenotypingRecordOverview>,
}

#[derive(Debug, Error)]
pub enum BusinessReadError {
    #[error("business read rejected: {0}")]
    Rejected(&'static str),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl BusinessReadError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Rejected(code) => code,
            Self::Store(_) => "business_read_unavailable",
        }
    }
}

#[derive(Clone)]
pub struct BusinessReadService {
    store: Arc<dyn MuriArcStore>,
    access: BusinessReadAccess,
}

impl BusinessReadService {
    pub fn new(store: Arc<dyn MuriArcStore>, access: BusinessReadAccess) -> Self {
        Self { store, access }
    }

    pub fn access(&self) -> &BusinessReadAccess {
        &self.access
    }

    async fn protect_model_provenance(
        &self,
        project_id: Option<Uuid>,
        entity_id: Option<Uuid>,
        items: &mut Vec<Provenance>,
    ) -> Result<(), BusinessReadError> {
        let audits = self
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: self.access.lab_id,
                project_id,
                entity_id,
            })
            .await?;
        let hidden_entities = audits
            .iter()
            .filter(|entry| {
                is_private_ai_source_attachment_audit(entry)
                    || is_private_ai_source_job_audit(entry)
            })
            .map(|entry| (entry.entity_type.as_str(), entry.entity_id))
            .collect::<BTreeSet<_>>();
        items.retain(|item| {
            item.entity_type != EntityType::AiConversationSource
                && !hidden_entities.contains(&(item.entity_type.as_str(), item.entity_id))
        });
        Ok(())
    }

    async fn authorize_project(&self, project_id: Uuid) -> Result<Project, BusinessReadError> {
        if !self.access.allows_project(project_id) {
            return Err(BusinessReadError::Rejected("project_forbidden"));
        }
        let project = self.store.get_project(project_id).await?;
        if project.lab_id != self.access.lab_id {
            return Err(BusinessReadError::Rejected("project_forbidden"));
        }
        Ok(project)
    }

    async fn authorize_animal(
        &self,
        animal_id: Uuid,
        project_id: Option<Uuid>,
    ) -> Result<muriarc_core::Animal, BusinessReadError> {
        if let Some(project_id) = project_id {
            self.authorize_project(project_id).await?;
        } else if !self.access.lab_registry_read {
            return Err(BusinessReadError::Rejected("project_required"));
        }
        let animal = self.store.get_animal(animal_id).await?;
        if animal.lab_id != self.access.lab_id {
            return Err(BusinessReadError::Rejected("animal_forbidden"));
        }
        if let Some(project_id) = project_id {
            let assignments = self
                .store
                .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                    lab_id: self.access.lab_id,
                    project_id: Some(project_id),
                    animal_id: Some(animal.id),
                })
                .await?;
            if assignments.is_empty() {
                return Err(BusinessReadError::Rejected("animal_forbidden"));
            }
        }
        Ok(animal)
    }

    fn require_lab_registry(&self) -> Result<(), BusinessReadError> {
        if self.access.lab_registry_read {
            Ok(())
        } else {
            Err(BusinessReadError::Rejected("lab_registry_forbidden"))
        }
    }

    async fn authorize_experiment(
        &self,
        project_id: Uuid,
        experiment_id: Uuid,
    ) -> Result<Experiment, BusinessReadError> {
        self.authorize_project(project_id).await?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        if experiment.lab_id != self.access.lab_id || experiment.project_id != project_id {
            return Err(BusinessReadError::Rejected("experiment_forbidden"));
        }
        Ok(experiment)
    }

    async fn authorize_registry_scope(
        &self,
        project_id: Option<Uuid>,
        animal_id: Option<Uuid>,
    ) -> Result<(), BusinessReadError> {
        match project_id {
            Some(project_id) => {
                self.authorize_project(project_id).await?;
                if let Some(animal_id) = animal_id {
                    self.authorize_animal(animal_id, Some(project_id)).await?;
                }
            }
            None => {
                self.require_lab_registry()?;
                if let Some(animal_id) = animal_id {
                    self.authorize_animal(animal_id, None).await?;
                }
            }
        }
        Ok(())
    }

    fn scope(&self, project_id: Option<Uuid>) -> RecordScope {
        project_id.map_or(RecordScope::Lab, |project_id| RecordScope::Project {
            project_id,
        })
    }

    pub async fn genotyping_query(
        &self,
        request: GenotypingQueryRequest,
    ) -> Result<BusinessReadResult<ReadEnvelope<CurrentGenotypingRecordOverview>>, BusinessReadError>
    {
        let mut search = ResourceSearchRequest::new(BusinessResource::GenotypingRecords);
        search.project_id = request.project_id;
        search.animal_id = request.animal_id;
        search.genotyping_state = request.state;
        search.page = request.page;
        let result = self.resource_search(search).await?;
        let ResourceSearchResult::GenotypingRecords(data) = result.data else {
            unreachable!("the fixed genotyping resource must return current Genetics v2 records");
        };
        Ok(BusinessReadResult {
            data,
            sources: result.sources,
        })
    }

    pub async fn resource_search(
        &self,
        request: ResourceSearchRequest,
    ) -> Result<BusinessReadResult<ResourceSearchResult>, BusinessReadError> {
        let page = request.page.checked()?;
        if request
            .query
            .as_deref()
            .is_some_and(|query| query.chars().count() > 256)
        {
            return Err(BusinessReadError::Rejected("query_too_long"));
        }
        let project = match request.project_id {
            Some(project_id) => Some(self.authorize_project(project_id).await?),
            None => None,
        };

        match request.resource {
            BusinessResource::Animals => {
                if project.is_none() && !self.access.lab_registry_read {
                    return Err(BusinessReadError::Rejected("project_required"));
                }
                let mut items = self
                    .store
                    .list_animal_overviews(
                        &AnimalFilter {
                            lab_id: self.access.lab_id,
                            project_id: request.project_id,
                            cage_id: request.cage_id,
                            status: request.animal_status,
                            query: request.query,
                        },
                        page.offset,
                        page.limit + 1,
                    )
                    .await?;
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Animal,
                            item.animal.id,
                            Some(item.animal.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Animals(envelope),
                    sources,
                })
            }
            BusinessResource::GenotypingRecords => {
                if project.is_none() && !self.access.lab_registry_read {
                    return Err(BusinessReadError::Rejected("project_required"));
                }
                if let Some(animal_id) = request.animal_id {
                    self.authorize_animal(animal_id, request.project_id).await?;
                }
                let mut items = self
                    .store
                    .list_current_genotyping_record_overviews(
                        &CurrentGenotypingRecordFilter {
                            lab_id: self.access.lab_id,
                            project_id: request.project_id,
                            animal_id: request.animal_id,
                            state: request.genotyping_state,
                        },
                        page.offset,
                        page.limit + 1,
                    )
                    .await?;
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .flat_map(genotyping_overview_sources)
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::GenotypingRecords(envelope),
                    sources,
                })
            }
            BusinessResource::GeneLoci => {
                self.authorize_registry_scope(request.project_id, request.animal_id)
                    .await?;
                let mut items = self.store.list_gene_loci(self.access.lab_id).await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.meta.deleted_at.is_none()
                        && (matches_query(&item.symbol, request.query.as_deref())
                            || item.description.as_deref().is_some_and(|description| {
                                matches_query(description, request.query.as_deref())
                            }))
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::GeneLocus,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::GeneLoci(envelope),
                    sources,
                })
            }
            BusinessResource::Alleles => {
                self.authorize_registry_scope(request.project_id, request.animal_id)
                    .await?;
                let locus_id = request
                    .locus_id
                    .ok_or(BusinessReadError::Rejected("locus_required"))?;
                let locus = self.store.get_gene_locus(locus_id).await?;
                if locus.lab_id != self.access.lab_id || locus.meta.deleted_at.is_some() {
                    return Err(BusinessReadError::Rejected("locus_forbidden"));
                }
                let mut items = self.store.list_alleles(locus_id).await?;
                items.retain(|item| {
                    item.locus_id == locus_id
                        && item.meta.deleted_at.is_none()
                        && (matches_query(&item.symbol, request.query.as_deref())
                            || item.description.as_deref().is_some_and(|description| {
                                matches_query(description, request.query.as_deref())
                            }))
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Allele,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Alleles(envelope),
                    sources,
                })
            }
            BusinessResource::GenotypeDefinitions => {
                self.authorize_registry_scope(request.project_id, request.animal_id)
                    .await?;
                let mut items = self
                    .store
                    .list_genotype_definitions(self.access.lab_id)
                    .await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.meta.deleted_at.is_none()
                        && (matches_query(&item.name, request.query.as_deref())
                            || item.description.as_deref().is_some_and(|description| {
                                matches_query(description, request.query.as_deref())
                            }))
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::GenotypeDefinition,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::GenotypeDefinitions(envelope),
                    sources,
                })
            }
            BusinessResource::GenotypingHistory => {
                let animal_id = request
                    .animal_id
                    .ok_or(BusinessReadError::Rejected("animal_required"))?;
                self.authorize_registry_scope(request.project_id, Some(animal_id))
                    .await?;
                let mut items = self.store.list_genotyping_records(animal_id).await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.animal_id == animal_id
                        && item.meta.deleted_at.is_none()
                        && request.project_id.is_none_or(|project_id| {
                            item.project_id.is_none_or(|id| id == project_id)
                        })
                        && request
                            .genotyping_state
                            .is_none_or(|state| item.state == state)
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::GenotypingRecord,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::GenotypingHistory(envelope),
                    sources,
                })
            }
            BusinessResource::Projects => {
                reject_irrelevant_project(request.project_id)?;
                let mut items = self.store.list_projects(self.access.lab_id).await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && self.access.allows_project(item.id)
                        && request
                            .project_status
                            .is_none_or(|status| item.status == status)
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Project,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Projects(envelope),
                    sources,
                })
            }
            BusinessResource::Cages => {
                let mut items = if let Some(project_id) = request.project_id {
                    self.store
                        .list_cages_for_project(self.access.lab_id, project_id)
                        .await?
                } else {
                    if !self.access.lab_registry_read {
                        return Err(BusinessReadError::Rejected("project_required"));
                    }
                    self.store.list_cages(self.access.lab_id).await?
                };
                items.retain(|item| item.lab_id == self.access.lab_id);
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(EntityType::Cage, item.id, Some(item.meta.revision))
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Cages(envelope),
                    sources,
                })
            }
            BusinessResource::Experiments => {
                let project_id = request
                    .project_id
                    .ok_or(BusinessReadError::Rejected("project_required"))?;
                let mut items = self
                    .store
                    .list_experiments(&ExperimentFilter {
                        project_id,
                        status: request.experiment_status,
                    })
                    .await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id && item.project_id == project_id
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Experiment,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Experiments(envelope),
                    sources,
                })
            }
            BusinessResource::Measurements => {
                let project_id = request
                    .project_id
                    .ok_or(BusinessReadError::Rejected("project_required"))?;
                let mut items = self
                    .store
                    .list_measurements(&MeasurementFilter {
                        project_id,
                        experiment_id: request.experiment_id,
                        animal_id: request.animal_id,
                    })
                    .await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.project_id == project_id
                        && request.measurement_key.as_deref().is_none_or(|key| {
                            !key.trim().is_empty() && item.key.eq_ignore_ascii_case(key.trim())
                        })
                });
                let mut items = slice_page(
                    items.into_iter().map(MeasurementReadView::from).collect(),
                    page,
                );
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Measurement,
                            item.id,
                            Some(item.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Measurements(envelope),
                    sources,
                })
            }
            BusinessResource::Samples => {
                let project_id = request
                    .project_id
                    .ok_or(BusinessReadError::Rejected("project_required"))?;
                let mut items = self
                    .store
                    .list_samples(&SampleFilter {
                        project_id,
                        experiment_id: request.experiment_id,
                        animal_id: request.animal_id,
                    })
                    .await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.project_id == project_id
                        && request.sample_type.as_deref().is_none_or(|sample_type| {
                            !sample_type.trim().is_empty()
                                && item.sample_type.eq_ignore_ascii_case(sample_type.trim())
                        })
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Sample,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Samples(envelope),
                    sources,
                })
            }
            BusinessResource::BreedingLines => {
                self.require_lab_registry()?;
                let mut items = self.store.list_breeding_lines(self.access.lab_id).await?;
                items.retain(|item| matches_query(&item.name, request.query.as_deref()));
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::BreedingLine,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::BreedingLines(envelope),
                    sources,
                })
            }
            BusinessResource::Colonies => {
                self.require_lab_registry()?;
                let mut items = self
                    .store
                    .list_colonies(self.access.lab_id, request.breeding_line_id)
                    .await?;
                items.retain(|item| matches_query(&item.name, request.query.as_deref()));
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Colony,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Colonies(envelope),
                    sources,
                })
            }
            BusinessResource::BreedingPairs => {
                self.require_lab_registry()?;
                let mut items = self
                    .store
                    .list_breeding_pairs(self.access.lab_id, request.colony_id)
                    .await?;
                items.retain(|item| {
                    request
                        .breeding_pair_status
                        .is_none_or(|status| item.status == status)
                        && matches_query(&item.name, request.query.as_deref())
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::BreedingPair,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::BreedingPairs(envelope),
                    sources,
                })
            }
            BusinessResource::MatingEvents => {
                self.require_lab_registry()?;
                let breeding_pair_id = request
                    .breeding_pair_id
                    .ok_or(BusinessReadError::Rejected("breeding_pair_required"))?;
                let pair = self.store.get_breeding_pair(breeding_pair_id).await?;
                if pair.lab_id != self.access.lab_id {
                    return Err(BusinessReadError::Rejected("breeding_pair_forbidden"));
                }
                let items = self.store.list_mating_events(breeding_pair_id).await?;
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::MatingEvent,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::MatingEvents(envelope),
                    sources,
                })
            }
            BusinessResource::Litters => {
                self.require_lab_registry()?;
                let breeding_pair_id = request
                    .breeding_pair_id
                    .ok_or(BusinessReadError::Rejected("breeding_pair_required"))?;
                let pair = self.store.get_breeding_pair(breeding_pair_id).await?;
                if pair.lab_id != self.access.lab_id {
                    return Err(BusinessReadError::Rejected("breeding_pair_forbidden"));
                }
                let mut items = self.store.list_litters(breeding_pair_id).await?;
                items.retain(|item| {
                    request
                        .mating_event_id
                        .is_none_or(|event_id| item.mating_event_id == event_id)
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Litter,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Litters(envelope),
                    sources,
                })
            }
            BusinessResource::Pedigrees => {
                let animal_id = request
                    .animal_id
                    .ok_or(BusinessReadError::Rejected("animal_required"))?;
                self.authorize_animal(animal_id, request.project_id).await?;
                let mut items = self.store.list_related_pedigrees(animal_id).await?;
                if let Some(project_id) = request.project_id {
                    let visible_animal_ids = self
                        .store
                        .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                            lab_id: self.access.lab_id,
                            project_id: Some(project_id),
                            animal_id: None,
                        })
                        .await?
                        .into_iter()
                        .map(|assignment| assignment.animal_id)
                        .collect::<BTreeSet<_>>();
                    items.retain(|pedigree| {
                        visible_animal_ids.contains(&pedigree.animal_id)
                            && visible_animal_ids.contains(&pedigree.parent_id)
                    });
                }
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Pedigree,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Pedigrees(envelope),
                    sources,
                })
            }
            BusinessResource::Cohorts => {
                let (project_id, experiment_id) = required_project_experiment(&request)?;
                self.authorize_experiment(project_id, experiment_id).await?;
                let mut items = self.store.list_cohorts(experiment_id).await?;
                items.retain(|item| matches_query(&item.name, request.query.as_deref()));
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Cohort,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Cohorts(envelope),
                    sources,
                })
            }
            BusinessResource::Procedures => {
                let (project_id, experiment_id) = required_project_experiment(&request)?;
                self.authorize_experiment(project_id, experiment_id).await?;
                if let Some(animal_id) = request.animal_id {
                    self.authorize_animal(animal_id, Some(project_id)).await?;
                }
                let mut items = self
                    .store
                    .list_procedures(experiment_id, request.animal_id)
                    .await?;
                items.retain(|item| {
                    request
                        .procedure_status
                        .is_none_or(|status| item.status == status)
                        && matches_query(&item.name, request.query.as_deref())
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Procedure,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Procedures(envelope),
                    sources,
                })
            }
            BusinessResource::ExperimentEvents => {
                let (project_id, experiment_id) = required_project_experiment(&request)?;
                self.authorize_experiment(project_id, experiment_id).await?;
                let mut items = self.store.list_experiment_events(experiment_id).await?;
                items.retain(|item| {
                    matches_query(&item.label, request.query.as_deref())
                        || matches_query(&item.event_key, request.query.as_deref())
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::ExperimentEvent,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::ExperimentEvents(envelope),
                    sources,
                })
            }
            BusinessResource::ObservationDefinitions => {
                let (project_id, experiment_id) = required_project_experiment(&request)?;
                self.authorize_experiment(project_id, experiment_id).await?;
                let mut items = self
                    .store
                    .list_observation_definitions(experiment_id)
                    .await?;
                items.retain(|item| {
                    matches_query(&item.label, request.query.as_deref())
                        || matches_query(&item.key, request.query.as_deref())
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::ObservationDefinition,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::ObservationDefinitions(envelope),
                    sources,
                })
            }
            BusinessResource::Observations => {
                let (project_id, experiment_id) = required_project_experiment(&request)?;
                self.authorize_experiment(project_id, experiment_id).await?;
                let subject_id = request.observation_subject_id.or_else(|| {
                    (request.observation_subject_type == Some(ObservationSubjectType::Animal))
                        .then_some(request.animal_id)
                        .flatten()
                });
                if request.observation_subject_type == Some(ObservationSubjectType::Animal) {
                    let animal_id =
                        subject_id.ok_or(BusinessReadError::Rejected("animal_required"))?;
                    self.authorize_animal(animal_id, Some(project_id)).await?;
                }
                let mut items = self
                    .store
                    .list_observations(&ObservationFilter {
                        experiment_id,
                        experiment_event_id: request.experiment_event_id,
                        subject_type: request.observation_subject_type,
                        subject_id,
                    })
                    .await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id && item.project_id == project_id
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Observation,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Observations(envelope),
                    sources,
                })
            }
            BusinessResource::ObservationValues => {
                let observation_id = request
                    .observation_id
                    .ok_or(BusinessReadError::Rejected("observation_required"))?;
                let observation = self.store.get_observation(observation_id).await?;
                if observation.lab_id != self.access.lab_id {
                    return Err(BusinessReadError::Rejected("observation_forbidden"));
                }
                if request
                    .project_id
                    .is_some_and(|project_id| project_id != observation.project_id)
                {
                    return Err(BusinessReadError::Rejected("observation_forbidden"));
                }
                self.authorize_project(observation.project_id).await?;
                let items = self
                    .store
                    .list_observation_values(observation_id)
                    .await?
                    .into_iter()
                    .map(ObservationValueReadView::from)
                    .collect();
                let mut items = slice_page(items, page);
                let envelope = envelope(
                    &mut items,
                    page,
                    RecordScope::Project {
                        project_id: observation.project_id,
                    },
                );
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::ObservationValue,
                            item.id,
                            Some(item.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::ObservationValues(envelope),
                    sources,
                })
            }
            BusinessResource::Participations => {
                let project_id = request
                    .project_id
                    .ok_or(BusinessReadError::Rejected("project_required"))?;
                self.authorize_project(project_id).await?;
                let cohort_id = request
                    .cohort_id
                    .ok_or(BusinessReadError::Rejected("cohort_required"))?;
                if let Some(experiment_id) = request.experiment_id {
                    self.authorize_experiment(project_id, experiment_id).await?;
                    if !self
                        .store
                        .list_cohorts(experiment_id)
                        .await?
                        .iter()
                        .any(|cohort| cohort.id == cohort_id)
                    {
                        return Err(BusinessReadError::Rejected("cohort_forbidden"));
                    }
                }
                if let Some(animal_id) = request.animal_id {
                    self.authorize_animal(animal_id, Some(project_id)).await?;
                }
                let mut items = self
                    .store
                    .list_participations(&ParticipationFilter {
                        project_id,
                        experiment_id: request.experiment_id,
                        animal_id: request.animal_id,
                        cohort_id: Some(cohort_id),
                    })
                    .await?;
                items.retain(|item| item.cohort_id == Some(cohort_id));
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Project { project_id });
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::Participation,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Participations(envelope),
                    sources,
                })
            }
            BusinessResource::AnimalDrafts => {
                self.require_lab_registry()?;
                if request.project_id.is_some() {
                    return Err(BusinessReadError::Rejected(
                        "project_filter_not_valid_for_resource",
                    ));
                }
                let litter_id = request
                    .litter_id
                    .ok_or(BusinessReadError::Rejected("litter_required"))?;
                let litter = self.store.get_litter(litter_id).await?;
                if litter.lab_id != self.access.lab_id || litter.meta.deleted_at.is_some() {
                    return Err(BusinessReadError::Rejected("litter_forbidden"));
                }
                let mut items = self.store.list_animal_drafts(litter_id).await?;
                items.retain(|item| {
                    item.lab_id == self.access.lab_id
                        && item.litter_id == litter_id
                        && item.meta.deleted_at.is_none()
                        && matches_query(&item.temporary_label, request.query.as_deref())
                });
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, RecordScope::Lab);
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::AnimalDraft,
                            item.id,
                            Some(item.meta.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::AnimalDrafts(envelope),
                    sources,
                })
            }
            BusinessResource::Attachments => {
                let items = if let Some(project_id) = request.project_id {
                    self.authorize_project(project_id).await?;
                    self.store
                        .list_project_attachments(self.access.lab_id, project_id)
                        .await?
                } else {
                    self.require_lab_registry()?;
                    self.store.list_lab_attachments(self.access.lab_id).await?
                };
                let items = items
                    .into_iter()
                    .filter(|item| {
                        !is_ai_managed_attachment_entity_type(&item.entity_type)
                            && request
                                .entity_type
                                .is_none_or(|entity_type| item.entity_type == entity_type.as_str())
                            && request
                                .entity_id
                                .is_none_or(|entity_id| item.entity_id == entity_id)
                            && matches_query(&item.file_name, request.query.as_deref())
                    })
                    .map(AttachmentReadView::from)
                    .collect::<Vec<_>>();
                let mut items = slice_page(items, page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(EntityType::Attachment, item.id, Some(item.revision))
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Attachments(envelope),
                    sources,
                })
            }
            BusinessResource::Library => {
                if let Some(project_id) = request.project_id {
                    self.authorize_project(project_id).await?;
                } else if !self.access.lab_registry_read
                    && self.access.allowed_project_ids.is_empty()
                {
                    return Err(BusinessReadError::Rejected("project_required"));
                }
                let mut items = self
                    .store
                    .list_template_versions(self.access.lab_id, None)
                    .await?;
                items.retain(|item| {
                    (self.access.lab_registry_read || item.status == TemplateStatus::Published)
                        && request
                            .template_status
                            .is_none_or(|status| item.status == status)
                        && (matches_query(&item.name, request.query.as_deref())
                            || matches_query(&item.template_key, request.query.as_deref()))
                });
                let mut items = slice_page(
                    items
                        .into_iter()
                        .map(ExperimentTemplateReadView::from)
                        .collect(),
                    page,
                );
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            EntityType::ExperimentTemplateVersion,
                            item.id,
                            Some(item.revision),
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Library(envelope),
                    sources,
                })
            }
            BusinessResource::Jobs => {
                let current_user_id = self
                    .access
                    .current_user_id
                    .ok_or(BusinessReadError::Rejected("job_owner_required"))?;
                if let Some(project_id) = request.project_id {
                    self.authorize_project(project_id).await?;
                } else {
                    self.require_lab_registry()?;
                }
                let mut items = self
                    .store
                    .list_jobs(&JobFilter {
                        lab_id: self.access.lab_id,
                        project_id: request.project_id,
                        created_by: Some(current_user_id),
                    })
                    .await?;
                items.retain(|item| {
                    request.job_kind.is_none_or(|kind| item.kind == kind)
                        && request
                            .job_status
                            .is_none_or(|status| item.status == status)
                });
                items.reverse();
                let mut items =
                    slice_page(items.into_iter().map(JobReadView::from).collect(), page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(EntityType::Job, item.id, Some(item.revision))
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Jobs(envelope),
                    sources,
                })
            }
            BusinessResource::Activity => {
                if !self.access.activity_read {
                    return Err(BusinessReadError::Rejected("activity_forbidden"));
                }
                if let Some(project_id) = request.project_id {
                    self.authorize_project(project_id).await?;
                } else {
                    self.require_lab_registry()?;
                }
                let mut items = self
                    .store
                    .list_audit_entries(&AuditFilter {
                        lab_id: self.access.lab_id,
                        project_id: request.project_id,
                        entity_id: request.entity_id,
                    })
                    .await?;
                protect_public_audit_entries(&mut items);
                items.reverse();
                items.retain(|item| {
                    is_key_activity(item)
                        && request
                            .entity_type
                            .is_none_or(|entity_type| item.entity_type == entity_type)
                        && matches_query(&item.operation_code, request.query.as_deref())
                });
                let mut items =
                    slice_page(items.into_iter().map(AuditReadView::from).collect(), page);
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| {
                        BusinessSourceRef::new(
                            item.entity_type,
                            item.entity_id,
                            item.entity_revision,
                        )
                    })
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Activity(envelope),
                    sources,
                })
            }
            BusinessResource::Provenance => {
                if !self.access.audit_read {
                    return Err(BusinessReadError::Rejected("audit_forbidden"));
                }
                if let Some(project_id) = request.project_id {
                    self.authorize_project(project_id).await?;
                } else {
                    self.require_lab_registry()?;
                }
                let mut items = self
                    .store
                    .list_provenance(&ProvenanceFilter {
                        lab_id: self.access.lab_id,
                        project_id: request.project_id,
                        entity_type: request.entity_type,
                        entity_id: request.entity_id,
                        source: request.provenance_source,
                    })
                    .await?;
                self.protect_model_provenance(request.project_id, request.entity_id, &mut items)
                    .await?;
                items.reverse();
                let mut items = slice_page(
                    items.into_iter().map(ProvenanceReadView::from).collect(),
                    page,
                );
                let envelope = envelope(&mut items, page, self.scope(request.project_id));
                let sources = envelope
                    .items
                    .iter()
                    .map(|item| BusinessSourceRef::new(item.entity_type, item.entity_id, None))
                    .collect();
                Ok(BusinessReadResult {
                    data: ResourceSearchResult::Provenance(envelope),
                    sources,
                })
            }
        }
    }

    pub async fn activity_query(
        &self,
        request: ActivityQueryRequest,
    ) -> Result<BusinessReadResult<ReadEnvelope<AuditReadView>>, BusinessReadError> {
        if !self.access.activity_read {
            return Err(BusinessReadError::Rejected("activity_forbidden"));
        }
        let page = request.page.checked()?;
        if request
            .query
            .as_deref()
            .is_some_and(|query| query.chars().count() > 256)
        {
            return Err(BusinessReadError::Rejected("query_too_long"));
        }
        if let Some(project_id) = request.project_id {
            self.authorize_project(project_id).await?;
        } else {
            self.require_lab_registry()?;
        }
        let mut items = self
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: self.access.lab_id,
                project_id: request.project_id,
                entity_id: request.entity_id,
            })
            .await?;
        protect_public_audit_entries(&mut items);
        items.reverse();
        items.retain(|item| {
            is_key_activity(item)
                && request
                    .entity_type
                    .is_none_or(|entity_type| item.entity_type == entity_type)
                && matches_query(&item.operation_code, request.query.as_deref())
        });
        let mut items = slice_page(items.into_iter().map(AuditReadView::from).collect(), page);
        let envelope = envelope(&mut items, page, self.scope(request.project_id));
        let sources = envelope
            .items
            .iter()
            .map(|item| {
                BusinessSourceRef::new(item.entity_type, item.entity_id, item.entity_revision)
            })
            .collect();
        Ok(BusinessReadResult {
            data: envelope,
            sources,
        })
    }

    pub async fn provenance_query(
        &self,
        request: ProvenanceQueryRequest,
    ) -> Result<BusinessReadResult<ReadEnvelope<ProvenanceReadView>>, BusinessReadError> {
        if !self.access.audit_read {
            return Err(BusinessReadError::Rejected("audit_forbidden"));
        }
        let page = request.page.checked()?;
        if let Some(project_id) = request.project_id {
            self.authorize_project(project_id).await?;
        } else {
            self.require_lab_registry()?;
        }
        let mut items = self
            .store
            .list_provenance(&ProvenanceFilter {
                lab_id: self.access.lab_id,
                project_id: request.project_id,
                entity_type: request.entity_type,
                entity_id: request.entity_id,
                source: request.source,
            })
            .await?;
        self.protect_model_provenance(request.project_id, request.entity_id, &mut items)
            .await?;
        items.reverse();
        let mut items = slice_page(
            items.into_iter().map(ProvenanceReadView::from).collect(),
            page,
        );
        let envelope = envelope(&mut items, page, self.scope(request.project_id));
        let sources = envelope
            .items
            .iter()
            .map(|item| BusinessSourceRef::new(item.entity_type, item.entity_id, None))
            .collect();
        Ok(BusinessReadResult {
            data: envelope,
            sources,
        })
    }

    pub async fn audit_query(
        &self,
        request: AuditQueryRequest,
    ) -> Result<BusinessReadResult<ReadEnvelope<AuditReadView>>, BusinessReadError> {
        if !self.access.audit_read {
            return Err(BusinessReadError::Rejected("audit_forbidden"));
        }
        let page = request.page.checked()?;
        if let Some(project_id) = request.project_id {
            self.authorize_project(project_id).await?;
        } else {
            self.require_lab_registry()?;
        }
        let mut items = self
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: self.access.lab_id,
                project_id: request.project_id,
                entity_id: request.entity_id,
            })
            .await?;
        protect_public_audit_entries(&mut items);
        items.reverse();
        items.retain(|item| {
            request
                .entity_type
                .is_none_or(|entity_type| item.entity_type == entity_type)
                && request.action.is_none_or(|action| item.action == action)
                && request.source.is_none_or(|source| item.source == source)
        });
        let mut items = slice_page(items.into_iter().map(AuditReadView::from).collect(), page);
        let envelope = envelope(&mut items, page, self.scope(request.project_id));
        let sources = envelope
            .items
            .iter()
            .map(|item| {
                BusinessSourceRef::new(item.entity_type, item.entity_id, item.entity_revision)
            })
            .collect();
        Ok(BusinessReadResult {
            data: envelope,
            sources,
        })
    }

    pub async fn animal_context(
        &self,
        request: AnimalContextRequest,
    ) -> Result<BusinessReadResult<AnimalContext>, BusinessReadError> {
        let page = request.page.checked()?;
        let animal = self
            .authorize_animal(request.animal_id, request.project_id)
            .await?;
        let assignments = self
            .store
            .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                lab_id: self.access.lab_id,
                project_id: request.project_id,
                animal_id: Some(animal.id),
            })
            .await?
            .into_iter()
            .filter(|assignment| self.access.allows_project(assignment.project_id))
            .map(ProjectAnimalAssignmentReadView::from)
            .collect::<Vec<_>>();
        let cage = match animal.current_cage_id {
            Some(cage_id) => {
                let visible = request.project_id.is_none_or(|project_id| {
                    assignments
                        .iter()
                        .any(|assignment| assignment.project_id == project_id)
                });
                if visible {
                    let cage = self.store.get_cage(cage_id).await?;
                    (cage.lab_id == self.access.lab_id).then_some(cage)
                } else {
                    None
                }
            }
            None => None,
        };

        let current_genotyping_records = self
            .store
            .list_current_genotyping_record_overviews(
                &CurrentGenotypingRecordFilter {
                    lab_id: self.access.lab_id,
                    project_id: request.project_id,
                    animal_id: Some(animal.id),
                    state: None,
                },
                0,
                MAX_LIMIT,
            )
            .await?;
        let genotyping_history_count = self
            .store
            .list_genotyping_records(animal.id)
            .await?
            .into_iter()
            .filter(|record| {
                record.lab_id == self.access.lab_id
                    && record.meta.deleted_at.is_none()
                    && request.project_id.is_none_or(|project_id| {
                        record.project_id.is_none_or(|id| id == project_id)
                    })
            })
            .count();

        let mut events = self.store.list_animal_events(animal.id).await?;
        events.retain(|event| {
            event.lab_id == self.access.lab_id
                && match request.project_id {
                    Some(project_id) => event.project_id == Some(project_id),
                    None => event
                        .project_id
                        .is_none_or(|project_id| self.access.allows_project(project_id)),
                }
        });
        let mut events = slice_page(
            events.into_iter().map(AnimalEventReadView::from).collect(),
            page,
        );
        let events = envelope(&mut events, page, self.scope(request.project_id));

        let (participations, measurements, samples) = if let Some(project_id) = request.project_id {
            let mut participations = self
                .store
                .list_participations(&ParticipationFilter {
                    project_id,
                    experiment_id: None,
                    animal_id: Some(animal.id),
                    cohort_id: None,
                })
                .await?;
            let mut measurements: Vec<_> = self
                .store
                .list_measurements(&MeasurementFilter {
                    project_id,
                    experiment_id: None,
                    animal_id: Some(animal.id),
                })
                .await?
                .into_iter()
                .map(MeasurementReadView::from)
                .collect();
            let mut samples = self
                .store
                .list_samples(&SampleFilter {
                    project_id,
                    experiment_id: None,
                    animal_id: Some(animal.id),
                })
                .await?;
            (
                envelope(
                    &mut slice_page_in_place(&mut participations, page),
                    page,
                    RecordScope::Project { project_id },
                ),
                envelope(
                    &mut slice_page_in_place(&mut measurements, page),
                    page,
                    RecordScope::Project { project_id },
                ),
                envelope(
                    &mut slice_page_in_place(&mut samples, page),
                    page,
                    RecordScope::Project { project_id },
                ),
            )
        } else {
            (
                empty_envelope(page, RecordScope::Lab),
                empty_envelope(page, RecordScope::Lab),
                empty_envelope(page, RecordScope::Lab),
            )
        };

        let mut sources = vec![BusinessSourceRef::new(
            EntityType::Animal,
            animal.id,
            Some(animal.meta.revision),
        )];
        if let Some(cage) = &cage {
            sources.push(BusinessSourceRef::new(
                EntityType::Cage,
                cage.id,
                Some(cage.meta.revision),
            ));
        }
        sources.extend(assignments.iter().map(|item| {
            BusinessSourceRef::new(
                EntityType::ProjectAnimalAssignment,
                item.id,
                Some(item.revision),
            )
        }));
        sources.extend(
            current_genotyping_records
                .iter()
                .flat_map(genotyping_overview_sources),
        );
        sources.extend(
            events
                .items
                .iter()
                .map(|item| BusinessSourceRef::new(EntityType::AnimalEvent, item.id, None)),
        );
        sources.extend(participations.items.iter().map(|item| {
            BusinessSourceRef::new(EntityType::Participation, item.id, Some(item.meta.revision))
        }));
        sources.extend(measurements.items.iter().map(|item| {
            BusinessSourceRef::new(EntityType::Measurement, item.id, Some(item.revision))
        }));
        sources.extend(samples.items.iter().map(|item| {
            BusinessSourceRef::new(EntityType::Sample, item.id, Some(item.meta.revision))
        }));

        Ok(BusinessReadResult {
            data: AnimalContext {
                animal,
                cage,
                assignments,
                current_genotyping_records,
                genotyping_history_count,
                events,
                participations,
                measurements,
                samples,
            },
            sources,
        })
    }

    pub async fn project_context(
        &self,
        request: ProjectContextRequest,
    ) -> Result<BusinessReadResult<ProjectContext>, BusinessReadError> {
        let page = request.page.checked()?;
        let project = self.authorize_project(request.project_id).await?;
        let mut animals = self
            .store
            .list_animal_overviews(
                &AnimalFilter {
                    lab_id: self.access.lab_id,
                    project_id: Some(project.id),
                    ..AnimalFilter::default()
                },
                page.offset,
                page.limit + 1,
            )
            .await?;
        let animals = envelope(
            &mut animals,
            page,
            RecordScope::Project {
                project_id: project.id,
            },
        );
        let mut cages = self
            .store
            .list_cages_for_project(self.access.lab_id, project.id)
            .await?;
        let cages = envelope(
            &mut slice_page_in_place(&mut cages, page),
            page,
            RecordScope::Project {
                project_id: project.id,
            },
        );
        let mut experiments = self
            .store
            .list_experiments(&ExperimentFilter {
                project_id: project.id,
                status: None,
            })
            .await?;
        let experiments = envelope(
            &mut slice_page_in_place(&mut experiments, page),
            page,
            RecordScope::Project {
                project_id: project.id,
            },
        );
        let mut current_genotyping_records = self
            .store
            .list_current_genotyping_record_overviews(
                &CurrentGenotypingRecordFilter {
                    lab_id: self.access.lab_id,
                    project_id: Some(project.id),
                    animal_id: None,
                    state: None,
                },
                page.offset,
                page.limit + 1,
            )
            .await?;
        let current_genotyping_records = envelope(
            &mut current_genotyping_records,
            page,
            RecordScope::Project {
                project_id: project.id,
            },
        );

        let mut sources = vec![BusinessSourceRef::new(
            EntityType::Project,
            project.id,
            Some(project.meta.revision),
        )];
        sources.extend(animals.items.iter().map(|item| {
            BusinessSourceRef::new(
                EntityType::Animal,
                item.animal.id,
                Some(item.animal.meta.revision),
            )
        }));
        sources.extend(cages.items.iter().map(|item| {
            BusinessSourceRef::new(EntityType::Cage, item.id, Some(item.meta.revision))
        }));
        sources.extend(experiments.items.iter().map(|item| {
            BusinessSourceRef::new(EntityType::Experiment, item.id, Some(item.meta.revision))
        }));
        sources.extend(
            current_genotyping_records
                .items
                .iter()
                .flat_map(genotyping_overview_sources),
        );

        Ok(BusinessReadResult {
            data: ProjectContext {
                project,
                animals,
                cages,
                experiments,
                current_genotyping_records,
            },
            sources,
        })
    }
}

fn genotyping_overview_sources(item: &CurrentGenotypingRecordOverview) -> Vec<BusinessSourceRef> {
    let mut sources = vec![BusinessSourceRef::new(
        EntityType::GenotypingRecord,
        item.record.id,
        Some(item.record.meta.revision),
    )];
    if let Some(batch) = &item.source_batch {
        sources.push(BusinessSourceRef::new(
            EntityType::GenotypingBatch,
            batch.id,
            Some(batch.revision),
        ));
        sources.extend(batch.gel_attachments.iter().map(|attachment| {
            BusinessSourceRef::new(
                EntityType::Attachment,
                attachment.id,
                Some(attachment.revision),
            )
        }));
    }
    sources
}

fn required_project_experiment(
    request: &ResourceSearchRequest,
) -> Result<(Uuid, Uuid), BusinessReadError> {
    Ok((
        request
            .project_id
            .ok_or(BusinessReadError::Rejected("project_required"))?,
        request
            .experiment_id
            .ok_or(BusinessReadError::Rejected("experiment_required"))?,
    ))
}

fn matches_query(value: &str, query: Option<&str>) -> bool {
    query.is_none_or(|query| {
        let query = query.trim();
        !query.is_empty() && value.to_lowercase().contains(&query.to_lowercase())
    })
}

fn is_key_activity(entry: &AuditEntry) -> bool {
    if matches!(
        entry.entity_type,
        EntityType::TechnicalLogPolicy
            | EntityType::AiPrivateImage
            | EntityType::AiConversationSource
            | EntityType::AiConversation
            | EntityType::AiConversationMessage
    ) {
        return false;
    }
    matches!(
        entry.entity_type,
        EntityType::AnimalEvent
            | EntityType::ProjectAnimalAssignment
            | EntityType::BreedingPair
            | EntityType::MatingEvent
            | EntityType::Litter
            | EntityType::AnimalDraft
            | EntityType::Experiment
            | EntityType::Participation
            | EntityType::Procedure
            | EntityType::Measurement
            | EntityType::Sample
            | EntityType::Attachment
            | EntityType::Job
            | EntityType::Approval
    ) || matches!(
        entry.action,
        AuditAction::SoftDelete
            | AuditAction::Sign
            | AuditAction::Import
            | AuditAction::Export
            | AuditAction::Cleanup
    )
}

fn reject_irrelevant_project(project_id: Option<Uuid>) -> Result<(), BusinessReadError> {
    if project_id.is_some() {
        Err(BusinessReadError::Rejected(
            "project_filter_not_valid_for_resource",
        ))
    } else {
        Ok(())
    }
}

fn slice_page<T>(items: Vec<T>, page: CheckedPage) -> Vec<T> {
    items
        .into_iter()
        .skip(page.offset as usize)
        .take(page.limit as usize + 1)
        .collect()
}

fn slice_page_in_place<T>(items: &mut Vec<T>, page: CheckedPage) -> Vec<T> {
    slice_page(std::mem::take(items), page)
}

fn envelope<T>(
    items: &mut Vec<T>,
    page: CheckedPage,
    record_scope: RecordScope,
) -> ReadEnvelope<T> {
    let has_more = items.len() > page.limit as usize;
    if has_more {
        items.truncate(page.limit as usize);
    }
    ReadEnvelope {
        page: ReadPage {
            offset: page.offset,
            limit: page.limit,
            returned: items.len(),
            complete: !has_more,
            has_more,
            next_offset: has_more.then_some(page.offset.saturating_add(page.limit)),
        },
        items: std::mem::take(items),
        record_scope,
        permission_state: ReadPermissionState::Granted,
    }
}

fn empty_envelope<T>(page: CheckedPage, record_scope: RecordScope) -> ReadEnvelope<T> {
    ReadEnvelope {
        items: Vec::new(),
        page: ReadPage {
            offset: page.offset,
            limit: page.limit,
            returned: 0,
            complete: true,
            has_more: false,
            next_offset: None,
        },
        record_scope,
        permission_state: ReadPermissionState::Granted,
    }
}
