use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::{DateTime, Duration, Utc};

use async_trait::async_trait;
use muriarc_application::{
    ActivityQueryRequest, AnimalContextRequest, AuditQueryRequest, BusinessReadAccess,
    BusinessReadError, BusinessReadResult, BusinessReadService, BusinessSourceRef,
    ExperimentGroupingRequest, GenotypingQueryRequest, GroupingCandidate, MAX_GROUPING_CANDIDATES,
    ProjectContextRequest, ProvenanceQueryRequest, ResourceSearchRequest,
    build_experiment_grouping_plan,
};
use muriarc_core::{
    AiAutonomyMode, AiExperimentGroupingApplication, AiGroupingAnimalRevision,
    AiGroupingLatestWeightRevision, AnimalFilter, AnimalStatus, Cohort, EntityType,
    ExperimentFilter, ExperimentStatus, Measurement, MeasurementFilter, MeasurementValue,
    MuriArcStore, Participation, ParticipationFilter, Project, ProjectStatus, SampleFilter,
    StoreError,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AiActionPolicy, AiDataAccessContext, AiDataToolBackend, Citation, DomainToolExecutor,
    DomainToolOutput, DomainToolRequest, DraftKind, FieldChange, ProposalActor, ToolExecutionError,
    ToolName, WriteDraft,
};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
const MAX_OFFSET: usize = 10_000;
const MAX_QUERY_LENGTH: usize = 256;
const MAX_SHORT_TEXT_LENGTH: usize = 128;
const STORE_MODEL_READ_TOOLS: [ToolName; 4] = [
    ToolName::ResourceSearch,
    ToolName::GenotypingQuery,
    ToolName::AnimalContext,
    ToolName::ProjectContext,
];

/// Store-level access already resolved by the authenticated application layer.
///
/// A value is intentionally scoped to one lab. Project-specific tools can only
/// access identifiers present in `allowed_project_ids`, even if a caller puts a
/// cross-lab project into tool arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreToolAccessContext {
    lab_id: Uuid,
    allowed_project_ids: BTreeSet<Uuid>,
    lab_registry_read: bool,
    activity_read: bool,
    audit_read: bool,
    current_user_id: Option<Uuid>,
    writable_project_ids: BTreeSet<Uuid>,
}

impl StoreToolAccessContext {
    pub fn new(lab_id: Uuid, allowed_project_ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            lab_id,
            allowed_project_ids: allowed_project_ids.into_iter().collect(),
            lab_registry_read: false,
            activity_read: false,
            audit_read: false,
            current_user_id: None,
            writable_project_ids: BTreeSet::new(),
        }
    }

    /// Enables lab-wide Animal Registry and cage reads for roles such as Lab
    /// Admin or Animal Manager. Project membership alone must not set this.
    pub const fn with_lab_registry_read(mut self, allowed: bool) -> Self {
        self.lab_registry_read = allowed;
        self
    }

    pub const fn with_activity_read(mut self, allowed: bool) -> Self {
        self.activity_read = allowed;
        self
    }

    pub const fn with_audit_read(mut self, allowed: bool) -> Self {
        self.audit_read = allowed;
        self
    }

    pub const fn with_current_user(mut self, user_id: Uuid) -> Self {
        self.current_user_id = Some(user_id);
        self
    }

    /// Projects in which this actor may create approval-gated measurement
    /// drafts. This must be derived from current human/project permissions,
    /// never from model input.
    pub fn with_writable_projects(mut self, project_ids: impl IntoIterator<Item = Uuid>) -> Self {
        self.writable_project_ids = project_ids
            .into_iter()
            .filter(|id| self.allowed_project_ids.contains(id))
            .collect();
        self
    }

    pub fn allows_project(&self, project_id: Uuid) -> bool {
        self.allowed_project_ids.contains(&project_id)
    }

    pub const fn lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub fn allowed_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.allowed_project_ids
    }

    pub const fn can_read_lab_registry(&self) -> bool {
        self.lab_registry_read
    }

    pub const fn can_read_activity(&self) -> bool {
        self.activity_read
    }

    pub const fn can_read_audit(&self) -> bool {
        self.audit_read
    }

    pub fn can_write_project(&self, project_id: Uuid) -> bool {
        self.writable_project_ids.contains(&project_id)
    }

    pub fn writable_project_ids(&self) -> &BTreeSet<Uuid> {
        &self.writable_project_ids
    }
}

/// Read-only implementation of MuriArc's fixed domain tools.
///
/// This executor only calls `MuriArcStore` read methods. It has no raw-SQL or
/// write API. Models see the aggregate context tools and one focused Genetics
/// v2 query, plus `audit_query` only
/// when explicitly authorized; legacy V1 read names remain executable only for
/// compatibility with trusted existing callers.
#[derive(Clone)]
pub struct StoreDomainToolExecutor {
    store: Arc<dyn MuriArcStore>,
    access: StoreToolAccessContext,
    business_reads: BusinessReadService,
    data_tools: Option<(AiDataAccessContext, Arc<dyn AiDataToolBackend>)>,
    autonomy_mode: AiAutonomyMode,
}

