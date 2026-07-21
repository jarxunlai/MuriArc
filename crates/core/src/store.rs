use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    Animal, AnimalEvent, AnimalOverview, AnimalStatus, AnimalTransfer, AuditContext, AuditEntry,
    Cage, Experiment, ExperimentStatus, ImportCommitOptions, ImportCommitResult, ImportPlan, Lab,
    Measurement, Membership, Participation, ParticipationStatus, Pedigree, Project, Provenance,
    ProvenanceSource, Sample, User,
};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{entity} {id} was not found")]
    NotFound { entity: &'static str, id: Uuid },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub cage_id: Option<Uuid>,
    pub status: Option<AnimalStatus>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFilter {
    pub lab_id: Uuid,
    pub status: Option<crate::UserStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipFilter {
    pub lab_id: Uuid,
    pub user_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentFilter {
    pub project_id: Uuid,
    pub status: Option<ExperimentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipationFilter {
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Option<Uuid>,
    pub cohort_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementFilter {
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleFilter {
    pub project_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub animal_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceFilter {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub entity_type: Option<crate::EntityType>,
    pub entity_id: Option<Uuid>,
    pub source: Option<ProvenanceSource>,
}

/// Persistence boundary shared by the local SQLite and shared PostgreSQL modes.
///
/// Callers construct validated domain entities. Implementations must apply each
/// write and its audit entry in one database transaction.
#[async_trait]
pub trait MuriArcStore: crate::WorkspaceStore + Send + Sync {
    async fn migrate(&self) -> StoreResult<()>;
    async fn health_check(&self) -> StoreResult<()>;

    async fn create_lab(&self, lab: &Lab, audit: &AuditContext) -> StoreResult<()>;
    async fn get_lab(&self, id: Uuid) -> StoreResult<Lab>;
    async fn update_lab(
        &self,
        lab: &Lab,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn create_user(&self, user: &User, audit: &AuditContext) -> StoreResult<()>;
    async fn get_user(&self, id: Uuid) -> StoreResult<User>;
    async fn list_users(&self, filter: &UserFilter) -> StoreResult<Vec<User>>;
    async fn update_user(
        &self,
        user: &User,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn create_membership(
        &self,
        membership: &Membership,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_membership(&self, id: Uuid) -> StoreResult<Membership>;
    async fn list_memberships(&self, filter: &MembershipFilter) -> StoreResult<Vec<Membership>>;
    async fn update_membership(
        &self,
        membership: &Membership,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn soft_delete_membership(
        &self,
        id: Uuid,
        expected_revision: i64,
        deleted_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Membership>;

    async fn create_project(&self, project: &Project, audit: &AuditContext) -> StoreResult<()>;
    async fn get_project(&self, id: Uuid) -> StoreResult<Project>;
    async fn list_projects(&self, lab_id: Uuid) -> StoreResult<Vec<Project>>;

    async fn create_cage(&self, cage: &Cage, audit: &AuditContext) -> StoreResult<()>;
    async fn get_cage(&self, id: Uuid) -> StoreResult<Cage>;
    async fn list_cages(&self, lab_id: Uuid) -> StoreResult<Vec<Cage>>;

    async fn create_animal(&self, animal: &Animal, audit: &AuditContext) -> StoreResult<()>;
    async fn create_animal_with_genotyping_records(
        &self,
        animal: &Animal,
        genotyping_records: &[crate::GenotypingRecord],
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_animal(&self, id: Uuid) -> StoreResult<Animal>;
    async fn list_animals(&self, filter: &AnimalFilter) -> StoreResult<Vec<Animal>>;
    /// Lists one bounded page of animals with batched genotype, project and
    /// latest-weight summaries. limit must be non-zero; adapters reject
    /// unreasonably large pages instead of issuing unbounded list queries.
    async fn list_animal_overviews(
        &self,
        filter: &AnimalFilter,
        offset: u32,
        limit: u32,
    ) -> StoreResult<Vec<AnimalOverview>>;
    /// Loads a bounded set of animals without one query per pedigree edge.
    async fn list_animals_by_ids(
        &self,
        lab_id: Uuid,
        project_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> StoreResult<Vec<Animal>>;
    async fn append_animal_event(
        &self,
        event: &AnimalEvent,
        audit: &AuditContext,
    ) -> StoreResult<Animal>;
    async fn list_animal_events(&self, animal_id: Uuid) -> StoreResult<Vec<AnimalEvent>>;
    async fn transfer_animals(
        &self,
        transfer: &AnimalTransfer,
        audit: &AuditContext,
    ) -> StoreResult<Vec<Animal>>;

    async fn create_experiment(
        &self,
        experiment: &Experiment,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_experiment(&self, id: Uuid) -> StoreResult<Experiment>;
    async fn list_experiments(&self, filter: &ExperimentFilter) -> StoreResult<Vec<Experiment>>;
    async fn transition_experiment(
        &self,
        id: Uuid,
        target: ExperimentStatus,
        expected_revision: i64,
        occurred_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Experiment>;

    /// Enrolls an animal and captures its current per-definition genotyping
    /// state in the same transaction.
    async fn create_participation(
        &self,
        participation: &Participation,
        audit: &AuditContext,
    ) -> StoreResult<Participation>;
    async fn get_participation(&self, id: Uuid) -> StoreResult<Participation>;
    async fn list_participations(
        &self,
        filter: &ParticipationFilter,
    ) -> StoreResult<Vec<Participation>>;
    async fn transition_participation(
        &self,
        id: Uuid,
        target: ParticipationStatus,
        expected_revision: i64,
        occurred_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Participation>;

    async fn create_measurement(
        &self,
        measurement: &Measurement,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_measurement(&self, id: Uuid) -> StoreResult<Measurement>;
    async fn list_measurements(&self, filter: &MeasurementFilter) -> StoreResult<Vec<Measurement>>;
    async fn update_measurement(
        &self,
        measurement: &Measurement,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    async fn create_sample(&self, sample: &Sample, audit: &AuditContext) -> StoreResult<()>;
    async fn get_sample(&self, id: Uuid) -> StoreResult<Sample>;
    async fn list_samples(&self, filter: &SampleFilter) -> StoreResult<Vec<Sample>>;

    async fn create_gene_locus(
        &self,
        locus: &crate::GeneLocus,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_gene_locus(&self, id: Uuid) -> StoreResult<crate::GeneLocus>;
    async fn list_gene_loci(&self, lab_id: Uuid) -> StoreResult<Vec<crate::GeneLocus>>;
    async fn list_gene_loci_including_archived(
        &self,
        lab_id: Uuid,
    ) -> StoreResult<Vec<crate::GeneLocus>>;
    async fn gene_locus_reference_counts(
        &self,
        id: Uuid,
    ) -> StoreResult<crate::GeneticsReferenceCounts>;
    async fn archive_gene_locus(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::GeneLocus>;
    async fn restore_gene_locus(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::GeneLocus>;
    async fn create_allele(&self, allele: &crate::Allele, audit: &AuditContext) -> StoreResult<()>;
    async fn get_allele(&self, id: Uuid) -> StoreResult<crate::Allele>;
    async fn list_alleles(&self, locus_id: Uuid) -> StoreResult<Vec<crate::Allele>>;
    async fn list_alleles_including_archived(
        &self,
        locus_id: Uuid,
    ) -> StoreResult<Vec<crate::Allele>>;
    async fn allele_reference_counts(
        &self,
        id: Uuid,
    ) -> StoreResult<crate::GeneticsReferenceCounts>;
    async fn archive_allele(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::Allele>;
    async fn restore_allele(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::Allele>;
    async fn create_genotype(
        &self,
        genotype: &crate::Genotype,
        project_id: Option<Uuid>,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_genotype(&self, id: Uuid) -> StoreResult<crate::Genotype>;
    async fn list_genotypes(&self, animal_id: Uuid) -> StoreResult<Vec<crate::Genotype>>;

    async fn create_genotype_definition(
        &self,
        definition: &crate::GenotypeDefinition,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_genotype_definition(&self, id: Uuid) -> StoreResult<crate::GenotypeDefinition>;
    async fn list_genotype_definitions(
        &self,
        lab_id: Uuid,
    ) -> StoreResult<Vec<crate::GenotypeDefinition>>;
    async fn list_genotype_definitions_including_archived(
        &self,
        lab_id: Uuid,
    ) -> StoreResult<Vec<crate::GenotypeDefinition>>;
    async fn genotype_definition_reference_counts(
        &self,
        id: Uuid,
    ) -> StoreResult<crate::GeneticsReferenceCounts>;
    async fn archive_genotype_definition(
        &self,
        id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::GenotypeDefinition>;
    async fn restore_genotype_definition(
        &self,
        id: Uuid,
        expected_revision: i64,
        restored_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::GenotypeDefinition>;
    async fn create_genotyping_record(
        &self,
        record: &crate::GenotypingRecord,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_genotyping_record(&self, id: Uuid) -> StoreResult<crate::GenotypingRecord>;
    async fn list_genotyping_records(
        &self,
        animal_id: Uuid,
    ) -> StoreResult<Vec<crate::GenotypingRecord>>;
    async fn list_current_genotyping_records(
        &self,
        animal_id: Uuid,
    ) -> StoreResult<Vec<crate::GenotypingRecord>>;
    async fn void_genotyping_record(
        &self,
        id: Uuid,
        expected_revision: i64,
        reason: &str,
        voided_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::GenotypingRecord>;
    async fn correct_genotyping_record(
        &self,
        id: Uuid,
        expected_revision: i64,
        reason: &str,
        voided_at: DateTime<Utc>,
        replacement: &crate::GenotypingRecord,
        audit: &AuditContext,
    ) -> StoreResult<(crate::GenotypingRecord, crate::GenotypingRecord)>;

    async fn create_breeding_line(
        &self,
        line: &crate::BreedingLine,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_breeding_line(&self, id: Uuid) -> StoreResult<crate::BreedingLine>;
    async fn list_breeding_lines(&self, lab_id: Uuid) -> StoreResult<Vec<crate::BreedingLine>>;
    async fn create_colony(&self, colony: &crate::Colony, audit: &AuditContext) -> StoreResult<()>;
    async fn get_colony(&self, id: Uuid) -> StoreResult<crate::Colony>;
    async fn list_colonies(
        &self,
        lab_id: Uuid,
        breeding_line_id: Option<Uuid>,
    ) -> StoreResult<Vec<crate::Colony>>;
    async fn create_breeding_pair(
        &self,
        pair: &crate::BreedingPair,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_breeding_pair(&self, id: Uuid) -> StoreResult<crate::BreedingPair>;
    async fn list_breeding_pairs(
        &self,
        lab_id: Uuid,
        colony_id: Option<Uuid>,
    ) -> StoreResult<Vec<crate::BreedingPair>>;
    async fn retire_breeding_pair(
        &self,
        id: Uuid,
        expected_revision: i64,
        ended_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::BreedingPair>;
    async fn create_mating_event(
        &self,
        event: &crate::MatingEvent,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_mating_event(&self, id: Uuid) -> StoreResult<crate::MatingEvent>;
    async fn list_mating_events(
        &self,
        breeding_pair_id: Uuid,
    ) -> StoreResult<Vec<crate::MatingEvent>>;
    /// Creates a litter and every pending offspring draft in one transaction.
    async fn create_litter(
        &self,
        litter: &crate::Litter,
        drafts: &[crate::AnimalDraft],
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_litter(&self, id: Uuid) -> StoreResult<crate::Litter>;
    async fn list_litters(&self, breeding_pair_id: Uuid) -> StoreResult<Vec<crate::Litter>>;
    async fn get_animal_draft(&self, id: Uuid) -> StoreResult<crate::AnimalDraft>;
    async fn list_animal_drafts(&self, litter_id: Uuid) -> StoreResult<Vec<crate::AnimalDraft>>;
    /// Registers one draft as an Animal, writes both parent pedigree edges,
    /// lifecycle events, audit and provenance atomically.
    async fn register_animal_draft(
        &self,
        draft_id: Uuid,
        expected_revision: i64,
        animal: &crate::Animal,
        audit: &AuditContext,
    ) -> StoreResult<crate::AnimalDraft>;

    async fn create_experiment_event(
        &self,
        event: &crate::ExperimentEvent,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_experiment_event(&self, id: Uuid) -> StoreResult<crate::ExperimentEvent>;
    async fn list_experiment_events(
        &self,
        experiment_id: Uuid,
    ) -> StoreResult<Vec<crate::ExperimentEvent>>;
    async fn create_observation_definition(
        &self,
        definition: &crate::ObservationDefinition,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_observation_definition(
        &self,
        id: Uuid,
    ) -> StoreResult<crate::ObservationDefinition>;
    async fn list_observation_definitions(
        &self,
        experiment_id: Uuid,
    ) -> StoreResult<Vec<crate::ObservationDefinition>>;
    /// Creates an observation and its initial value in one transaction.
    async fn create_observation(
        &self,
        observation: &crate::Observation,
        value: &crate::ObservationValueRecord,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_observation(&self, id: Uuid) -> StoreResult<crate::Observation>;
    async fn list_observations(
        &self,
        filter: &crate::ObservationFilter,
    ) -> StoreResult<Vec<crate::Observation>>;
    async fn get_observation_value(&self, id: Uuid) -> StoreResult<crate::ObservationValueRecord>;
    async fn list_observation_values(
        &self,
        observation_id: Uuid,
    ) -> StoreResult<Vec<crate::ObservationValueRecord>>;
    /// Appends a value version and advances the observation projection atomically.
    async fn revise_observation_value(
        &self,
        observation_id: Uuid,
        expected_revision: i64,
        value: &crate::ObservationValueRecord,
        audit: &AuditContext,
    ) -> StoreResult<crate::Observation>;

    async fn create_pedigree(
        &self,
        pedigree: &crate::Pedigree,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_pedigree(&self, id: Uuid) -> StoreResult<crate::Pedigree>;
    async fn list_pedigrees(&self, animal_id: Uuid) -> StoreResult<Vec<crate::Pedigree>>;
    /// Returns both parent edges and offspring edges for an animal.
    async fn list_related_pedigrees(&self, animal_id: Uuid) -> StoreResult<Vec<Pedigree>>;
    async fn create_template_version(
        &self,
        template: &crate::ExperimentTemplateVersion,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_template_version(&self, id: Uuid)
    -> StoreResult<crate::ExperimentTemplateVersion>;
    /// Publishes one draft template and its audit entry atomically.
    ///
    /// Implementations must reject a stale `expected_revision` rather than
    /// overwriting a concurrent change.
    async fn publish_template_version(
        &self,
        id: Uuid,
        expected_revision: i64,
        published_by: Uuid,
        published_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::ExperimentTemplateVersion>;
    async fn list_template_versions(
        &self,
        lab_id: Uuid,
        template_key: Option<&str>,
    ) -> StoreResult<Vec<crate::ExperimentTemplateVersion>>;
    async fn create_cohort(&self, cohort: &crate::Cohort, audit: &AuditContext) -> StoreResult<()>;
    async fn list_cohorts(&self, experiment_id: Uuid) -> StoreResult<Vec<crate::Cohort>>;
    async fn create_procedure(
        &self,
        procedure: &crate::Procedure,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn list_procedures(
        &self,
        experiment_id: Uuid,
        animal_id: Option<Uuid>,
    ) -> StoreResult<Vec<crate::Procedure>>;
    async fn create_attachment(
        &self,
        attachment: &crate::Attachment,
        audit: &AuditContext,
    ) -> StoreResult<()>;
    async fn get_attachment(&self, id: Uuid) -> StoreResult<crate::Attachment>;
    async fn soft_delete_attachment(
        &self,
        id: Uuid,
        expected_revision: i64,
        deleted_at: DateTime<Utc>,
        audit: &AuditContext,
    ) -> StoreResult<crate::Attachment>;
    async fn list_attachments(
        &self,
        lab_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
    ) -> StoreResult<Vec<crate::Attachment>>;
    /// Lists every active attachment metadata row in a lab. Snapshot/export
    /// code uses this scoped method instead of guessing entity types or
    /// reaching into a database adapter directly.
    async fn list_lab_attachments(&self, lab_id: Uuid) -> StoreResult<Vec<crate::Attachment>>;

    async fn create_job(&self, job: &crate::Job, audit: &AuditContext) -> StoreResult<()>;
    async fn get_job(&self, id: Uuid) -> StoreResult<crate::Job>;
    async fn find_job_by_idempotency(
        &self,
        lab_id: Uuid,
        created_by: Uuid,
        idempotency_key: &str,
    ) -> StoreResult<Option<crate::Job>>;
    async fn list_jobs(&self, filter: &JobFilter) -> StoreResult<Vec<crate::Job>>;
    async fn update_job(
        &self,
        job: &crate::Job,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()>;

    /// Atomically confirms one fully resolved import preview. Implementations
    /// must write every entity and Import audit entry in one transaction, or
    /// write nothing. Repeating the same idempotency key and preview hash must
    /// return the original receipt without repeating entity writes.
    async fn commit_import(
        &self,
        plan: &ImportPlan,
        options: ImportCommitOptions,
        audit: &AuditContext,
    ) -> StoreResult<ImportCommitResult>;

    async fn list_audit_entries(&self, filter: &AuditFilter) -> StoreResult<Vec<AuditEntry>>;
    async fn list_provenance(&self, filter: &ProvenanceFilter) -> StoreResult<Vec<Provenance>>;
}
