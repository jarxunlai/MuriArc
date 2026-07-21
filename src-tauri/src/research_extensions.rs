use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::{
    CorrectGenotypingRecordCommand, CreateAnimalDraftInput as CreateAnimalDraftCommandInput,
    CreateAnimalIdentifierScope, CreateBreedingLineCommand, CreateBreedingPairCommand,
    CreateColonyCommand, CreateExperimentEventCommand, CreateGenotypeComponentInput,
    CreateGenotypeDefinitionCommand, CreateGenotypingRecordCommand, CreateLitterCommand,
    CreateMatingEventCommand, CreateObservationDefinitionCommand, GeneticsArchiveCommand,
    RecordObservationCommand, RegisterAnimalDraftCommand, ReviseObservationValueCommand,
    VoidGenotypingRecordCommand,
    archive_genotype_definition as archive_genotype_definition_use_case, breeding_prediction,
    correct_genotyping_record as correct_genotyping_record_use_case, create_breeding_line,
    create_breeding_pair, create_colony, create_experiment_event, create_genotype_definition,
    create_genotyping_record, create_litter, create_mating_event, create_observation_definition,
    record_observation, register_animal_draft,
    restore_genotype_definition as restore_genotype_definition_use_case, retire_breeding_pair,
    revise_observation_value, void_genotyping_record as void_genotyping_record_use_case,
};
use muriarc_core::{
    Animal, AnimalDraft, BreedingLine, BreedingPair, Colony, ExperimentEvent,
    GeneticsReferenceCounts, GenotypeComponentMode, GenotypeDefinition, GenotypingRecord,
    GenotypingState, Litter, LocusPrediction, MatingEvent, MuriArcStore, Observation,
    ObservationDefinition, ObservationFilter, ObservationPolicy, ObservationSubjectType,
    ObservationValueData, ObservationValueRecord, ObservationValueType, Sex, StoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::application::{DesktopError, DesktopState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GenotypeDefinitionComponentInput {
    locus_id: Uuid,
    allele_1_id: Uuid,
    allele_2_id: Option<Uuid>,
    mode: GenotypeComponentMode,
    display_order: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateGenotypeDefinitionInput {
    name: String,
    description: Option<String>,
    components: Vec<GenotypeDefinitionComponentInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateGenotypingRecordInput {
    project_id: Option<Uuid>,
    animal_id: Uuid,
    genotype_definition_id: Uuid,
    state: GenotypingState,
    assessed_at: Option<DateTime<Utc>>,
    method: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GeneticsArchiveInput {
    pub(crate) id: Uuid,
    pub(crate) expected_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VoidGenotypingRecordInput {
    record_id: Uuid,
    expected_revision: i64,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CorrectGenotypingRecordInput {
    record_id: Uuid,
    expected_revision: i64,
    reason: String,
    genotype_definition_id: Uuid,
    state: GenotypingState,
    assessed_at: Option<DateTime<Utc>>,
    method: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CorrectGenotypingRecordView {
    voided: GenotypingRecord,
    replacement: GenotypingRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateBreedingLineInput {
    name: String,
    description: Option<String>,
    genotype_definition_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateColonyInput {
    breeding_line_id: Uuid,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateBreedingPairInput {
    colony_id: Uuid,
    name: String,
    male_animal_id: Uuid,
    female_animal_ids: Vec<Uuid>,
    started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RetireBreedingPairInput {
    id: Uuid,
    expected_revision: i64,
    ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateMatingEventInput {
    breeding_pair_id: Uuid,
    male_animal_id: Uuid,
    female_animal_id: Uuid,
    occurred_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AnimalDraftInput {
    temporary_label: String,
    sex: Sex,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateLitterInput {
    mating_event_id: Uuid,
    born_on: NaiveDate,
    size_total: i32,
    drafts: Vec<AnimalDraftInput>,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreatedLitterView {
    pub litter: Litter,
    pub drafts: Vec<AnimalDraft>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResearchIdentifierScopeInput {
    Lab,
    Project,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RegisterAnimalDraftInput {
    draft_id: Uuid,
    expected_revision: i64,
    identifier_scope: ResearchIdentifierScopeInput,
    project_id: Option<Uuid>,
    display_id: String,
    strain: Option<String>,
    initial_cage_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RegisteredAnimalDraftView {
    pub draft: AnimalDraft,
    pub animal: Animal,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BreedingPredictionInput {
    male_genotype_definition_id: Uuid,
    female_genotype_definition_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateExperimentEventInput {
    experiment_id: Uuid,
    event_key: String,
    label: String,
    occurred_at: Option<DateTime<Utc>>,
    #[serde(default = "empty_object")]
    details: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateObservationDefinitionInput {
    experiment_id: Uuid,
    key: String,
    label: String,
    value_type: ObservationValueType,
    unit: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    policy: ObservationPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordObservationInput {
    experiment_id: Uuid,
    experiment_event_id: Uuid,
    definition_id: Uuid,
    subject_type: ObservationSubjectType,
    subject_id: Uuid,
    #[serde(default = "empty_object")]
    context: Value,
    value: ObservationValueData,
    recorded_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReviseObservationInput {
    observation_id: Uuid,
    expected_revision: i64,
    value: ObservationValueData,
    recorded_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecordedObservationView {
    pub observation: Observation,
    pub value: ObservationValueRecord,
}

impl DesktopState {
    pub(crate) async fn list_genotype_definitions(
        &self,
        include_archived: bool,
    ) -> Result<Vec<GenotypeDefinition>, DesktopError> {
        if include_archived {
            Ok(self
                .domain_store()
                .list_genotype_definitions_including_archived(self.local_lab_id())
                .await?)
        } else {
            Ok(self
                .domain_store()
                .list_genotype_definitions(self.local_lab_id())
                .await?)
        }
    }

    pub(crate) async fn genotype_definition_references(
        &self,
        id: Uuid,
    ) -> Result<GeneticsReferenceCounts, DesktopError> {
        let definition = self.domain_store().get_genotype_definition(id).await?;
        if definition.lab_id != self.local_lab_id() {
            return Err(DesktopError::Store(StoreError::Validation(
                "genotype definition belongs to another lab".to_owned(),
            )));
        }
        Ok(self
            .domain_store()
            .genotype_definition_reference_counts(id)
            .await?)
    }

    pub(crate) async fn archive_genotype_definition(
        &self,
        input: GeneticsArchiveInput,
    ) -> Result<GenotypeDefinition, DesktopError> {
        let audit = self.audit("archive_genotype_definition").await?;
        Ok(archive_genotype_definition_use_case(
            self.domain_store(),
            GeneticsArchiveCommand {
                id: input.id,
                expected_revision: input.expected_revision,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn restore_genotype_definition(
        &self,
        input: GeneticsArchiveInput,
    ) -> Result<GenotypeDefinition, DesktopError> {
        let audit = self.audit("restore_genotype_definition").await?;
        Ok(restore_genotype_definition_use_case(
            self.domain_store(),
            GeneticsArchiveCommand {
                id: input.id,
                expected_revision: input.expected_revision,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn create_genotype_definition(
        &self,
        input: CreateGenotypeDefinitionInput,
    ) -> Result<GenotypeDefinition, DesktopError> {
        let audit = self.audit("create_genotype_definition").await?;
        Ok(create_genotype_definition(
            self.domain_store(),
            CreateGenotypeDefinitionCommand {
                lab_id: self.local_lab_id(),
                name: input.name,
                description: input.description,
                components: input
                    .components
                    .into_iter()
                    .map(|component| CreateGenotypeComponentInput {
                        locus_id: component.locus_id,
                        allele_1_id: component.allele_1_id,
                        allele_2_id: component.allele_2_id,
                        mode: component.mode,
                        display_order: component.display_order,
                    })
                    .collect(),
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_genotyping_records(
        &self,
        animal_id: Uuid,
    ) -> Result<Vec<GenotypingRecord>, DesktopError> {
        Ok(self
            .domain_store()
            .list_genotyping_records(animal_id)
            .await?)
    }

    pub(crate) async fn create_genotyping_record(
        &self,
        input: CreateGenotypingRecordInput,
    ) -> Result<GenotypingRecord, DesktopError> {
        let audit = self.audit("create_genotyping_record").await?;
        Ok(create_genotyping_record(
            self.domain_store(),
            CreateGenotypingRecordCommand {
                lab_id: self.local_lab_id(),
                project_id: input.project_id,
                animal_id: input.animal_id,
                genotype_definition_id: input.genotype_definition_id,
                state: input.state,
                assessed_at: input.assessed_at,
                method: input.method,
                notes: input.notes,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn void_genotyping_record(
        &self,
        input: VoidGenotypingRecordInput,
    ) -> Result<GenotypingRecord, DesktopError> {
        let audit = self.audit("void_genotyping_record").await?;
        Ok(void_genotyping_record_use_case(
            self.domain_store(),
            VoidGenotypingRecordCommand {
                record_id: input.record_id,
                expected_revision: input.expected_revision,
                reason: input.reason,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn correct_genotyping_record(
        &self,
        input: CorrectGenotypingRecordInput,
    ) -> Result<CorrectGenotypingRecordView, DesktopError> {
        let audit = self.audit("correct_genotyping_record").await?;
        let (voided, replacement) = correct_genotyping_record_use_case(
            self.domain_store(),
            CorrectGenotypingRecordCommand {
                record_id: input.record_id,
                expected_revision: input.expected_revision,
                reason: input.reason,
                genotype_definition_id: input.genotype_definition_id,
                state: input.state,
                assessed_at: input.assessed_at,
                method: input.method,
                notes: input.notes,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(CorrectGenotypingRecordView {
            voided,
            replacement,
        })
    }

    pub(crate) async fn list_breeding_lines(&self) -> Result<Vec<BreedingLine>, DesktopError> {
        Ok(self
            .domain_store()
            .list_breeding_lines(self.local_lab_id())
            .await?)
    }

    pub(crate) async fn create_breeding_line(
        &self,
        input: CreateBreedingLineInput,
    ) -> Result<BreedingLine, DesktopError> {
        let audit = self.audit("create_breeding_line").await?;
        Ok(create_breeding_line(
            self.domain_store(),
            CreateBreedingLineCommand {
                lab_id: self.local_lab_id(),
                name: input.name,
                description: input.description,
                genotype_definition_ids: input.genotype_definition_ids,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_colonies(
        &self,
        breeding_line_id: Option<Uuid>,
    ) -> Result<Vec<Colony>, DesktopError> {
        Ok(self
            .domain_store()
            .list_colonies(self.local_lab_id(), breeding_line_id)
            .await?)
    }

    pub(crate) async fn create_colony(
        &self,
        input: CreateColonyInput,
    ) -> Result<Colony, DesktopError> {
        let audit = self.audit("create_colony").await?;
        Ok(create_colony(
            self.domain_store(),
            CreateColonyCommand {
                lab_id: self.local_lab_id(),
                breeding_line_id: input.breeding_line_id,
                name: input.name,
                description: input.description,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_breeding_pairs(
        &self,
        colony_id: Option<Uuid>,
    ) -> Result<Vec<BreedingPair>, DesktopError> {
        Ok(self
            .domain_store()
            .list_breeding_pairs(self.local_lab_id(), colony_id)
            .await?)
    }

    pub(crate) async fn create_breeding_pair(
        &self,
        input: CreateBreedingPairInput,
    ) -> Result<BreedingPair, DesktopError> {
        let now = Utc::now();
        let audit = self.audit("create_breeding_pair").await?;
        Ok(create_breeding_pair(
            self.domain_store(),
            CreateBreedingPairCommand {
                lab_id: self.local_lab_id(),
                colony_id: input.colony_id,
                name: input.name,
                male_animal_id: input.male_animal_id,
                female_animal_ids: input.female_animal_ids,
                started_at: input.started_at.unwrap_or(now),
                now,
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn retire_breeding_pair(
        &self,
        input: RetireBreedingPairInput,
    ) -> Result<BreedingPair, DesktopError> {
        let audit = self.audit("retire_breeding_pair").await?;
        Ok(retire_breeding_pair(
            self.domain_store(),
            input.id,
            input.expected_revision,
            input.ended_at.unwrap_or_else(Utc::now),
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_mating_events(
        &self,
        breeding_pair_id: Uuid,
    ) -> Result<Vec<MatingEvent>, DesktopError> {
        Ok(self
            .domain_store()
            .list_mating_events(breeding_pair_id)
            .await?)
    }

    pub(crate) async fn create_mating_event(
        &self,
        input: CreateMatingEventInput,
    ) -> Result<MatingEvent, DesktopError> {
        let now = Utc::now();
        let audit = self.audit("create_mating_event").await?;
        Ok(create_mating_event(
            self.domain_store(),
            CreateMatingEventCommand {
                lab_id: self.local_lab_id(),
                breeding_pair_id: input.breeding_pair_id,
                male_animal_id: input.male_animal_id,
                female_animal_id: input.female_animal_id,
                occurred_at: input.occurred_at.unwrap_or(now),
                notes: input.notes,
                now,
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_litters(
        &self,
        breeding_pair_id: Uuid,
    ) -> Result<Vec<Litter>, DesktopError> {
        Ok(self.domain_store().list_litters(breeding_pair_id).await?)
    }

    pub(crate) async fn create_litter(
        &self,
        input: CreateLitterInput,
    ) -> Result<CreatedLitterView, DesktopError> {
        let audit = self.audit("create_litter").await?;
        let created = create_litter(
            self.domain_store(),
            CreateLitterCommand {
                lab_id: self.local_lab_id(),
                mating_event_id: input.mating_event_id,
                born_on: input.born_on,
                size_total: input.size_total,
                drafts: input
                    .drafts
                    .into_iter()
                    .map(|draft| CreateAnimalDraftCommandInput {
                        temporary_label: draft.temporary_label,
                        sex: draft.sex,
                    })
                    .collect(),
                notes: input.notes,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(CreatedLitterView {
            litter: created.litter,
            drafts: created.drafts,
        })
    }

    pub(crate) async fn list_animal_drafts(
        &self,
        litter_id: Uuid,
    ) -> Result<Vec<AnimalDraft>, DesktopError> {
        Ok(self.domain_store().list_animal_drafts(litter_id).await?)
    }

    pub(crate) async fn register_animal_draft(
        &self,
        input: RegisterAnimalDraftInput,
    ) -> Result<RegisteredAnimalDraftView, DesktopError> {
        let identifier_scope = match input.identifier_scope {
            ResearchIdentifierScopeInput::Lab => CreateAnimalIdentifierScope::Lab,
            ResearchIdentifierScopeInput::Project => {
                CreateAnimalIdentifierScope::Project(input.project_id.ok_or({
                    DesktopError::InvalidId {
                        field: "project_id",
                    }
                })?)
            }
        };
        let audit = self.audit("register_animal_draft").await?;
        let registered = register_animal_draft(
            self.domain_store(),
            RegisterAnimalDraftCommand {
                lab_id: self.local_lab_id(),
                draft_id: input.draft_id,
                expected_revision: input.expected_revision,
                identifier_scope,
                display_id: input.display_id,
                strain: input.strain,
                initial_cage_id: input.initial_cage_id,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(RegisteredAnimalDraftView {
            draft: registered.draft,
            animal: registered.animal,
        })
    }

    pub(crate) async fn breeding_prediction(
        &self,
        input: BreedingPredictionInput,
    ) -> Result<Vec<LocusPrediction>, DesktopError> {
        Ok(breeding_prediction(
            self.domain_store(),
            input.male_genotype_definition_id,
            input.female_genotype_definition_id,
        )
        .await?)
    }

    pub(crate) async fn list_experiment_events(
        &self,
        experiment_id: Uuid,
    ) -> Result<Vec<ExperimentEvent>, DesktopError> {
        Ok(self
            .domain_store()
            .list_experiment_events(experiment_id)
            .await?)
    }

    pub(crate) async fn create_experiment_event(
        &self,
        input: CreateExperimentEventInput,
    ) -> Result<ExperimentEvent, DesktopError> {
        let experiment = self
            .domain_store()
            .get_experiment(input.experiment_id)
            .await?;
        let now = Utc::now();
        let audit = self.audit("create_experiment_event").await?;
        Ok(create_experiment_event(
            self.domain_store(),
            CreateExperimentEventCommand {
                lab_id: self.local_lab_id(),
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                event_key: input.event_key,
                label: input.label,
                occurred_at: input.occurred_at.unwrap_or(now),
                details: input.details,
                now,
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_observation_definitions(
        &self,
        experiment_id: Uuid,
    ) -> Result<Vec<ObservationDefinition>, DesktopError> {
        Ok(self
            .domain_store()
            .list_observation_definitions(experiment_id)
            .await?)
    }

    pub(crate) async fn create_observation_definition(
        &self,
        input: CreateObservationDefinitionInput,
    ) -> Result<ObservationDefinition, DesktopError> {
        let experiment = self
            .domain_store()
            .get_experiment(input.experiment_id)
            .await?;
        let audit = self.audit("create_observation_definition").await?;
        Ok(create_observation_definition(
            self.domain_store(),
            CreateObservationDefinitionCommand {
                lab_id: self.local_lab_id(),
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                key: input.key,
                label: input.label,
                value_type: input.value_type,
                unit: input.unit,
                categories: input.categories,
                policy: input.policy,
                now: Utc::now(),
            },
            &audit,
        )
        .await?)
    }

    pub(crate) async fn list_observations(
        &self,
        experiment_id: Uuid,
        experiment_event_id: Option<Uuid>,
        subject_type: Option<ObservationSubjectType>,
        subject_id: Option<Uuid>,
    ) -> Result<Vec<Observation>, DesktopError> {
        Ok(self
            .domain_store()
            .list_observations(&ObservationFilter {
                experiment_id,
                experiment_event_id,
                subject_type,
                subject_id,
            })
            .await?)
    }

    pub(crate) async fn record_observation(
        &self,
        input: RecordObservationInput,
    ) -> Result<RecordedObservationView, DesktopError> {
        let experiment = self
            .domain_store()
            .get_experiment(input.experiment_id)
            .await?;
        let now = Utc::now();
        let audit = self.audit("record_observation").await?;
        let recorded = record_observation(
            self.domain_store(),
            RecordObservationCommand {
                lab_id: self.local_lab_id(),
                project_id: experiment.project_id,
                experiment_id: experiment.id,
                experiment_event_id: input.experiment_event_id,
                definition_id: input.definition_id,
                subject_type: input.subject_type,
                subject_id: input.subject_id,
                context: input.context,
                value: input.value,
                recorded_at: input.recorded_at.unwrap_or(now),
                recorded_by: Some(self.local_user_id()),
                notes: input.notes,
                now,
            },
            &audit,
        )
        .await?;
        Ok(RecordedObservationView {
            observation: recorded.observation,
            value: recorded.value,
        })
    }

    pub(crate) async fn list_observation_values(
        &self,
        observation_id: Uuid,
    ) -> Result<Vec<ObservationValueRecord>, DesktopError> {
        Ok(self
            .domain_store()
            .list_observation_values(observation_id)
            .await?)
    }

    pub(crate) async fn revise_observation(
        &self,
        input: ReviseObservationInput,
    ) -> Result<RecordedObservationView, DesktopError> {
        let now = Utc::now();
        let audit = self.audit("revise_observation").await?;
        let revised = revise_observation_value(
            self.domain_store(),
            ReviseObservationValueCommand {
                observation_id: input.observation_id,
                expected_revision: input.expected_revision,
                value: input.value,
                recorded_at: input.recorded_at.unwrap_or(now),
                recorded_by: Some(self.local_user_id()),
                notes: input.notes,
                now,
            },
            &audit,
        )
        .await?;
        Ok(RecordedObservationView {
            observation: revised.observation,
            value: revised.value,
        })
    }
}