impl StoreDomainToolExecutor {
    pub fn new(store: Arc<dyn MuriArcStore>, access: StoreToolAccessContext) -> Self {
        let read_access =
            BusinessReadAccess::new(access.lab_id, access.allowed_project_ids.iter().copied())
                .with_lab_registry_read(access.lab_registry_read)
                .with_activity_read(access.activity_read)
                .with_audit_read(access.audit_read);
        let read_access = match access.current_user_id {
            Some(user_id) => read_access.with_current_user(user_id),
            None => read_access,
        };
        Self {
            business_reads: BusinessReadService::new(store.clone(), read_access),
            store,
            access,
            data_tools: None,
            autonomy_mode: AiAutonomyMode::Ask,
        }
    }

    pub const fn with_autonomy_mode(mut self, mode: AiAutonomyMode) -> Self {
        self.autonomy_mode = mode;
        self
    }

    pub fn with_data_tools(
        mut self,
        access: AiDataAccessContext,
        backend: Arc<dyn AiDataToolBackend>,
    ) -> Self {
        self.data_tools = Some((access, backend));
        self
    }

    pub fn access(&self) -> &StoreToolAccessContext {
        &self.access
    }

    async fn authorize_project(&self, project_id: Uuid) -> Result<Project, ToolExecutionError> {
        if !self.access.allows_project(project_id) {
            return Err(rejected("project_forbidden"));
        }
        let project = self
            .store
            .get_project(project_id)
            .await
            .map_err(map_store_error)?;
        if project.lab_id != self.access.lab_id {
            return Err(rejected("project_forbidden"));
        }
        Ok(project)
    }

    async fn animal_search(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: AnimalSearchArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        if let Some(query) = arguments.query.as_deref() {
            validate_text_length(query, MAX_QUERY_LENGTH, "query_too_long")?;
        }
        if let Some(project_id) = arguments.project_id {
            self.authorize_project(project_id).await?;
        } else if !self.access.lab_registry_read {
            return Err(rejected("project_required"));
        }
        let mut animals = self
            .store
            .list_animals(&AnimalFilter {
                lab_id: self.access.lab_id,
                project_id: arguments.project_id,
                cage_id: arguments.cage_id,
                status: arguments.status,
                query: arguments.query,
            })
            .await
            .map_err(map_store_error)?;
        animals.retain(|animal| animal.lab_id == self.access.lab_id);
        let (animals, page) = paginate(animals, page);
        let citations = animals
            .iter()
            .map(|animal| Citation::new(EntityType::Animal, animal.id, Some(animal.meta.revision)))
            .collect();
        read_items(animals, page, citations)
    }

