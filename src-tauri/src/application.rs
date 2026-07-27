use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::{
    ApplicationError, CreateAnimalCommand, CreateAnimalIdentifierScope, CreateCageCommand,
    CreateCohortCommand, CreateExperimentCommand, CreateParticipationCommand,
    CreateProcedureCommand, CreateProjectCommand, CreateTemplateVersionCommand,
    InitialGenotypingRecordInput, PublishTemplateVersionCommand, TransferAnimalsCommand,
    TransitionExperimentCommand, TransitionParticipationCommand,
    create_animal as create_animal_use_case, create_cage as create_cage_use_case,
    create_cohort as create_cohort_use_case, create_experiment as create_experiment_use_case,
    create_participation as create_participation_use_case,
    create_procedure as create_procedure_use_case, create_project as create_project_use_case,
    create_template_version as create_template_version_use_case,
    publish_template_version as publish_template_version_use_case,
    transfer_animals as transfer_animals_use_case,
    transition_experiment as transition_experiment_use_case,
    transition_participation as transition_participation_use_case,
};
use muriarc_core::{
    Actor, Animal, AnimalEvent, AnimalEventKind, AnimalFilter, AnimalOverview, AnimalStatus,
    AuditContext, Cage, CageKind, Cohort, DomainError, Experiment, ExperimentFilter,
    ExperimentStatus, ExperimentTemplateVersion, FieldValueType, GenotypingState, JobFilter,
    JobKind, JobStatus, LOCAL_LAB_ID, LOCAL_OPERATOR_NAME, LOCAL_USER_ID, MuriArcStore,
    Participation, ParticipationFilter, ParticipationStatus, Procedure, ProcedureStatus, Sex,
    StoreError, TemplateField, TemplateStatus, User, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::settings::{AiSettingsView, SaveAiSettingsInput, SettingsError, SettingsService};

#[derive(Debug, Error)]
pub(crate) enum DesktopError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("invalid {field} identifier")]
    InvalidId { field: &'static str },
    #[error("{field} must be an ISO-8601 date or timestamp")]
    InvalidDate { field: &'static str },
    #[error("{field} must not exceed {max} characters")]
    TooLong { field: &'static str, max: usize },
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

impl DesktopError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Store(StoreError::NotFound { .. })
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::NotFound { .. })) => {
                "not_found"
            }
            Self::Store(StoreError::Conflict(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Conflict(_))) => {
                "conflict"
            }
            Self::Store(StoreError::Validation(_))
            | Self::Settings(SettingsError::ModelProfileStore(StoreError::Validation(_)))
            | Self::Domain(_)
            | Self::InvalidId { .. }
            | Self::InvalidDate { .. }
            | Self::TooLong { .. } => "validation",
            Self::Settings(error) if error.is_validation() => "validation",
            Self::Store(StoreError::Database(_) | StoreError::Serialization(_))
            | Self::Settings(SettingsError::ModelProfileStore(
                StoreError::Database(_) | StoreError::Serialization(_),
            ))
            | Self::Settings(_) => "storage_error",
        }
    }
}

