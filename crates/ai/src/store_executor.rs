use std::{collections::BTreeSet, sync::Arc};

use chrono::{DateTime, Duration, Utc};

use async_trait::async_trait;
use muriarc_core::{
    AiAutonomyMode, AnimalFilter, AnimalStatus, EntityType, ExperimentFilter, ExperimentStatus,
    Measurement, MeasurementFilter, MeasurementValue, MuriArcStore, ParticipationFilter, Project,
    ProjectStatus, SampleFilter, StoreError,
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
const STORE_READ_TOOLS: [ToolName; 7] = [
    ToolName::AnimalSearch,
    ToolName::AnimalTimeline,
    ToolName::CageList,
    ToolName::ProjectList,
    ToolName::ExperimentStatus,
    ToolName::MeasurementQuery,
    ToolName::SampleInventory,
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
    writable_project_ids: BTreeSet<Uuid>,
}

impl StoreToolAccessContext {
    pub fn new(lab_id: Uuid, allowed_project_ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            lab_id,
            allowed_project_ids: allowed_project_ids.into_iter().collect(),
            lab_registry_read: false,
            writable_project_ids: BTreeSet::new(),
        }
    }

    /// Enables lab-wide Animal Registry and cage reads for roles such as Lab
    /// Admin or Animal Manager. Project membership alone must not set this.
    pub const fn with_lab_registry_read(mut self, allowed: bool) -> Self {
        self.lab_registry_read = allowed;
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
/// write API and advertises only the seven V1 read tools it implements.
#[derive(Clone)]
pub struct StoreDomainToolExecutor {
    store: Arc<dyn MuriArcStore>,
    access: StoreToolAccessContext,
    data_tools: Option<(AiDataAccessContext, Arc<dyn AiDataToolBackend>)>,
    autonomy_mode: AiAutonomyMode,
}

impl StoreDomainToolExecutor {
    pub fn new(store: Arc<dyn MuriArcStore>, access: StoreToolAccessContext) -> Self {
        Self {
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
            let participations = self
                .store
                .list_participations(&ParticipationFilter {
                    project_id,
                    experiment_id: None,
                    animal_id: Some(animal.id),
                    cohort_id: None,
                })
                .await
                .map_err(map_store_error)?;
            if participations.is_empty() {
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
        let mut tools = STORE_READ_TOOLS
            .into_iter()
            .filter(|tool| {
                policy.allows_tool(*tool)
                    && (self.access.lab_registry_read || *tool != ToolName::CageList)
            })
            .collect::<Vec<_>>();
        if !self.access.writable_project_ids.is_empty() {
            tools.push(ToolName::MutationDraft);
        }
        if let Some((access, backend)) = &self.data_tools {
            for tool in backend.supported_tools(access) {
                if matches!(
                    tool,
                    ToolName::ImportPreview | ToolName::ImportCommitDraft | ToolName::ExportCreate
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
            ToolName::ImportPreview | ToolName::ImportCommitDraft | ToolName::ExportCreate => {
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