    async fn resource_search(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let request: ResourceSearchRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .resource_search(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn genotyping_query(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let request: GenotypingQueryRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .genotyping_query(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn animal_context(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let request: AnimalContextRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .animal_context(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn project_context(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let request: ProjectContextRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .project_context(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn audit_query(&self, arguments: Value) -> Result<DomainToolOutput, ToolExecutionError> {
        if !self.access.audit_read {
            return Err(rejected("audit_forbidden"));
        }
        let request: AuditQueryRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .audit_query(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn activity_query(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        if !self.access.activity_read {
            return Err(rejected("activity_forbidden"));
        }
        let request: ActivityQueryRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .activity_query(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn provenance_query(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        if !self.access.audit_read {
            return Err(rejected("audit_forbidden"));
        }
        let request: ProvenanceQueryRequest = parse_arguments(arguments)?;
        let result = self
            .business_reads
            .provenance_query(request)
            .await
            .map_err(map_business_read_error)?;
        business_read_output(result)
    }

    async fn animal_timeline(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: AnimalTimelineArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        if let Some(project_id) = arguments.project_id {
            self.authorize_project(project_id).await?;
        } else if !self.access.lab_registry_read {
            return Err(rejected("project_required"));
        }
        let animal = self
            .store
            .get_animal(arguments.animal_id)
            .await
            .map_err(map_store_error)?;
        if animal.lab_id != self.access.lab_id {
            return Err(rejected("animal_forbidden"));
        }
        if !self.access.lab_registry_read {
            let Some(project_id) = arguments.project_id else {
                return Err(rejected("project_required"));
            };
            let assignments = self
                .store
                .list_project_animal_assignments(&muriarc_core::ProjectAnimalAssignmentFilter {
                    lab_id: self.access.lab_id,
                    project_id: Some(project_id),
                    animal_id: Some(animal.id),
                })
                .await
                .map_err(map_store_error)?;
            if assignments.is_empty() {
                return Err(rejected("animal_forbidden"));
            }
        }
        let mut events = self
            .store
            .list_animal_events(animal.id)
            .await
            .map_err(map_store_error)?;
        events.retain(|event| {
            if event.lab_id != self.access.lab_id {
                return false;
            }
            match arguments.project_id {
                Some(project_id) => event.project_id == Some(project_id),
                None => event
                    .project_id
                    .is_none_or(|project_id| self.access.allows_project(project_id)),
            }
        });
        let (events, page) = paginate(events, page);
        let mut citations = vec![Citation::new(
            EntityType::Animal,
            animal.id,
            Some(animal.meta.revision),
        )];
        citations.extend(
            events
                .iter()
                .map(|event| Citation::new(EntityType::AnimalEvent, event.id, None)),
        );
        Ok(DomainToolOutput::read(
            json!({"animal": animal, "items": events, "page": page}),
            citations,
        ))
    }

    async fn cage_list(&self, arguments: Value) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ListArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        if !self.access.lab_registry_read {
            return Err(rejected("lab_registry_forbidden"));
        }
        let mut cages = self
            .store
            .list_cages(self.access.lab_id)
            .await
            .map_err(map_store_error)?;
        cages.retain(|cage| cage.lab_id == self.access.lab_id);
        let (cages, page) = paginate(cages, page);
        let citations = cages
            .iter()
            .map(|cage| Citation::new(EntityType::Cage, cage.id, Some(cage.meta.revision)))
            .collect();
        read_items(cages, page, citations)
    }

    async fn project_list(&self, arguments: Value) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ProjectListArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        let mut projects = self
            .store
            .list_projects(self.access.lab_id)
            .await
            .map_err(map_store_error)?;
        projects.retain(|project| {
            project.lab_id == self.access.lab_id
                && self.access.allows_project(project.id)
                && arguments
                    .status
                    .is_none_or(|status| project.status == status)
        });
        let (projects, page) = paginate(projects, page);
        let citations = projects
            .iter()
            .map(|project| {
                Citation::new(EntityType::Project, project.id, Some(project.meta.revision))
            })
            .collect();
        read_items(projects, page, citations)
    }

    async fn experiment_status(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ExperimentStatusArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        self.authorize_project(arguments.project_id).await?;
        let mut experiments = self
            .store
            .list_experiments(&ExperimentFilter {
                project_id: arguments.project_id,
                status: arguments.status,
            })
            .await
            .map_err(map_store_error)?;
        experiments.retain(|experiment| {
            experiment.lab_id == self.access.lab_id && experiment.project_id == arguments.project_id
        });
        let (experiments, page) = paginate(experiments, page);
        let citations = experiments
            .iter()
            .map(|experiment| {
                Citation::new(
                    EntityType::Experiment,
                    experiment.id,
                    Some(experiment.meta.revision),
                )
            })
            .collect();
        read_items(experiments, page, citations)
    }

    async fn measurement_query(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: MeasurementQueryArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        self.authorize_project(arguments.project_id).await?;
        if let Some(key) = arguments.measurement_key.as_deref() {
            validate_non_empty_text(key, MAX_SHORT_TEXT_LENGTH, "measurement_key_invalid")?;
        }
        let mut measurements = self
            .store
            .list_measurements(&MeasurementFilter {
                project_id: arguments.project_id,
                experiment_id: arguments.experiment_id,
                animal_id: arguments.animal_id,
            })
            .await
            .map_err(map_store_error)?;
        measurements.retain(|measurement| {
            measurement.lab_id == self.access.lab_id
                && measurement.project_id == arguments.project_id
                && arguments
                    .measurement_key
                    .as_deref()
                    .is_none_or(|key| measurement.key.eq_ignore_ascii_case(key.trim()))
        });
        let (measurements, page) = paginate(measurements, page);
        let citations = measurements
            .iter()
            .map(|measurement| {
                Citation::new(
                    EntityType::Measurement,
                    measurement.id,
                    Some(measurement.meta.revision),
                )
            })
            .collect();
        read_items(measurements, page, citations)
    }

    async fn measurement_draft(
        &self,
        request: &DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: MeasurementDraftArgs = parse_arguments(request.arguments.clone())?;
        if arguments.operation != MutationOperation::RecordMeasurement {
            return Err(rejected("mutation_operation_forbidden"));
        }
        if !self.access.can_write_project(arguments.project_id) {
            return Err(rejected("project_write_forbidden"));
        }
        let project = self.authorize_project(arguments.project_id).await?;
        let animal = self
            .store
            .get_animal(arguments.animal_id)
            .await
            .map_err(map_store_error)?;
        if animal.lab_id != self.access.lab_id || animal.meta.revision != arguments.animal_revision
        {
            return Err(rejected("animal_revision_conflict"));
        }
        let participations = self
            .store
            .list_participations(&ParticipationFilter {
                project_id: project.id,
                experiment_id: arguments.experiment_id,
                animal_id: Some(animal.id),
                cohort_id: None,
            })
            .await
            .map_err(map_store_error)?;
        if participations.is_empty() {
            return Err(rejected("animal_not_in_project"));
        }

        let mut citations = vec![
            Citation::new(EntityType::Project, project.id, Some(project.meta.revision)),
            Citation::new(EntityType::Animal, animal.id, Some(animal.meta.revision)),
        ];
        if let Some(experiment_id) = arguments.experiment_id {
            let experiment = self
                .store
                .get_experiment(experiment_id)
                .await
                .map_err(map_store_error)?;
            if experiment.lab_id != self.access.lab_id || experiment.project_id != project.id {
                return Err(rejected("experiment_forbidden"));
            }
            citations.push(Citation::new(
                EntityType::Experiment,
                experiment.id,
                Some(experiment.meta.revision),
            ));
            if let Some(procedure_id) = arguments.procedure_id {
                let procedures = self
                    .store
                    .list_procedures(experiment.id, Some(animal.id))
                    .await
                    .map_err(map_store_error)?;
                if !procedures.iter().any(|procedure| {
                    procedure.id == procedure_id
                        && procedure
                            .animal_id
                            .is_none_or(|procedure_animal_id| procedure_animal_id == animal.id)
                }) {
                    return Err(rejected("procedure_forbidden"));
                }
                citations.push(Citation::new(EntityType::Procedure, procedure_id, None));
            }
        } else if arguments.procedure_id.is_some() {
            return Err(rejected("procedure_requires_experiment"));
        }

        validate_non_empty_text(
            &arguments.key,
            MAX_SHORT_TEXT_LENGTH,
            "measurement_key_invalid",
        )?;
        validate_non_empty_text(
            &arguments.label,
            MAX_SHORT_TEXT_LENGTH,
            "measurement_label_invalid",
        )?;
        let unit = arguments
            .unit
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if matches!(arguments.value, MeasurementValue::Number(_)) && unit.is_none() {
            return Err(rejected("numeric_measurement_unit_required"));
        }

        let now = Utc::now();
        let mut measurement = Measurement::draft(
            self.access.lab_id,
            project.id,
            animal.id,
            arguments.key.trim(),
            arguments.label.trim(),
            arguments.value,
            arguments.measured_at,
            now,
        )
        .map_err(|_| rejected("measurement_invalid"))?;
        measurement.experiment_id = arguments.experiment_id;
        measurement.procedure_id = arguments.procedure_id;
        measurement.unit = unit;
        measurement
            .validate_record()
            .map_err(|_| rejected("measurement_invalid"))?;
        let measurement_value =
            serde_json::to_value(&measurement).map_err(|_| rejected("measurement_invalid"))?;
        let draft = WriteDraft::new(
            DraftKind::MeasurementResult,
            ToolName::MutationDraft,
            ProposalActor::Ai {
                user_id: request.user_id,
                tool_run_id: request.tool_run_id,
            },
            Some(project.id),
            vec![FieldChange {
                path: format!("/measurements/{}", measurement.id),
                before: None,
                after: Some(measurement_value.clone()),
            }],
            json!({
                "operation": "create_measurement",
                "measurement": measurement_value,
                "animal_revision": arguments.animal_revision,
            }),
            now,
            now + Duration::hours(24),
        )
        .map_err(|_| rejected("measurement_draft_invalid"))?;
        Ok(DomainToolOutput::write_draft(draft, citations))
    }

    async fn experiment_grouping_draft(
        &self,
        request: &DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: ExperimentGroupingDraftArgs = parse_arguments(request.arguments.clone())?;
        if !self.access.can_write_project(arguments.project_id) {
            return Err(rejected("project_write_forbidden"));
        }
        arguments.validate()?;
        let project = self.authorize_project(arguments.project_id).await?;
        if project.status != ProjectStatus::Active {
            return Err(rejected("grouping_project_inactive"));
        }
        let experiment = self
            .store
            .get_experiment(arguments.experiment_id)
            .await
            .map_err(map_store_error)?;
        if experiment.lab_id != self.access.lab_id
            || experiment.project_id != project.id
            || !matches!(
                experiment.status,
                ExperimentStatus::Draft | ExperimentStatus::Active
            )
        {
            return Err(rejected("experiment_forbidden"));
        }

        // One bounded aggregate query supplies all authorized project animals,
        // their latest weights and optimistic revisions. The model never
        // supplies an animal UUID list.
        let overviews = self
            .store
            .list_animal_overviews(
                &AnimalFilter {
                    lab_id: self.access.lab_id,
                    project_id: Some(project.id),
                    cage_id: None,
                    status: None,
                    query: None,
                },
                0,
                (MAX_GROUPING_CANDIDATES + 1) as u32,
            )
            .await
            .map_err(map_store_error)?;
        if overviews.is_empty() {
            return Err(rejected("grouping_candidates_empty"));
        }
        if overviews.len() > MAX_GROUPING_CANDIDATES {
            return Err(rejected("grouping_candidates_limit_exceeded"));
        }
        let existing = self
            .store
            .list_participations(&ParticipationFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: None,
                cohort_id: None,
            })
            .await
            .map_err(map_store_error)?
            .into_iter()
            .map(|participation| participation.animal_id)
            .collect::<BTreeSet<_>>();
        let now = Utc::now();
        let stratify_by = arguments
            .stratify_by
            .iter()
            .map(|factor| factor.as_str().to_owned())
            .collect::<Vec<_>>();
        let balance_by = arguments
            .balance_by
            .iter()
            .map(|factor| factor.as_str().to_owned())
            .collect::<Vec<_>>();
        let tracks_latest_weight = arguments
            .balance_by
            .contains(&GroupingBalanceFactor::WeightGrams);
        let candidates = overviews
            .iter()
            .map(|overview| {
                let animal = &overview.animal;
                let mut strata = BTreeMap::new();
                for factor in &arguments.stratify_by {
                    strata.insert(
                        factor.as_str().to_owned(),
                        factor
                            .value(animal)
                            .unwrap_or_else(|| "<missing>".to_owned()),
                    );
                }
                let mut covariates = BTreeMap::new();
                for factor in &arguments.balance_by {
                    if let Some(value) = factor.value(overview, now) {
                        covariates.insert(factor.as_str().to_owned(), value);
                    }
                }
                let exclusion_reason =
                    grouping_exclusion_reason(animal, &existing, &arguments, &strata, &covariates);
                GroupingCandidate {
                    animal_id: animal.id,
                    expected_revision: animal.meta.revision,
                    strata,
                    covariates,
                    exclusion_reason,
                }
            })
            .collect();
        let plan = build_experiment_grouping_plan(ExperimentGroupingRequest {
            project_id: project.id,
            expected_project_revision: project.meta.revision,
            experiment_id: experiment.id,
            expected_experiment_revision: experiment.meta.revision,
            seed: arguments.seed,
            cohort_names: arguments.cohort_names,
            stratify_by,
            balance_by,
            candidates,
        })
        .map_err(|_| rejected("grouping_plan_invalid"))?;
        if plan.assignments.is_empty() {
            return Err(rejected("grouping_candidates_empty"));
        }

        let cohorts = plan
            .cohort_names
            .iter()
            .map(|name| {
                let mut cohort = Cohort::new(experiment.id, name, now)
                    .map_err(|_| rejected("grouping_cohort_invalid"))?;
                cohort.description = Some(format!("AI deterministic grouping, seed {}", plan.seed));
                Ok(cohort)
            })
            .collect::<Result<Vec<_>, ToolExecutionError>>()?;
        let cohort_ids = cohorts.iter().map(|cohort| cohort.id).collect::<Vec<_>>();
        let participations = plan
            .assignments
            .iter()
            .map(|assignment| {
                let mut participation =
                    Participation::enroll(experiment.id, assignment.animal_id, now);
                participation.cohort_id = Some(cohort_ids[assignment.cohort_index]);
                participation
            })
            .collect::<Vec<_>>();
        let application = AiExperimentGroupingApplication {
            lab_id: self.access.lab_id,
            project_id: project.id,
            expected_project_revision: project.meta.revision,
            experiment_id: experiment.id,
            expected_experiment_revision: experiment.meta.revision,
            input_snapshot_sha256: plan.input_snapshot_sha256.clone(),
            cohorts: cohorts.clone(),
            participations: participations.clone(),
            expected_animal_revisions: plan
                .assignments
                .iter()
                .map(|assignment| AiGroupingAnimalRevision {
                    animal_id: assignment.animal_id,
                    expected_revision: assignment.expected_revision,
                })
                .chain(
                    plan.exclusions
                        .iter()
                        .map(|exclusion| AiGroupingAnimalRevision {
                            animal_id: exclusion.animal_id,
                            expected_revision: exclusion.expected_revision,
                        }),
                )
                .collect(),
            expected_latest_weights: if tracks_latest_weight {
                overviews
                    .iter()
                    .map(|overview| AiGroupingLatestWeightRevision {
                        animal_id: overview.animal.id,
                        measurement_id: overview
                            .latest_weight
                            .as_ref()
                            .map(|weight| weight.measurement_id),
                        expected_revision: overview
                            .latest_weight
                            .as_ref()
                            .map(|weight| weight.revision),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        };
        let mut changes = cohorts
            .iter()
            .map(|cohort| FieldChange {
                path: format!("/cohorts/{}", cohort.id),
                before: None,
                after: serde_json::to_value(cohort).ok(),
            })
            .collect::<Vec<_>>();
        changes.extend(participations.iter().map(|participation| FieldChange {
            path: format!("/participations/{}", participation.id),
            before: None,
            after: serde_json::to_value(participation).ok(),
        }));
        let draft = WriteDraft::new(
            DraftKind::ResearchPlan,
            ToolName::ExperimentGroupingDraft,
            ProposalActor::Ai {
                user_id: request.user_id,
                tool_run_id: request.tool_run_id,
            },
            Some(project.id),
            changes,
            json!({
                "operation": "apply_experiment_grouping",
                "plan": plan,
                "application": application,
            }),
            now,
            now + Duration::hours(24),
        )
        .map_err(|_| rejected("grouping_draft_invalid"))?;
        let mut citations = vec![
            Citation::new(EntityType::Project, project.id, Some(project.meta.revision)),
            Citation::new(
                EntityType::Experiment,
                experiment.id,
                Some(experiment.meta.revision),
            ),
        ];
        citations.extend(overviews.iter().map(|overview| {
            Citation::new(
                EntityType::Animal,
                overview.animal.id,
                Some(overview.animal.meta.revision),
            )
        }));
        Ok(DomainToolOutput::write_draft(draft, citations))
    }

    async fn sample_inventory(
        &self,
        arguments: Value,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        let arguments: SampleInventoryArgs = parse_arguments(arguments)?;
        let page = arguments.page()?;
        self.authorize_project(arguments.project_id).await?;
        if let Some(sample_type) = arguments.sample_type.as_deref() {
            validate_non_empty_text(sample_type, MAX_SHORT_TEXT_LENGTH, "sample_type_invalid")?;
        }
        let mut samples = self
            .store
            .list_samples(&SampleFilter {
                project_id: arguments.project_id,
                experiment_id: arguments.experiment_id,
                animal_id: arguments.animal_id,
            })
            .await
            .map_err(map_store_error)?;
        samples.retain(|sample| {
            sample.lab_id == self.access.lab_id
                && sample.project_id == arguments.project_id
                && arguments.sample_type.as_deref().is_none_or(|sample_type| {
                    sample.sample_type.eq_ignore_ascii_case(sample_type.trim())
                })
        });
        let (samples, page) = paginate(samples, page);
        let citations = samples
            .iter()
            .map(|sample| Citation::new(EntityType::Sample, sample.id, Some(sample.meta.revision)))
            .collect();
        read_items(samples, page, citations)
    }
}

#[async_trait]
impl DomainToolExecutor for StoreDomainToolExecutor {
    fn supported_tools(&self) -> Vec<ToolName> {
        let policy = AiActionPolicy::new(self.autonomy_mode);
        let mut tools = STORE_MODEL_READ_TOOLS
            .into_iter()
            .filter(|tool| policy.allows_tool(*tool))
            .collect::<Vec<_>>();
        if !self.access.writable_project_ids.is_empty() {
            tools.push(ToolName::MutationDraft);
            tools.push(ToolName::ExperimentGroupingDraft);
        }
        if self.access.activity_read {
            tools.push(ToolName::ActivityQuery);
        }
        if self.access.audit_read {
            tools.push(ToolName::AuditQuery);
            tools.push(ToolName::ProvenanceQuery);
        }
        if let Some((access, backend)) = &self.data_tools {
            for tool in backend.supported_tools(access) {
                if matches!(
                    tool,
                    ToolName::SourceImportPreview
                        | ToolName::ImportPreview
                        | ToolName::ImportCommitDraft
                        | ToolName::ExportCreate
                ) && policy.allows_tool(tool)
                    && !tools.contains(&tool)
                {
                    tools.push(tool);
                }
            }
        }
        tools
    }

    async fn execute(
        &self,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        if !AiActionPolicy::new(self.autonomy_mode).allows_tool(request.tool) {
            return Err(rejected("autonomy_confirmation_required"));
        }
        match request.tool {
            ToolName::ResourceSearch => self.resource_search(request.arguments).await,
            ToolName::GenotypingQuery => self.genotyping_query(request.arguments).await,
            ToolName::AnimalContext => self.animal_context(request.arguments).await,
            ToolName::ProjectContext => self.project_context(request.arguments).await,
            ToolName::ActivityQuery if self.access.activity_read => {
                self.activity_query(request.arguments).await
            }
            ToolName::ActivityQuery => Err(rejected("activity_forbidden")),
            ToolName::AuditQuery if self.access.audit_read => {
                self.audit_query(request.arguments).await
            }
            ToolName::AuditQuery => Err(rejected("audit_forbidden")),
            ToolName::ProvenanceQuery if self.access.audit_read => {
                self.provenance_query(request.arguments).await
            }
            ToolName::ProvenanceQuery => Err(rejected("audit_forbidden")),
            ToolName::AnimalSearch => self.animal_search(request.arguments).await,
            ToolName::AnimalTimeline => self.animal_timeline(request.arguments).await,
            ToolName::CageList => self.cage_list(request.arguments).await,
            ToolName::ProjectList => self.project_list(request.arguments).await,
            ToolName::ExperimentStatus => self.experiment_status(request.arguments).await,
            ToolName::MeasurementQuery => self.measurement_query(request.arguments).await,
            ToolName::SampleInventory => self.sample_inventory(request.arguments).await,
            ToolName::MutationDraft if !self.access.writable_project_ids.is_empty() => {
                self.measurement_draft(&request).await
            }
            ToolName::MutationDraft => Err(rejected("unsupported_tool")),
            ToolName::ExperimentGroupingDraft if !self.access.writable_project_ids.is_empty() => {
                self.experiment_grouping_draft(&request).await
            }
            ToolName::ExperimentGroupingDraft => Err(rejected("unsupported_tool")),
            ToolName::SourceImportPreview
            | ToolName::ImportPreview
            | ToolName::ImportCommitDraft
            | ToolName::ExportCreate => {
                let Some((access, backend)) = &self.data_tools else {
                    return Err(rejected("unsupported_tool"));
                };
                if !backend.supported_tools(access).contains(&request.tool) {
                    return Err(rejected("unsupported_tool"));
                }
                backend.execute(access, request).await
            }
            ToolName::ExperimentTemplateDraft => Err(rejected("unsupported_tool")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MutationOperation {
    RecordMeasurement,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementDraftArgs {
    operation: MutationOperation,
    project_id: Uuid,
    animal_id: Uuid,
    animal_revision: i64,
    #[serde(default)]
    experiment_id: Option<Uuid>,
    #[serde(default)]
    procedure_id: Option<Uuid>,
    key: String,
    label: String,
    value: MeasurementValue,
    #[serde(default)]
    unit: Option<String>,
    measured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GroupingStratifyFactor {
    Sex,
    Strain,
    CurrentStatus,
}

impl GroupingStratifyFactor {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Sex => "sex",
            Self::Strain => "strain",
            Self::CurrentStatus => "current_status",
        }
    }

    fn value(self, animal: &muriarc_core::Animal) -> Option<String> {
        match self {
            Self::Sex => serde_json::to_value(animal.sex)
                .ok()?
                .as_str()
                .map(str::to_owned),
            Self::Strain => animal
                .strain
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            Self::CurrentStatus => serde_json::to_value(animal.current_status)
                .ok()?
                .as_str()
                .map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GroupingBalanceFactor {
    AgeDays,
    WeightGrams,
}

impl GroupingBalanceFactor {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AgeDays => "age_days",
            Self::WeightGrams => "weight_grams",
        }
    }

    fn value(self, overview: &muriarc_core::AnimalOverview, now: DateTime<Utc>) -> Option<f64> {
        match self {
            Self::AgeDays => overview
                .animal
                .birth_date
                .map(|birth_date| (now.date_naive() - birth_date).num_days())
                .filter(|days| *days >= 0)
                .map(|days| days as f64),
            Self::WeightGrams => {
                let weight = overview.latest_weight.as_ref()?;
                let unit = weight.unit.as_deref()?.trim().to_ascii_lowercase();
                match unit.as_str() {
                    "g" | "gram" | "grams" => Some(weight.value),
                    "mg" => Some(weight.value / 1_000.0),
                    "kg" => Some(weight.value * 1_000.0),
                    _ => None,
                }
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupingExclusionArgs {
    #[serde(default)]
    statuses: Vec<AnimalStatus>,
    #[serde(default)]
    missing_factors: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentGroupingDraftArgs {
    project_id: Uuid,
    experiment_id: Uuid,
    seed: u64,
    cohort_names: Vec<String>,
    #[serde(default)]
    stratify_by: Vec<GroupingStratifyFactor>,
    #[serde(default)]
    balance_by: Vec<GroupingBalanceFactor>,
    #[serde(default)]
    exclusion: GroupingExclusionArgs,
}

impl ExperimentGroupingDraftArgs {
    fn validate(&self) -> Result<(), ToolExecutionError> {
        if self.project_id.is_nil()
            || self.experiment_id.is_nil()
            || !(2..=20).contains(&self.cohort_names.len())
            || self.stratify_by.len() > 3
            || self.balance_by.len() > 2
            || self.exclusion.statuses.len() > 8
            || self.stratify_by.iter().collect::<BTreeSet<_>>().len() != self.stratify_by.len()
            || self.balance_by.iter().collect::<BTreeSet<_>>().len() != self.balance_by.len()
            || self
                .exclusion
                .statuses
                .iter()
                .enumerate()
                .any(|(index, status)| self.exclusion.statuses[..index].contains(status))
        {
            return Err(rejected("grouping_arguments_invalid"));
        }
        Ok(())
    }
}

fn grouping_exclusion_reason(
    animal: &muriarc_core::Animal,
    existing: &BTreeSet<Uuid>,
    arguments: &ExperimentGroupingDraftArgs,
    strata: &BTreeMap<String, String>,
    covariates: &BTreeMap<String, f64>,
) -> Option<String> {
    if existing.contains(&animal.id) {
        return Some("already_participates_in_experiment".to_owned());
    }
    if !matches!(
        animal.current_status,
        AnimalStatus::Alive | AnimalStatus::InExperiment | AnimalStatus::Sampled
    ) {
        return Some("animal_status_not_enrollable".to_owned());
    }
    if arguments
        .exclusion
        .statuses
        .contains(&animal.current_status)
    {
        return Some("explicit_status_exclusion".to_owned());
    }
    if arguments.exclusion.missing_factors
        && (strata.values().any(|value| value == "<missing>")
            || covariates.len() != arguments.balance_by.len())
    {
        return Some("missing_requested_factor".to_owned());
    }
    None
}

#[derive(Debug, Clone, Copy, Serialize)]
struct PageResult {
    offset: usize,
    limit: usize,
    returned: usize,
    total: usize,
}

#[derive(Serialize)]
struct PagedItems<T> {
    items: Vec<T>,
    page: PageResult,
}

#[derive(Debug, Clone, Copy)]
struct PageRequest {
    offset: usize,
    limit: usize,
}

fn checked_page(
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<PageRequest, ToolExecutionError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let offset = offset.unwrap_or(0);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(rejected("limit_out_of_range"));
    }
    if offset > MAX_OFFSET {
        return Err(rejected("offset_out_of_range"));
    }
    Ok(PageRequest { offset, limit })
}

fn paginate<T>(items: Vec<T>, page: PageRequest) -> (Vec<T>, PageResult) {
    let total = items.len();
    let items = items
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect::<Vec<_>>();
    let result = PageResult {
        offset: page.offset,
        limit: page.limit,
        returned: items.len(),
        total,
    };
    (items, result)
}

fn read_items<T: Serialize>(
    items: Vec<T>,
    page: PageResult,
    citations: Vec<Citation>,
) -> Result<DomainToolOutput, ToolExecutionError> {
    let data = serde_json::to_value(PagedItems { items, page })
        .map_err(|_| ToolExecutionError::Unavailable)?;
    Ok(DomainToolOutput::read(data, citations))
}

fn parse_arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolExecutionError> {
    serde_json::from_value(arguments).map_err(|_| rejected("invalid_arguments"))
}

fn validate_text_length(
    value: &str,
    maximum: usize,
    code: &'static str,
) -> Result<(), ToolExecutionError> {
    if value.chars().count() > maximum {
        Err(rejected(code))
    } else {
        Ok(())
    }
}

fn validate_non_empty_text(
    value: &str,
    maximum: usize,
    code: &'static str,
) -> Result<(), ToolExecutionError> {
    if value.trim().is_empty() || value.chars().count() > maximum {
        Err(rejected(code))
    } else {
        Ok(())
    }
}

fn map_store_error(error: StoreError) -> ToolExecutionError {
    match error {
        StoreError::NotFound { .. } => rejected("not_found"),
        StoreError::Database(_)
        | StoreError::Serialization(_)
        | StoreError::Conflict(_)
        | StoreError::Validation(_) => ToolExecutionError::Unavailable,
    }
}

fn map_business_read_error(error: BusinessReadError) -> ToolExecutionError {
    match error {
        BusinessReadError::Rejected(code) => rejected(code),
        BusinessReadError::Store(error) => map_store_error(error),
    }
}

fn business_read_output<T: Serialize>(
    result: BusinessReadResult<T>,
) -> Result<DomainToolOutput, ToolExecutionError> {
    let data = serde_json::to_value(result.data).map_err(|_| ToolExecutionError::Unavailable)?;
    let citations = result
        .sources
        .into_iter()
        .map(citation_from_source)
        .collect();
    Ok(DomainToolOutput::read(data, citations))
}

fn citation_from_source(source: BusinessSourceRef) -> Citation {
    Citation::new(source.entity_type, source.entity_id, source.revision)
}

fn rejected(code: &'static str) -> ToolExecutionError {
    ToolExecutionError::Rejected {
        code: code.to_owned(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl ListArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimalSearchArgs {
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default)]
    cage_id: Option<Uuid>,
    #[serde(default)]
    status: Option<AnimalStatus>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl AnimalSearchArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimalTimelineArgs {
    animal_id: Uuid,
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl AnimalTimelineArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectListArgs {
    #[serde(default)]
    status: Option<ProjectStatus>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl ProjectListArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentStatusArgs {
    project_id: Uuid,
    #[serde(default)]
    status: Option<ExperimentStatus>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl ExperimentStatusArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementQueryArgs {
    project_id: Uuid,
    #[serde(default)]
    experiment_id: Option<Uuid>,
    #[serde(default)]
    animal_id: Option<Uuid>,
    #[serde(default)]
    measurement_key: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl MeasurementQueryArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleInventoryArgs {
    project_id: Uuid,
    #[serde(default)]
    experiment_id: Option<Uuid>,
    #[serde(default)]
    animal_id: Option<Uuid>,
    #[serde(default)]
    sample_type: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

impl SampleInventoryArgs {
    fn page(&self) -> Result<PageRequest, ToolExecutionError> {
        checked_page(self.limit, self.offset)
    }
}