impl From<ApplicationError> for DesktopError {
    fn from(error: ApplicationError) -> Self {
        match error {
            ApplicationError::Store(error) => Self::Store(error),
            ApplicationError::Domain(error) => Self::Domain(error),
            ApplicationError::TooLong { field, max } => Self::TooLong { field, max },
            ApplicationError::TooManyBytes { field, max } => Self::TooLong { field, max },
            ApplicationError::Validation(message) => Self::Store(StoreError::Validation(message)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopState {
    store: SqliteStore,
    lab_id: Uuid,
    user_id: Uuid,
    settings: SettingsService,
}

impl DesktopState {
    #[cfg(test)]
    pub(crate) async fn initialize(database_path: impl AsRef<Path>) -> Result<Self, DesktopError> {
        let app_data_dir = database_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let settings = SettingsService::for_app_data(app_data_dir);
        let store = SqliteStore::connect_path(&database_path).await?;
        store.migrate().await?;
        if store.compatibility_report().await?.observed.is_none() {
            store.adopt_current_release(Uuid::new_v4()).await?;
        }
        Self::initialize_with_settings(database_path, settings).await
    }

    pub(crate) async fn initialize_with_settings(
        database_path: impl AsRef<Path>,
        settings: SettingsService,
    ) -> Result<Self, DesktopError> {
        let store = SqliteStore::connect_path(database_path).await?;
        store
            .compatibility_report()
            .await?
            .require_compatible()
            .map_err(StoreError::Conflict)?;
        bootstrap_local_identity(&store).await?;
        let preview_bootstrap = crate::runtime_compatibility::preview_bootstrap_enabled()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        if cfg!(test) || preview_bootstrap {
            let mut migration_audit = AuditContext::system(WriteSource::Migration);
            migration_audit.reason = Some("materialize_desktop_ai_model_profiles".to_owned());
            settings
                .initialize_model_profiles(&store, &migration_audit)
                .await?;
        }
        Ok(Self {
            store,
            lab_id: LOCAL_LAB_ID,
            user_id: LOCAL_USER_ID,
            settings,
        })
    }

    pub(crate) fn domain_store(&self) -> &SqliteStore {
        &self.store
    }

    pub(crate) fn model_profile_settings(&self) -> &SettingsService {
        &self.settings
    }

    pub(crate) const fn local_lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub(crate) const fn local_user_id(&self) -> Uuid {
        self.user_id
    }

    pub(crate) async fn audit(&self, reason: &'static str) -> Result<AuditContext, DesktopError> {
        let operator = self.store.get_user(self.user_id).await?;
        Ok(AuditContext {
            actor: Actor::human(self.user_id, operator.display_name),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some(reason.to_owned()),
        })
    }

    pub(crate) async fn get_workspace_settings(
        &self,
    ) -> Result<WorkspaceSettingsView, DesktopError> {
        let lab = self.store.get_lab(self.lab_id).await?;
        let user = self.store.get_user(self.user_id).await?;
        Ok(WorkspaceSettingsView {
            lab_name: lab.name,
            operator_name: user.display_name,
        })
    }

    pub(crate) async fn save_workspace_settings(
        &self,
        input: SaveWorkspaceSettingsInput,
    ) -> Result<WorkspaceSettingsView, DesktopError> {
        let lab_name = normalized_required("lab.name", input.lab_name, 128)?;
        let operator_name = normalized_required("user.display_name", input.operator_name, 128)?;
        let audit = self.audit("update_workspace_settings").await?;
        let now = Utc::now();

        let mut lab = self.store.get_lab(self.lab_id).await?;
        if lab.name != lab_name {
            let expected_revision = lab.meta.revision;
            lab.rename(lab_name, now)?;
            self.store
                .update_lab(&lab, expected_revision, &audit)
                .await?;
        }

        let mut user = self.store.get_user(self.user_id).await?;
        if user.display_name != operator_name {
            let expected_revision = user.meta.revision;
            user.rename(operator_name, now)?;
            self.store
                .update_user(&user, expected_revision, &audit)
                .await?;
        }

        self.get_workspace_settings().await
    }

    pub(crate) fn get_ai_settings(&self) -> Result<AiSettingsView, DesktopError> {
        self.settings.get().map_err(Into::into)
    }

    pub(crate) async fn save_ai_settings(
        &self,
        input: SaveAiSettingsInput,
    ) -> Result<AiSettingsView, DesktopError> {
        let _model_profile_operation = self.settings.profile_coordinator().lock().await;
        let audit = self.audit("update_ai_model_profile").await?;
        self.settings
            .save_and_materialize(&self.store, input, &audit)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn clear_ai_api_key(&self) -> Result<AiSettingsView, DesktopError> {
        let _model_profile_operation = self.settings.profile_coordinator().lock().await;
        let audit = self.audit("revoke_ai_model_profile_credentials").await?;
        self.settings
            .clear_key_with_metadata(&self.store, &audit)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn list_cages(&self) -> Result<Vec<CageView>, DesktopError> {
        let cages = self.store.list_cages(self.lab_id).await?;
        let animals = self
            .store
            .list_animals(&AnimalFilter {
                lab_id: self.lab_id,
                ..AnimalFilter::default()
            })
            .await?;
        Ok(cage_views(cages, &animals))
    }

    pub(crate) async fn create_cage(
        &self,
        input: CreateCageInput,
    ) -> Result<CageView, DesktopError> {
        let audit = self.audit("create_cage").await?;
        let cage = create_cage_use_case(
            &self.store,
            CreateCageCommand {
                lab_id: self.lab_id,
                section: input.room,
                display_id: input.code,
                location: Some(input.rack),
                kind: CageKind::Standard,
                capacity: input.capacity,
                sort_order: 0,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(cage_view(cage, &[]))
    }

    pub(crate) async fn create_animal(
        &self,
        input: CreateAnimalInput,
    ) -> Result<AnimalView, DesktopError> {
        let birth_date = parse_date("animal.birth_date", input.birth_date)?;
        let identifier_scope = match input.identifier_scope {
            AnimalIdentifierScopeInput::Lab => {
                if input.project_id.is_some() {
                    return Err(DesktopError::Store(StoreError::Validation(
                        "实验室编号命名空间不能携带项目".to_owned(),
                    )));
                }
                CreateAnimalIdentifierScope::Lab
            }
            AnimalIdentifierScopeInput::Project => {
                let project_id = parse_id(
                    "project",
                    input.project_id.as_deref().ok_or(DesktopError::Store(
                        StoreError::Validation("项目编号命名空间必须选择项目".to_owned()),
                    ))?,
                )?;
                CreateAnimalIdentifierScope::Project(project_id)
            }
        };
        let cage_id = input
            .cage_id
            .as_deref()
            .map(|value| parse_id("cage", value))
            .transpose()?;

        let now = Utc::now();
        let audit = self.audit("create_animal").await?;
        let animal = create_animal_use_case(
            &self.store,
            CreateAnimalCommand {
                lab_id: self.lab_id,
                identifier_scope,
                display_id: input.display_id,
                sex: input.sex,
                strain: Some(input.strain),
                birth_date,
                legacy_id: None,
                initial_cage_id: cage_id,
                initial_genotyping_records: input
                    .initial_genotyping_records
                    .into_iter()
                    .map(|record| InitialGenotypingRecordInput {
                        genotype_definition_id: record.genotype_definition_id,
                        state: record.state,
                        assessed_at: record.assessed_at,
                        method: record.method,
                        notes: record.notes,
                    })
                    .collect(),
                now,
            },
            &audit,
        )
        .await?;
        self.animal_detail(animal).await
    }

    pub(crate) async fn list_animals(&self) -> Result<Vec<AnimalView>, DesktopError> {
        const PAGE_SIZE: u32 = 1_000;
        const MAX_PAGES: u32 = 10;
        let filter = AnimalFilter {
            lab_id: self.lab_id,
            ..AnimalFilter::default()
        };
        let mut overviews = Vec::new();
        for page in 0..MAX_PAGES {
            let mut rows = self
                .store
                .list_animal_overviews(&filter, page * PAGE_SIZE, PAGE_SIZE)
                .await?;
            let complete = rows.len() < PAGE_SIZE as usize;
            overviews.append(&mut rows);
            if complete {
                return Ok(overviews.into_iter().map(animal_overview_view).collect());
            }
        }
        Err(StoreError::Validation("本地动物列表超过 10000 条，请使用筛选或导出".to_owned()).into())
    }

    pub(crate) async fn get_animal(&self, id: &str) -> Result<Option<AnimalView>, DesktopError> {
        let id = parse_id("animal", id)?;
        let animal = match self.store.get_animal(id).await {
            Ok(animal) => animal,
            Err(StoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if animal.lab_id != self.lab_id {
            return Ok(None);
        }
        self.animal_detail(animal).await.map(Some)
    }

    async fn animal_overview(&self, animal: Animal) -> Result<AnimalOverview, DesktopError> {
        self.store
            .list_animal_overviews(
                &AnimalFilter {
                    lab_id: self.lab_id,
                    query: Some(animal.display_id.clone()),
                    ..AnimalFilter::default()
                },
                0,
                500,
            )
            .await?
            .into_iter()
            .find(|candidate| candidate.animal.id == animal.id)
            .ok_or_else(|| {
                StoreError::NotFound {
                    entity: "animal",
                    id: animal.id,
                }
                .into()
            })
    }

    async fn animal_detail(&self, animal: Animal) -> Result<AnimalView, DesktopError> {
        let events = self.store.list_animal_events(animal.id).await?;
        let cage_names = self
            .store
            .list_cages(self.lab_id)
            .await?
            .into_iter()
            .map(|cage| (cage.id, cage.display_id))
            .collect::<BTreeMap<_, _>>();
        let operator_name = self.store.get_user(self.user_id).await?.display_name;
        let mut view = animal_overview_view(self.animal_overview(animal).await?);
        view.timeline = events
            .into_iter()
            .rev()
            .take(500)
            .map(|event| timeline_event(event, &cage_names, self.user_id, &operator_name))
            .collect();
        Ok(view)
    }

    pub(crate) async fn move_animals(&self, input: MoveAnimalsInput) -> Result<(), DesktopError> {
        let animal_ids = input
            .animal_ids
            .iter()
            .map(|id| parse_id("animal", id))
            .collect::<Result<Vec<_>, _>>()?;
        let target_cage_id = parse_id("target cage", &input.target_cage_id)?;
        let now = Utc::now();
        let audit = self.audit("transfer_animals").await?;
        transfer_animals_use_case(
            &self.store,
            TransferAnimalsCommand {
                lab_id: self.lab_id,
                animal_ids,
                target_cage_id,
                occurred_at: now,
                recorded_at: now,
                recorded_by: Some(self.user_id),
                notes: Some("通过 MuriArc 桌面端转笼".to_owned()),
            },
            &audit,
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn create_project(
        &self,
        input: CreateProjectInput,
    ) -> Result<ProjectView, DesktopError> {
        let audit = self.audit("create_project").await?;
        let project = create_project_use_case(
            &self.store,
            CreateProjectCommand {
                lab_id: self.lab_id,
                name: input.name,
                description: Some(input.description),
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(ProjectView {
            id: project.id.to_string(),
            name: project.name,
        })
    }

    pub(crate) async fn list_projects(&self) -> Result<Vec<ProjectView>, DesktopError> {
        Ok(self
            .store
            .list_projects(self.lab_id)
            .await?
            .into_iter()
            .map(|project| ProjectView {
                id: project.id.to_string(),
                name: project.name,
            })
            .collect())
    }

    pub(crate) async fn list_published_templates(&self) -> Result<Vec<TemplateView>, DesktopError> {
        Ok(self
            .store
            .list_template_versions(self.lab_id, None)
            .await?
            .into_iter()
            .filter(|template| template.status == TemplateStatus::Published)
            .map(template_view)
            .collect())
    }

    pub(crate) async fn create_published_template(
        &self,
        input: CreateTemplateInput,
    ) -> Result<TemplateView, DesktopError> {
        let now = Utc::now();
        let audit = self.audit("create_template_draft").await?;
        let template = create_template_version_use_case(
            &self.store,
            CreateTemplateVersionCommand {
                lab_id: self.lab_id,
                template_key: format!("local.{}", Uuid::new_v4().simple()),
                version: 1,
                name: input.name,
                description: Some(input.description),
                fields: vec![TemplateField {
                    key: input.field_key,
                    label: input.field_label,
                    value_type: input.field_value_type,
                    unit: Some(input.field_unit),
                    required: false,
                    categories: Vec::new(),
                    minimum: None,
                    maximum: None,
                    display_order: 0,
                    ai_writable: false,
                }],
                now,
            },
            &audit,
        )
        .await?;
        let publish_audit = self.audit("publish_template").await?;
        let published = publish_template_version_use_case(
            &self.store,
            PublishTemplateVersionCommand {
                id: template.id,
                expected_revision: template.meta.revision,
                published_by: self.user_id,
                published_at: Utc::now(),
            },
            &publish_audit,
        )
        .await?;
        Ok(template_view(published))
    }

    pub(crate) async fn create_experiment(
        &self,
        input: CreateExperimentInput,
    ) -> Result<ExperimentView, DesktopError> {
        let project_id = parse_id("project", &input.project_id)?;
        let template_id = parse_id("template", &input.template_version_id)?;
        let project = self.store.get_project(project_id).await?;
        if project.lab_id != self.lab_id {
            return Err(StoreError::NotFound {
                entity: "project",
                id: project_id,
            }
            .into());
        }
        let template = self.store.get_template_version(template_id).await?;
        if template.lab_id != self.lab_id || template.status != TemplateStatus::Published {
            return Err(DesktopError::Store(StoreError::Validation(
                "实验只能使用当前实验室已发布的模板".to_owned(),
            )));
        }

        let now = Utc::now();
        let starts_at = parse_date("experiment.start_date", input.start_date)?
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .map(|value| value.and_utc());
        let audit = self.audit("create_experiment").await?;
        let experiment = create_experiment_use_case(
            &self.store,
            CreateExperimentCommand {
                lab_id: self.lab_id,
                project_id,
                template_version_id: Some(template_id),
                name: input.name,
                description: Some(input.description),
                starts_at,
                now,
            },
            &audit,
        )
        .await?;
        Ok(experiment_view(
            experiment,
            project.name,
            0,
            0,
            1,
            Vec::new(),
        ))
    }

    pub(crate) async fn complete_experiment(
        &self,
        input: LifecycleTransitionInput,
    ) -> Result<ExperimentView, DesktopError> {
        self.transition_experiment(input, ExperimentStatus::Completed)
            .await
    }

    pub(crate) async fn cancel_experiment(
        &self,
        input: LifecycleTransitionInput,
    ) -> Result<ExperimentView, DesktopError> {
        self.transition_experiment(input, ExperimentStatus::Cancelled)
            .await
    }

    async fn transition_experiment(
        &self,
        input: LifecycleTransitionInput,
        target: ExperimentStatus,
    ) -> Result<ExperimentView, DesktopError> {
        let id = parse_id("experiment", &input.id)?;
        let experiment = self.store.get_experiment(id).await?;
        self.ensure_local_experiment(&experiment)?;
        let audit = self.audit("transition_experiment").await?;
        transition_experiment_use_case(
            &self.store,
            TransitionExperimentCommand {
                id,
                target,
                expected_revision: input.expected_revision,
                occurred_at: Utc::now(),
            },
            &audit,
        )
        .await?;
        self.list_experiments()
            .await?
            .into_iter()
            .find(|experiment| experiment.id == input.id)
            .ok_or_else(|| {
                StoreError::NotFound {
                    entity: "experiment",
                    id,
                }
                .into()
            })
    }

    pub(crate) async fn list_cohorts(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<CohortView>, DesktopError> {
        let experiment_id = parse_id("experiment", experiment_id)?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        Ok(self
            .store
            .list_cohorts(experiment_id)
            .await?
            .into_iter()
            .map(cohort_view)
            .collect())
    }

    pub(crate) async fn create_cohort(
        &self,
        input: CreateCohortInput,
    ) -> Result<CohortView, DesktopError> {
        let experiment_id = parse_id("experiment", &input.experiment_id)?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        let audit = self.audit("create_cohort").await?;
        let cohort = create_cohort_use_case(
            &self.store,
            CreateCohortCommand {
                experiment_id,
                name: input.name,
                description: Some(input.description),
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(cohort_view(cohort))
    }

    pub(crate) async fn list_participations(
        &self,
        project_id: &str,
        experiment_id: &str,
    ) -> Result<Vec<ParticipationView>, DesktopError> {
        let project_id = parse_id("project", project_id)?;
        let experiment_id = parse_id("experiment", experiment_id)?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        if experiment.project_id != project_id {
            return Err(StoreError::NotFound {
                entity: "experiment",
                id: experiment_id,
            }
            .into());
        }
        Ok(self
            .store
            .list_participations(&ParticipationFilter {
                project_id,
                experiment_id: Some(experiment_id),
                animal_id: None,
                cohort_id: None,
            })
            .await?
            .into_iter()
            .map(participation_view)
            .collect())
    }

    pub(crate) async fn enroll_animal(
        &self,
        input: EnrollAnimalInput,
    ) -> Result<ParticipationView, DesktopError> {
        let experiment_id = parse_id("experiment", &input.experiment_id)?;
        let animal_id = parse_id("animal", &input.animal_id)?;
        let cohort_id = input
            .cohort_id
            .as_deref()
            .map(|value| parse_id("cohort", value))
            .transpose()?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        let audit = self.audit("enroll_animal").await?;
        let participation = create_participation_use_case(
            &self.store,
            CreateParticipationCommand {
                experiment_id,
                animal_id,
                cohort_id,
                enrolled_at: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(participation_view(participation))
    }

    pub(crate) async fn complete_participation(
        &self,
        input: LifecycleTransitionInput,
    ) -> Result<ParticipationView, DesktopError> {
        self.transition_participation(input, ParticipationStatus::Completed)
            .await
    }

    pub(crate) async fn withdraw_participation(
        &self,
        input: LifecycleTransitionInput,
    ) -> Result<ParticipationView, DesktopError> {
        self.transition_participation(input, ParticipationStatus::Withdrawn)
            .await
    }

    async fn transition_participation(
        &self,
        input: LifecycleTransitionInput,
        target: ParticipationStatus,
    ) -> Result<ParticipationView, DesktopError> {
        let id = parse_id("participation", &input.id)?;
        let participation = self.store.get_participation(id).await?;
        let experiment = self
            .store
            .get_experiment(participation.experiment_id)
            .await?;
        self.ensure_local_experiment(&experiment)?;
        let audit = self.audit("transition_participation").await?;
        let participation = transition_participation_use_case(
            &self.store,
            TransitionParticipationCommand {
                id,
                target,
                expected_revision: input.expected_revision,
                occurred_at: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(participation_view(participation))
    }

    pub(crate) async fn list_procedures(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<ProcedureView>, DesktopError> {
        let experiment_id = parse_id("experiment", experiment_id)?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        Ok(self
            .store
            .list_procedures(experiment_id, None)
            .await?
            .into_iter()
            .map(procedure_view)
            .collect())
    }

    pub(crate) async fn create_procedure(
        &self,
        input: CreateProcedureInput,
    ) -> Result<ProcedureView, DesktopError> {
        let experiment_id = parse_id("experiment", &input.experiment_id)?;
        let experiment = self.store.get_experiment(experiment_id).await?;
        self.ensure_local_experiment(&experiment)?;
        let animal_id = input
            .animal_id
            .as_deref()
            .map(|value| parse_id("animal", value))
            .transpose()?;
        let scheduled_at = parse_datetime("procedure.scheduled_at", input.scheduled_at)?;
        let performed_at = parse_datetime("procedure.performed_at", input.performed_at)?;
        let audit = self.audit("create_procedure").await?;
        let procedure = create_procedure_use_case(
            &self.store,
            CreateProcedureCommand {
                experiment_id,
                animal_id,
                name: input.name,
                scheduled_at,
                performed_at,
                status: input.status,
                details: input.details.unwrap_or_else(|| serde_json::json!({})),
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(procedure_view(procedure))
    }

    fn ensure_local_experiment(&self, experiment: &Experiment) -> Result<(), DesktopError> {
        if experiment.lab_id == self.lab_id {
            Ok(())
        } else {
            Err(StoreError::NotFound {
                entity: "experiment",
                id: experiment.id,
            }
            .into())
        }
    }

    pub(crate) async fn list_experiments(&self) -> Result<Vec<ExperimentView>, DesktopError> {
        let mut result = Vec::new();
        for project in self.store.list_projects(self.lab_id).await? {
            for experiment in self
                .store
                .list_experiments(&ExperimentFilter {
                    project_id: project.id,
                    status: None,
                })
                .await?
            {
                let procedures = self.store.list_procedures(experiment.id, None).await?;
                let completed_steps = procedures
                    .iter()
                    .filter(|procedure| procedure.status == ProcedureStatus::Completed)
                    .count();
                let total_steps = procedures.len().max(1);
                let participations = self
                    .store
                    .list_participations(&ParticipationFilter {
                        project_id: project.id,
                        experiment_id: Some(experiment.id),
                        animal_id: None,
                        cohort_id: None,
                    })
                    .await?;
                let animal_count = participations.len();
                let cohort_counts = participations
                    .iter()
                    .filter_map(|participation| {
                        participation
                            .cohort_id
                            .map(|cohort_id| (cohort_id, 1_usize))
                    })
                    .fold(
                        BTreeMap::<Uuid, usize>::new(),
                        |mut counts, (cohort_id, increment)| {
                            *counts.entry(cohort_id).or_default() += increment;
                            counts
                        },
                    );
                let groups = self
                    .store
                    .list_cohorts(experiment.id)
                    .await?
                    .into_iter()
                    .enumerate()
                    .map(|(index, cohort)| ExperimentGroupView {
                        count: cohort_counts.get(&cohort.id).copied().unwrap_or_default(),
                        name: cohort.name,
                        color: ["#7398bd", "#009ca6", "#ef9f27", "#8d9c65"][index % 4].to_owned(),
                    })
                    .collect();
                result.push(experiment_view(
                    experiment,
                    project.name.clone(),
                    animal_count,
                    completed_steps,
                    total_steps,
                    groups,
                ));
            }
        }
        Ok(result)
    }

    pub(crate) async fn list_data_jobs(&self) -> Result<Vec<DataJobView>, DesktopError> {
        let jobs = self
            .store
            .list_jobs(&JobFilter {
                lab_id: self.lab_id,
                project_id: None,
                created_by: Some(self.user_id),
            })
            .await?;
        Ok(jobs
            .into_iter()
            .map(|job| {
                let progress = job
                    .progress_total
                    .filter(|total| *total > 0)
                    .map(|total| ((job.progress_current * 100) / total).clamp(0, 100) as i32)
                    .unwrap_or_else(|| i32::from(job.status == JobStatus::Completed) * 100);
                DataJobView {
                    id: job.id.to_string(),
                    name: job.idempotency_key,
                    kind: match job.kind {
                        JobKind::Import | JobKind::BulkOperation => "import",
                        JobKind::Export => "export",
                        JobKind::Snapshot => "snapshot",
                    }
                    .to_owned(),
                    status: match job.status {
                        JobStatus::Queued => "queued",
                        JobStatus::AwaitingConfirmation => "needs-review",
                        JobStatus::Completed => "completed",
                        JobStatus::Failed => "failed",
                        JobStatus::Cancelled => "cancelled",
                        JobStatus::Parsing | JobStatus::Validating | JobStatus::Writing => {
                            "running"
                        }
                    }
                    .to_owned(),
                    progress,
                    created_at: job.meta.created_at.to_rfc3339(),
                    detail: job_detail(job.status, job.result.as_ref(), job.error_report.as_ref()),
                }
            })
            .collect())
    }

    pub(crate) fn read_store(&self) -> &SqliteStore {
        &self.store
    }

    pub(crate) const fn lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub(crate) const fn user_id(&self) -> Uuid {
        self.user_id
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> &SqliteStore {
        &self.store
    }
}

async fn bootstrap_local_identity(store: &SqliteStore) -> Result<(), DesktopError> {
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Desktop);
    match store.get_lab(LOCAL_LAB_ID).await {
        Ok(_) => {}
        Err(StoreError::NotFound { .. }) => {
            let mut lab = muriarc_core::Lab::new("个人实验室", now)?;
            lab.id = LOCAL_LAB_ID;
            store.create_lab(&lab, &audit).await?;
        }
        Err(error) => return Err(error.into()),
    }
    match store.get_user(LOCAL_USER_ID).await {
        Ok(_) => {}
        Err(StoreError::NotFound { .. }) => {
            let mut user = User::new(
                LOCAL_LAB_ID,
                "local.operator@muriarc.invalid",
                LOCAL_OPERATOR_NAME,
                now,
            )?;
            user.id = LOCAL_USER_ID;
            store.create_user(&user, &audit).await?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn parse_id(field: &'static str, value: &str) -> Result<Uuid, DesktopError> {
    Uuid::parse_str(value).map_err(|_| DesktopError::InvalidId { field })
}

fn parse_date(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<NaiveDate>, DesktopError> {
    value
        .map(|value| {
            NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
                .map_err(|_| DesktopError::InvalidDate { field })
        })
        .transpose()
}

fn parse_datetime(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, DesktopError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value.trim())
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| DesktopError::InvalidDate { field })
        })
        .transpose()
}

fn normalized_required(
    field: &'static str,
    value: String,
    max: usize,
) -> Result<String, DesktopError> {
    let value = value.trim().to_owned();
    if value.chars().count() > max {
        return Err(DesktopError::TooLong { field, max });
    }
    if value.is_empty() {
        return Err(DomainError::EmptyField { field }.into());
    }
    Ok(value)
}

fn cage_views(cages: Vec<Cage>, animals: &[Animal]) -> Vec<CageView> {
    cages
        .into_iter()
        .map(|cage| {
            let residents = animals
                .iter()
                .filter(|animal| animal.current_cage_id == Some(cage.id))
                .collect::<Vec<_>>();
            cage_view(cage, &residents)
        })
        .collect()
}

fn cage_view(cage: Cage, residents: &[&Animal]) -> CageView {
    let strains = residents
        .iter()
        .filter_map(|animal| animal.strain.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    let resident_count = residents.len();
    CageView {
        id: cage.id.to_string(),
        code: cage.display_id,
        room: cage.section,
        rack: cage.location.unwrap_or_else(|| "未设置".to_owned()),
        capacity: cage.capacity,
        animal_ids: residents
            .iter()
            .map(|animal| animal.id.to_string())
            .collect(),
        status: if residents.is_empty() {
            "empty"
        } else if resident_count > cage.capacity as usize {
            "attention"
        } else {
            "normal"
        }
        .to_owned(),
        summary: if residents.is_empty() {
            "空笼".to_owned()
        } else if strains.is_empty() {
            format!("{resident_count} 只动物")
        } else {
            strains.into_iter().collect::<Vec<_>>().join(" · ")
        },
        note: None,
    }
}

fn animal_summary(animal: Animal) -> AnimalView {
    AnimalView {
        id: animal.id.to_string(),
        code: animal.display_id,
        legacy_code: animal.legacy_id,
        sex: match animal.sex {
            Sex::Male => "male",
            Sex::Female => "female",
            Sex::Unknown => "unknown",
        }
        .to_owned(),
        strain: animal.strain.unwrap_or_else(|| "未设置".to_owned()),
        genotype: "待确认".to_owned(),
        birth_date: animal
            .birth_date
            .map(|date| date.to_string())
            .unwrap_or_default(),
        status: match animal.current_status {
            AnimalStatus::InExperiment | AnimalStatus::Sampled => "experiment",
            AnimalStatus::Deceased
            | AnimalStatus::Euthanized
            | AnimalStatus::Lost
            | AnimalStatus::Archived => "archived",
            AnimalStatus::Planned | AnimalStatus::Alive => "active",
        }
        .to_owned(),
        cage_id: animal.current_cage_id.map(|id| id.to_string()),
        project_names: Vec::new(),
        project_refs: Vec::new(),
        weight: None,
        timeline: Vec::new(),
    }
}

fn animal_overview_view(overview: AnimalOverview) -> AnimalView {
    let genotype = if overview.genotype_labels.is_empty() {
        "待确认".to_owned()
    } else {
        overview.genotype_labels.join(" · ")
    };
    let project_refs = overview
        .projects
        .into_iter()
        .map(|project| ProjectView {
            id: project.id.to_string(),
            name: project.name,
        })
        .collect::<Vec<_>>();
    let project_names = project_refs
        .iter()
        .map(|project| project.name.clone())
        .collect();
    let weight = overview.latest_weight.map(|weight| weight.value);
    let mut view = animal_summary(overview.animal);
    view.genotype = genotype;
    view.project_names = project_names;
    view.project_refs = project_refs;
    view.weight = weight;
    view
}

fn template_view(template: ExperimentTemplateVersion) -> TemplateView {
    TemplateView {
        id: template.id.to_string(),
        name: template.name,
        version: template.version,
    }
}

fn experiment_view(
    experiment: Experiment,
    project: String,
    animal_count: usize,
    completed_steps: usize,
    total_steps: usize,
    groups: Vec<ExperimentGroupView>,
) -> ExperimentView {
    ExperimentView {
        id: experiment.id.to_string(),
        project_id: experiment.project_id.to_string(),
        code: format!("EXP-{}", &experiment.id.simple().to_string()[..8]).to_uppercase(),
        name: experiment.name,
        project,
        status: match experiment.status {
            ExperimentStatus::Active => "active",
            ExperimentStatus::Completed => "completed",
            ExperimentStatus::Cancelled | ExperimentStatus::Archived => "cancelled",
            ExperimentStatus::Draft => "draft",
        }
        .to_owned(),
        start_date: experiment
            .starts_at
            .map(|value| value.date_naive().to_string())
            .unwrap_or_default(),
        animal_count,
        completed_steps,
        total_steps,
        groups,
        next_action: None,
        revision: experiment.meta.revision,
    }
}

fn cohort_view(cohort: Cohort) -> CohortView {
    CohortView {
        id: cohort.id.to_string(),
        experiment_id: cohort.experiment_id.to_string(),
        name: cohort.name,
        description: cohort.description,
    }
}

fn participation_view(participation: Participation) -> ParticipationView {
    ParticipationView {
        id: participation.id.to_string(),
        experiment_id: participation.experiment_id.to_string(),
        animal_id: participation.animal_id.to_string(),
        cohort_id: participation.cohort_id.map(|id| id.to_string()),
        status: format!("{:?}", participation.status).to_ascii_lowercase(),
        enrolled_at: participation.enrolled_at.to_rfc3339(),
        exited_at: participation.exited_at.map(|value| value.to_rfc3339()),
        genotype_snapshot: participation
            .genotype_snapshot
            .into_iter()
            .map(|entry| GenotypeSnapshotEntryView {
                genotyping_record_id: entry.genotyping_record_id.to_string(),
                genotype_definition_id: entry.genotype_definition_id.to_string(),
                state: format!("{:?}", entry.state).to_ascii_lowercase(),
                assessed_at: entry.assessed_at.map(|value| value.to_rfc3339()),
            })
            .collect(),
        revision: participation.meta.revision,
    }
}

fn procedure_view(procedure: Procedure) -> ProcedureView {
    ProcedureView {
        id: procedure.id.to_string(),
        experiment_id: procedure.experiment_id.to_string(),
        animal_id: procedure.animal_id.map(|id| id.to_string()),
        name: procedure.name,
        scheduled_at: procedure.scheduled_at.map(|value| value.to_rfc3339()),
        performed_at: procedure.performed_at.map(|value| value.to_rfc3339()),
        status: match procedure.status {
            ProcedureStatus::Planned => "planned",
            ProcedureStatus::Completed => "completed",
            ProcedureStatus::Skipped => "skipped",
            ProcedureStatus::Cancelled => "cancelled",
        }
        .to_owned(),
        details: procedure.details,
    }
}

fn timeline_event(
    event: AnimalEvent,
    cages: &BTreeMap<Uuid, String>,
    local_user_id: Uuid,
    local_operator_name: &str,
) -> TimelineEventView {
    let (event_type, title, detail) = match &event.kind {
        AnimalEventKind::Registered => ("note", "登记动物".to_owned(), "创建动物档案".to_owned()),
        AnimalEventKind::Born { birth_date } => (
            "birth",
            "出生登记".to_owned(),
            format!("出生日期 {birth_date}"),
        ),
        AnimalEventKind::Transferred {
            from_cage_id,
            to_cage_id,
        } => {
            let from = from_cage_id
                .and_then(|id| cages.get(&id))
                .map(String::as_str)
                .unwrap_or("未分配");
            let to = to_cage_id
                .and_then(|id| cages.get(&id))
                .map(String::as_str)
                .unwrap_or("未分配");
            ("transfer", "转笼".to_owned(), format!("{from} → {to}"))
        }
        AnimalEventKind::StatusChanged { from, to } => {
            ("note", "状态变更".to_owned(), format!("{from:?} → {to:?}"))
        }
        AnimalEventKind::Genotyped { .. } => (
            "genotype",
            "基因型记录".to_owned(),
            "已更新基因型".to_owned(),
        ),
        AnimalEventKind::GenotypingRecorded { state, .. } => (
            "genotype",
            "基因检测记录".to_owned(),
            format!("检测状态：{state:?}"),
        ),
        AnimalEventKind::ExperimentEnrolled { .. } | AnimalEventKind::ProcedurePerformed { .. } => {
            (
                "experiment",
                "实验记录".to_owned(),
                "已关联实验过程".to_owned(),
            )
        }
        AnimalEventKind::ExperimentParticipationEnded { status, .. } => (
            "experiment",
            "实验参与结束".to_owned(),
            match status {
                ParticipationStatus::Completed => "已完成实验参与",
                ParticipationStatus::Withdrawn => "已退出实验",
                ParticipationStatus::Enrolled => "实验参与状态已更新",
            }
            .to_owned(),
        ),
        AnimalEventKind::MeasurementRecorded { .. } => (
            "measurement",
            "记录测量".to_owned(),
            "已关联测量数据".to_owned(),
        ),
        AnimalEventKind::SampleCollected { .. } => (
            "sampling",
            "采集样本".to_owned(),
            "已关联采样记录".to_owned(),
        ),
        AnimalEventKind::Note { body } => ("note", "备注".to_owned(), body.clone()),
    };
    TimelineEventView {
        id: event.id.to_string(),
        at: event.occurred_at.to_rfc3339(),
        event_type: event_type.to_owned(),
        title,
        detail: event.notes.unwrap_or(detail),
        operator: if event.recorded_by == Some(local_user_id) {
            local_operator_name
        } else {
            "MuriArc"
        }
        .to_owned(),
    }
}

fn job_detail(status: JobStatus, result: Option<&Value>, error: Option<&Value>) -> String {
    match status {
        JobStatus::Failed => "任务执行失败，请查看错误报告".to_owned(),
        JobStatus::Cancelled => "任务已取消".to_owned(),
        JobStatus::Completed if result.is_some() => "任务已生成结果".to_owned(),
        JobStatus::Completed => "任务已完成".to_owned(),
        _ if error.is_some() => "任务未完成，请查看错误报告".to_owned(),
        _ => "任务处理中".to_owned(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSettingsView {
    pub lab_name: String,
    pub operator_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveWorkspaceSettingsInput {
    pub lab_name: String,
    pub operator_name: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnimalIdentifierScopeInput {
    Lab,
    Project,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAnimalInput {
    pub display_id: String,
    pub identifier_scope: AnimalIdentifierScopeInput,
    pub project_id: Option<String>,
    pub cage_id: Option<String>,
    pub sex: Sex,
    pub strain: String,
    pub birth_date: Option<String>,
    #[serde(default)]
    pub initial_genotyping_records: Vec<InitialGenotypingRecordInputView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InitialGenotypingRecordInputView {
    pub genotype_definition_id: Uuid,
    pub state: GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateProjectInput {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateTemplateInput {
    pub name: String,
    pub description: String,
    pub field_key: String,
    pub field_label: String,
    pub field_value_type: FieldValueType,
    pub field_unit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateExperimentInput {
    pub project_id: String,
    pub template_version_id: String,
    pub name: String,
    pub description: String,
    pub start_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LifecycleTransitionInput {
    pub id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateCohortInput {
    pub experiment_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EnrollAnimalInput {
    pub experiment_id: String,
    pub animal_id: String,
    pub cohort_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateProcedureInput {
    pub experiment_id: String,
    pub animal_id: Option<String>,
    pub name: String,
    pub scheduled_at: Option<String>,
    pub performed_at: Option<String>,
    pub status: ProcedureStatus,
    pub details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateCageInput {
    pub code: String,
    pub room: String,
    pub rack: String,
    pub capacity: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MoveAnimalsInput {
    pub animal_ids: Vec<String>,
    pub target_cage_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CageView {
    id: String,
    code: String,
    room: String,
    rack: String,
    capacity: i32,
    animal_ids: Vec<String>,
    status: String,
    summary: String,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnimalView {
    id: String,
    code: String,
    legacy_code: Option<String>,
    sex: String,
    strain: String,
    genotype: String,
    birth_date: String,
    status: String,
    cage_id: Option<String>,
    project_names: Vec<String>,
    project_refs: Vec<ProjectView>,
    weight: Option<f64>,
    timeline: Vec<TimelineEventView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineEventView {
    id: String,
    at: String,
    #[serde(rename = "type")]
    event_type: String,
    title: String,
    detail: String,
    operator: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateView {
    id: String,
    name: String,
    version: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CohortView {
    id: String,
    experiment_id: String,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParticipationView {
    id: String,
    experiment_id: String,
    animal_id: String,
    cohort_id: Option<String>,
    status: String,
    enrolled_at: String,
    exited_at: Option<String>,
    genotype_snapshot: Vec<GenotypeSnapshotEntryView>,
    revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenotypeSnapshotEntryView {
    genotyping_record_id: String,
    genotype_definition_id: String,
    state: String,
    assessed_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcedureView {
    id: String,
    experiment_id: String,
    animal_id: Option<String>,
    name: String,
    scheduled_at: Option<String>,
    performed_at: Option<String>,
    status: String,
    details: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectView {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExperimentView {
    id: String,
    project_id: String,
    code: String,
    name: String,
    project: String,
    status: String,
    start_date: String,
    animal_count: usize,
    completed_steps: usize,
    total_steps: usize,
    groups: Vec<ExperimentGroupView>,
    next_action: Option<String>,
    revision: i64,
}

#[derive(Debug, Serialize)]
struct ExperimentGroupView {
    name: String,
    count: usize,
    color: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataJobView {
    id: String,
    name: String,
    kind: String,
    status: String,
    progress: i32,
    created_at: String,
    detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use muriarc_core::{
        AuditAction, AuditFilter, Cohort, EntityType, Experiment, Participation, Project,
        RecordMeta,
    };
    use tempfile::tempdir;

    #[tokio::test]
    async fn bootstrap_is_idempotent_and_creates_real_local_identity() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("muriarc.sqlite3");
        let state = DesktopState::initialize(&path).await.unwrap();
        assert_eq!(
            state.store().get_lab(LOCAL_LAB_ID).await.unwrap().name,
            "个人实验室"
        );
        assert_eq!(
            state
                .store()
                .get_user(LOCAL_USER_ID)
                .await
                .unwrap()
                .display_name,
            LOCAL_OPERATOR_NAME
        );
        drop(state);
        let reopened = DesktopState::initialize(&path).await.unwrap();
        assert!(reopened.list_cages().await.unwrap().is_empty());
        drop(reopened);
    }

    #[tokio::test]
    async fn transfer_is_atomic_and_writes_event_and_human_audit() {
        let temp = tempdir().unwrap();
        let state = DesktopState::initialize(temp.path().join("muriarc.sqlite3"))
            .await
            .unwrap();
        let target = state
            .create_cage(CreateCageInput {
                code: "A01".to_owned(),
                room: "SPF-A".to_owned(),
                rack: "R1".to_owned(),
                capacity: 1,
            })
            .await
            .unwrap();
        let target_id = Uuid::parse_str(&target.id).unwrap();
        let now = Utc::now();
        let first = Animal::new_mouse(LOCAL_LAB_ID, "M001", Sex::Female, now).unwrap();
        let second = Animal::new_mouse(LOCAL_LAB_ID, "M002", Sex::Male, now).unwrap();
        let audit = AuditContext::system(WriteSource::Desktop);
        state.store().create_animal(&first, &audit).await.unwrap();
        state.store().create_animal(&second, &audit).await.unwrap();

        state
            .move_animals(MoveAnimalsInput {
                animal_ids: vec![first.id.to_string()],
                target_cage_id: target.id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            state
                .store()
                .get_animal(first.id)
                .await
                .unwrap()
                .current_cage_id,
            Some(target_id)
        );
        let events = state.store().list_animal_events(first.id).await.unwrap();
        assert!(events.iter().any(|event| {
            matches!(event.kind, AnimalEventKind::Transferred { .. })
                && event.recorded_by == Some(LOCAL_USER_ID)
        }));

        let rejected = state
            .move_animals(MoveAnimalsInput {
                animal_ids: vec![second.id.to_string()],
                target_cage_id: target.id,
            })
            .await;
        assert!(matches!(
            rejected,
            Err(DesktopError::Store(StoreError::Conflict(_)))
        ));
        assert_eq!(
            state
                .store()
                .get_animal(second.id)
                .await
                .unwrap()
                .current_cage_id,
            None
        );
        assert!(
            !state
                .store()
                .list_animal_events(second.id)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::Transferred { .. }))
        );

        let audits = state
            .store()
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(first.id),
            })
            .await
            .unwrap();
        assert!(audits.iter().any(|entry| {
            entry.entity_type == EntityType::Animal
                && entry.action == AuditAction::Update
                && entry.actor.user_id == Some(LOCAL_USER_ID)
                && entry.source == WriteSource::Desktop
        }));
        drop(state);
    }

    #[tokio::test]
    async fn workspace_settings_persist_and_future_audits_use_latest_operator() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("muriarc.sqlite3");
        let state = DesktopState::initialize(&path).await.unwrap();
        let saved = state
            .save_workspace_settings(SaveWorkspaceSettingsInput {
                lab_name: "呼吸研究实验室".to_owned(),
                operator_name: "研究者甲".to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(saved.lab_name, "呼吸研究实验室");
        assert_eq!(saved.operator_name, "研究者甲");

        let cage = state
            .create_cage(CreateCageInput {
                code: "S01".to_owned(),
                room: "SPF".to_owned(),
                rack: "R1".to_owned(),
                capacity: 5,
            })
            .await
            .unwrap();
        let audits = state
            .store()
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(Uuid::parse_str(&cage.id).unwrap()),
            })
            .await
            .unwrap();
        assert!(audits.iter().any(|entry| {
            entry.actor.user_id == Some(LOCAL_USER_ID)
                && entry.actor.display_name == "研究者甲"
                && entry.source == WriteSource::Desktop
        }));
        drop(state);

        let reopened = DesktopState::initialize(&path).await.unwrap();
        let loaded = reopened.get_workspace_settings().await.unwrap();
        assert_eq!(loaded.lab_name, "呼吸研究实验室");
        assert_eq!(loaded.operator_name, "研究者甲");
    }

    #[tokio::test]
    async fn local_crud_chain_uses_published_templates_and_atomic_animal_events() {
        let temp = tempdir().unwrap();
        let state = DesktopState::initialize(temp.path().join("muriarc.sqlite3"))
            .await
            .unwrap();
        let cage = state
            .create_cage(CreateCageInput {
                code: "A01".to_owned(),
                room: "SPF".to_owned(),
                rack: "R1".to_owned(),
                capacity: 5,
            })
            .await
            .unwrap();
        let project = state
            .create_project(CreateProjectInput {
                name: "DEMO".to_owned(),
                description: "动物实验".to_owned(),
            })
            .await
            .unwrap();
        let template = state
            .create_published_template(CreateTemplateInput {
                name: "体重观察".to_owned(),
                description: "通用模板".to_owned(),
                field_key: "body_weight".to_owned(),
                field_label: "体重".to_owned(),
                field_value_type: FieldValueType::Number,
                field_unit: "g".to_owned(),
            })
            .await
            .unwrap();
        let animal = state
            .create_animal(CreateAnimalInput {
                display_id: "M-001".to_owned(),
                identifier_scope: AnimalIdentifierScopeInput::Project,
                project_id: Some(project.id.clone()),
                cage_id: Some(cage.id),
                sex: Sex::Female,
                strain: "C57BL/6J".to_owned(),
                birth_date: Some("2026-06-01".to_owned()),
                initial_genotyping_records: Vec::new(),
            })
            .await
            .unwrap();
        let animal_id = Uuid::parse_str(&animal.id).unwrap();
        let events = state.store().list_animal_events(animal_id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::Registered))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::Born { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::Transferred { .. }))
        );
        assert!(
            events
                .iter()
                .all(|event| event.recorded_by == Some(LOCAL_USER_ID))
        );

        let experiment = state
            .create_experiment(CreateExperimentInput {
                project_id: project.id.clone(),
                template_version_id: template.id,
                name: "DEMO-001".to_owned(),
                description: String::new(),
                start_date: Some("2026-07-19".to_owned()),
            })
            .await
            .unwrap();
        let cohort = state
            .create_cohort(CreateCohortInput {
                experiment_id: experiment.id.clone(),
                name: "Control".to_owned(),
                description: String::new(),
            })
            .await
            .unwrap();
        let participation = state
            .enroll_animal(EnrollAnimalInput {
                experiment_id: experiment.id.clone(),
                animal_id: animal.id.clone(),
                cohort_id: Some(cohort.id),
            })
            .await
            .unwrap();
        state
            .create_procedure(CreateProcedureInput {
                experiment_id: experiment.id.clone(),
                animal_id: Some(animal.id),
                name: "给药".to_owned(),
                scheduled_at: None,
                performed_at: Some("2026-07-19T08:00:00Z".to_owned()),
                status: ProcedureStatus::Completed,
                details: Some(serde_json::json!({"dose": "vehicle"})),
            })
            .await
            .unwrap();

        let projected = state.store().get_animal(animal_id).await.unwrap();
        assert_eq!(projected.current_status, AnimalStatus::InExperiment);
        let events = state.store().list_animal_events(animal_id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::ExperimentEnrolled { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::ProcedurePerformed { .. }))
        );

        let completed = state
            .complete_experiment(LifecycleTransitionInput {
                id: experiment.id,
                expected_revision: experiment.revision,
            })
            .await
            .unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(
            state
                .store()
                .get_animal(animal_id)
                .await
                .unwrap()
                .current_status,
            AnimalStatus::Alive
        );
        assert_eq!(
            state
                .store()
                .get_participation(Uuid::parse_str(&participation.id).unwrap())
                .await
                .unwrap()
                .status,
            ParticipationStatus::Completed
        );
        assert!(
            state
                .store()
                .list_animal_events(animal_id)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(
                    event.kind,
                    AnimalEventKind::ExperimentParticipationEnded {
                        status: ParticipationStatus::Completed,
                        ..
                    }
                ))
        );
    }

    #[tokio::test]
    async fn experiment_views_use_real_participation_and_cohort_counts() {
        let temp = tempdir().unwrap();
        let state = DesktopState::initialize(temp.path().join("muriarc.sqlite3"))
            .await
            .unwrap();
        let now = Utc::now();
        let audit = AuditContext::system(WriteSource::Desktop);
        let project = Project::new(LOCAL_LAB_ID, "DEMO", now).unwrap();
        state
            .store()
            .create_project(&project, &audit)
            .await
            .unwrap();
        let experiment = Experiment::new(LOCAL_LAB_ID, project.id, "DEMO-001", now).unwrap();
        state
            .store()
            .create_experiment(&experiment, &audit)
            .await
            .unwrap();
        let cohort = Cohort {
            id: Uuid::new_v4(),
            experiment_id: experiment.id,
            name: "Control".to_owned(),
            description: None,
            meta: RecordMeta::new(now),
        };
        state.store().create_cohort(&cohort, &audit).await.unwrap();

        for code in ["M001", "M002"] {
            let animal = Animal::new_mouse(LOCAL_LAB_ID, code, Sex::Female, now).unwrap();
            state.store().create_animal(&animal, &audit).await.unwrap();
            let mut participation = Participation::enroll(experiment.id, animal.id, now);
            participation.cohort_id = Some(cohort.id);
            state
                .store()
                .create_participation(&participation, &audit)
                .await
                .unwrap();
        }

        let views = state.list_experiments().await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].animal_count, 2);
        assert_eq!(views[0].groups.len(), 1);
        assert_eq!(views[0].groups[0].count, 2);
    }

    #[tokio::test]
    async fn reviewed_real_migration_is_visible_to_desktop_when_configured() {
        let (Ok(path), Ok(expected_animals), Ok(expected_cages)) = (
            std::env::var("MURIARC_TEST_MIGRATED_DATABASE"),
            std::env::var("MURIARC_TEST_MIGRATED_ANIMALS"),
            std::env::var("MURIARC_TEST_MIGRATED_CAGES"),
        ) else {
            return;
        };
        let expected_animals = expected_animals
            .parse::<usize>()
            .expect("MURIARC_TEST_MIGRATED_ANIMALS must be a non-negative integer");
        let expected_cages = expected_cages
            .parse::<usize>()
            .expect("MURIARC_TEST_MIGRATED_CAGES must be a non-negative integer");
        let state = DesktopState::initialize(path).await.unwrap();
        let animals = state.list_animals().await.unwrap();
        assert_eq!(animals.len(), expected_animals);
        assert!(animals.iter().any(|animal| animal.genotype != "待确认"));
        assert_eq!(state.list_cages().await.unwrap().len(), expected_cages);
        drop(state);
    }
}
