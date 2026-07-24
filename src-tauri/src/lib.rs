use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

mod ai;
mod ai_data_tools;
mod ai_images;
mod ai_source_resolver;
mod ai_sources;
mod animal_details;
mod application;
mod data;
mod model_profiles;
mod research_extensions;
mod settings;

use ai::{
    DesktopAiError, DesktopAiState, DesktopAutonomyInput, DesktopConversationStartInput,
    DesktopConversationUpdateInput, DesktopDraftDecisionInput, parse_uuid,
};
use ai_images::{
    ApproveAiExtractionInput, ArchivePrivateAiImageInput, CreateAiExtractionInput,
    PrivateImageContent, PrivateImageView, RejectAiExtractionInput, UploadPrivateAiImageInput,
};
use ai_sources::{AiSourceView, ArchiveAiSourceInput, ListAiSourcesInput, UploadAiSourceInput};
use animal_details::{
    AlleleView, AnimalDetailView, CreateAlleleInput, CreateAnimalSampleInput, CreateGeneLocusInput,
    CreateGenotypeInput, CreatePedigreeInput, GeneLocusView, GenotypeView, PedigreeRelationView,
    SampleView,
};
use application::{
    AnimalView, CageView, CohortView, CreateAnimalInput, CreateCageInput, CreateCohortInput,
    CreateExperimentInput, CreateProcedureInput, CreateProjectInput, CreateTemplateInput,
    DataJobView, DesktopError, DesktopState, EnrollAnimalInput, ExperimentView,
    LifecycleTransitionInput, MoveAnimalsInput, ParticipationView, ProcedureView, ProjectView,
    SaveWorkspaceSettingsInput, TemplateView, WorkspaceSettingsView,
};
use data::{
    AnimalImportTemplateInput, AnimalImportTemplateView, AttachmentDownloadView,
    AttachmentScopeInput, AttachmentView, CancelDataImportInput, ConfirmDataImportInput,
    CreateDataExportInput, CreateDataSnapshotInput, DataArtifactView, DeleteAttachmentInput,
    DesktopDataError, DesktopDataState, ImportReceiptView, PreviewDataImportInput,
    RemapDataImportInput, UploadAttachmentInput,
};
use model_profiles::{
    AiModelDefaultsView, AiModelProfileView, AiModelValidationResult, SaveAiModelDefaultsInput,
    SaveAiModelProfileInput, ValidateAiModelProfileInput,
};
use muriarc_ai::{
    AiAutonomyView, AssistantConversationDetail, AssistantConversationStartResponse,
    AssistantConversationSummary, AssistantTurnRequest, AssistantTurnResponse,
    DraftDecisionResponse, DraftStatus, WriteDraftSummary,
};
use muriarc_core::AiConversationArchiveFilter;
use research_extensions::{
    BreedingPredictionInput, CorrectGenotypingRecordInput, CorrectGenotypingRecordView,
    CreateBreedingLineInput, CreateBreedingPairInput,
    CreateColonyInput as CreateResearchColonyInput, CreateExperimentEventInput,
    CreateGenotypeDefinitionInput, CreateGenotypingRecordInput, CreateLitterInput,
    CreateMatingEventInput, CreateObservationDefinitionInput, CreatedLitterView,
    GeneticsArchiveInput, RecordObservationInput, RecordedObservationView,
    RegisterAnimalDraftInput, RegisteredAnimalDraftView, RetireBreedingPairInput,
    ReviseObservationInput, VoidGenotypingRecordInput,
};
use settings::{AiSettingsView, SaveAiSettingsInput, SettingsService};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppContext {
    product_name: &'static str,
    mode: &'static str,
    api_version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    code: &'static str,
    message: String,
}

impl From<DesktopError> for CommandError {
    fn from(error: DesktopError) -> Self {
        let code = error.code();
        let message = if code == "storage_error" {
            "本地数据或安全存储操作失败，请重试或查看诊断日志".to_owned()
        } else {
            error.to_string()
        };
        Self { code, message }
    }
}

impl From<DesktopAiError> for CommandError {
    fn from(error: DesktopAiError) -> Self {
        Self {
            code: error.code(),
            message: error.safe_message(),
        }
    }
}

impl From<DesktopDataError> for CommandError {
    fn from(error: DesktopDataError) -> Self {
        Self {
            code: error.code(),
            message: error.safe_message(),
        }
    }
}

type CommandResult<T> = Result<T, CommandError>;

#[tauri::command]
fn app_context() -> AppContext {
    AppContext {
        product_name: env!("MURIARC_PRODUCT_NAME"),
        mode: "local",
        api_version: "v1",
    }
}

#[tauri::command]
async fn list_cages(state: tauri::State<'_, DesktopState>) -> CommandResult<Vec<CageView>> {
    state.list_cages().await.map_err(Into::into)
}

#[tauri::command]
async fn create_cage(
    state: tauri::State<'_, DesktopState>,
    input: CreateCageInput,
) -> CommandResult<CageView> {
    state.create_cage(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_animal(
    state: tauri::State<'_, DesktopState>,
    input: CreateAnimalInput,
) -> CommandResult<AnimalView> {
    state.create_animal(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_animals(state: tauri::State<'_, DesktopState>) -> CommandResult<Vec<AnimalView>> {
    state.list_animals().await.map_err(Into::into)
}

#[tauri::command]
async fn get_animal(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> CommandResult<Option<AnimalView>> {
    state.get_animal(&id).await.map_err(Into::into)
}

#[tauri::command]
async fn get_animal_detail(
    state: tauri::State<'_, DesktopState>,
    animal_id: String,
    project_id: Option<String>,
) -> CommandResult<AnimalDetailView> {
    state
        .get_animal_detail(&animal_id, project_id.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_animal_sample(
    state: tauri::State<'_, DesktopState>,
    input: CreateAnimalSampleInput,
) -> CommandResult<SampleView> {
    state.create_animal_sample(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_pedigree_relation(
    state: tauri::State<'_, DesktopState>,
    input: CreatePedigreeInput,
) -> CommandResult<PedigreeRelationView> {
    state
        .create_pedigree_relation(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_gene_loci(
    state: tauri::State<'_, DesktopState>,
    project_id: Option<String>,
    include_archived: Option<bool>,
) -> CommandResult<Vec<GeneLocusView>> {
    state
        .list_gene_loci(project_id.as_deref(), include_archived.unwrap_or(false))
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn gene_locus_references(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
) -> CommandResult<muriarc_core::GeneticsReferenceCounts> {
    state.gene_locus_references(id).await.map_err(Into::into)
}

#[tauri::command]
async fn archive_gene_locus(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<GeneLocusView> {
    state.archive_gene_locus(input).await.map_err(Into::into)
}

#[tauri::command]
async fn restore_gene_locus(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<GeneLocusView> {
    state.restore_gene_locus(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_gene_locus(
    state: tauri::State<'_, DesktopState>,
    input: CreateGeneLocusInput,
) -> CommandResult<GeneLocusView> {
    state.create_gene_locus(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_alleles(
    state: tauri::State<'_, DesktopState>,
    locus_id: String,
    project_id: Option<String>,
    include_archived: Option<bool>,
) -> CommandResult<Vec<AlleleView>> {
    state
        .list_alleles(
            &locus_id,
            project_id.as_deref(),
            include_archived.unwrap_or(false),
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn allele_references(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
) -> CommandResult<muriarc_core::GeneticsReferenceCounts> {
    state.allele_references(id).await.map_err(Into::into)
}

#[tauri::command]
async fn archive_allele(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<AlleleView> {
    state.archive_allele(input).await.map_err(Into::into)
}

#[tauri::command]
async fn restore_allele(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<AlleleView> {
    state.restore_allele(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_allele(
    state: tauri::State<'_, DesktopState>,
    input: CreateAlleleInput,
) -> CommandResult<AlleleView> {
    state.create_allele(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_genotypes(
    state: tauri::State<'_, DesktopState>,
    animal_id: String,
    project_id: Option<String>,
) -> CommandResult<Vec<GenotypeView>> {
    state
        .list_genotypes(&animal_id, project_id.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_genotype(
    state: tauri::State<'_, DesktopState>,
    input: CreateGenotypeInput,
) -> CommandResult<GenotypeView> {
    state.create_genotype(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_genotype_definitions(
    state: tauri::State<'_, DesktopState>,
    include_archived: Option<bool>,
) -> CommandResult<Vec<muriarc_core::GenotypeDefinition>> {
    state
        .list_genotype_definitions(include_archived.unwrap_or(false))
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn genotype_definition_references(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
) -> CommandResult<muriarc_core::GeneticsReferenceCounts> {
    state
        .genotype_definition_references(id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn archive_genotype_definition(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<muriarc_core::GenotypeDefinition> {
    state
        .archive_genotype_definition(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn restore_genotype_definition(
    state: tauri::State<'_, DesktopState>,
    input: GeneticsArchiveInput,
) -> CommandResult<muriarc_core::GenotypeDefinition> {
    state
        .restore_genotype_definition(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_genotype_definition(
    state: tauri::State<'_, DesktopState>,
    input: CreateGenotypeDefinitionInput,
) -> CommandResult<muriarc_core::GenotypeDefinition> {
    state
        .create_genotype_definition(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_genotyping_records(
    state: tauri::State<'_, DesktopState>,
    animal_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::GenotypingRecord>> {
    state
        .list_genotyping_records(animal_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_genotyping_record(
    state: tauri::State<'_, DesktopState>,
    input: CreateGenotypingRecordInput,
) -> CommandResult<muriarc_core::GenotypingRecord> {
    state
        .create_genotyping_record(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn void_genotyping_record(
    state: tauri::State<'_, DesktopState>,
    input: VoidGenotypingRecordInput,
) -> CommandResult<muriarc_core::GenotypingRecord> {
    state
        .void_genotyping_record(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn correct_genotyping_record(
    state: tauri::State<'_, DesktopState>,
    input: CorrectGenotypingRecordInput,
) -> CommandResult<CorrectGenotypingRecordView> {
    state
        .correct_genotyping_record(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_breeding_lines(
    state: tauri::State<'_, DesktopState>,
) -> CommandResult<Vec<muriarc_core::BreedingLine>> {
    state.list_breeding_lines().await.map_err(Into::into)
}

#[tauri::command]
async fn create_breeding_line(
    state: tauri::State<'_, DesktopState>,
    input: CreateBreedingLineInput,
) -> CommandResult<muriarc_core::BreedingLine> {
    state.create_breeding_line(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_colonies_v2(
    state: tauri::State<'_, DesktopState>,
    breeding_line_id: Option<uuid::Uuid>,
) -> CommandResult<Vec<muriarc_core::Colony>> {
    state
        .list_colonies(breeding_line_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_colony_v2(
    state: tauri::State<'_, DesktopState>,
    input: CreateResearchColonyInput,
) -> CommandResult<muriarc_core::Colony> {
    state.create_colony(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_breeding_pairs(
    state: tauri::State<'_, DesktopState>,
    colony_id: Option<uuid::Uuid>,
) -> CommandResult<Vec<muriarc_core::BreedingPair>> {
    state
        .list_breeding_pairs(colony_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_breeding_pair(
    state: tauri::State<'_, DesktopState>,
    input: CreateBreedingPairInput,
) -> CommandResult<muriarc_core::BreedingPair> {
    state.create_breeding_pair(input).await.map_err(Into::into)
}

#[tauri::command]
async fn retire_breeding_pair(
    state: tauri::State<'_, DesktopState>,
    input: RetireBreedingPairInput,
) -> CommandResult<muriarc_core::BreedingPair> {
    state.retire_breeding_pair(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_mating_events(
    state: tauri::State<'_, DesktopState>,
    breeding_pair_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::MatingEvent>> {
    state
        .list_mating_events(breeding_pair_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_mating_event(
    state: tauri::State<'_, DesktopState>,
    input: CreateMatingEventInput,
) -> CommandResult<muriarc_core::MatingEvent> {
    state.create_mating_event(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_litters(
    state: tauri::State<'_, DesktopState>,
    breeding_pair_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::Litter>> {
    state
        .list_litters(breeding_pair_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_litter(
    state: tauri::State<'_, DesktopState>,
    input: CreateLitterInput,
) -> CommandResult<CreatedLitterView> {
    state.create_litter(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_animal_drafts(
    state: tauri::State<'_, DesktopState>,
    litter_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::AnimalDraft>> {
    state
        .list_animal_drafts(litter_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn register_animal_draft(
    state: tauri::State<'_, DesktopState>,
    input: RegisterAnimalDraftInput,
) -> CommandResult<RegisteredAnimalDraftView> {
    state.register_animal_draft(input).await.map_err(Into::into)
}

#[tauri::command]
async fn predict_breeding(
    state: tauri::State<'_, DesktopState>,
    input: BreedingPredictionInput,
) -> CommandResult<Vec<muriarc_core::LocusPrediction>> {
    state.breeding_prediction(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_experiment_events(
    state: tauri::State<'_, DesktopState>,
    experiment_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::ExperimentEvent>> {
    state
        .list_experiment_events(experiment_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_experiment_event(
    state: tauri::State<'_, DesktopState>,
    input: CreateExperimentEventInput,
) -> CommandResult<muriarc_core::ExperimentEvent> {
    state
        .create_experiment_event(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_observation_definitions(
    state: tauri::State<'_, DesktopState>,
    experiment_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::ObservationDefinition>> {
    state
        .list_observation_definitions(experiment_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_observation_definition(
    state: tauri::State<'_, DesktopState>,
    input: CreateObservationDefinitionInput,
) -> CommandResult<muriarc_core::ObservationDefinition> {
    state
        .create_observation_definition(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_observations(
    state: tauri::State<'_, DesktopState>,
    experiment_id: uuid::Uuid,
    experiment_event_id: Option<uuid::Uuid>,
    subject_type: Option<muriarc_core::ObservationSubjectType>,
    subject_id: Option<uuid::Uuid>,
) -> CommandResult<Vec<muriarc_core::Observation>> {
    state
        .list_observations(experiment_id, experiment_event_id, subject_type, subject_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn record_observation(
    state: tauri::State<'_, DesktopState>,
    input: RecordObservationInput,
) -> CommandResult<RecordedObservationView> {
    state.record_observation(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_observation_values(
    state: tauri::State<'_, DesktopState>,
    observation_id: uuid::Uuid,
) -> CommandResult<Vec<muriarc_core::ObservationValueRecord>> {
    state
        .list_observation_values(observation_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn revise_observation(
    state: tauri::State<'_, DesktopState>,
    input: ReviseObservationInput,
) -> CommandResult<RecordedObservationView> {
    state.revise_observation(input).await.map_err(Into::into)
}

#[tauri::command]
async fn move_animals(
    state: tauri::State<'_, DesktopState>,
    input: MoveAnimalsInput,
) -> CommandResult<()> {
    state.move_animals(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_project(
    state: tauri::State<'_, DesktopState>,
    input: CreateProjectInput,
) -> CommandResult<ProjectView> {
    state.create_project(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_projects(state: tauri::State<'_, DesktopState>) -> CommandResult<Vec<ProjectView>> {
    state.list_projects().await.map_err(Into::into)
}

#[tauri::command]
async fn list_published_templates(
    state: tauri::State<'_, DesktopState>,
) -> CommandResult<Vec<TemplateView>> {
    state.list_published_templates().await.map_err(Into::into)
}

#[tauri::command]
async fn create_published_template(
    state: tauri::State<'_, DesktopState>,
    input: CreateTemplateInput,
) -> CommandResult<TemplateView> {
    state
        .create_published_template(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_experiment(
    state: tauri::State<'_, DesktopState>,
    input: CreateExperimentInput,
) -> CommandResult<ExperimentView> {
    state.create_experiment(input).await.map_err(Into::into)
}

#[tauri::command]
async fn complete_experiment(
    state: tauri::State<'_, DesktopState>,
    input: LifecycleTransitionInput,
) -> CommandResult<ExperimentView> {
    state.complete_experiment(input).await.map_err(Into::into)
}

#[tauri::command]
async fn cancel_experiment(
    state: tauri::State<'_, DesktopState>,
    input: LifecycleTransitionInput,
) -> CommandResult<ExperimentView> {
    state.cancel_experiment(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_cohorts(
    state: tauri::State<'_, DesktopState>,
    experiment_id: String,
) -> CommandResult<Vec<CohortView>> {
    state.list_cohorts(&experiment_id).await.map_err(Into::into)
}

#[tauri::command]
async fn create_cohort(
    state: tauri::State<'_, DesktopState>,
    input: CreateCohortInput,
) -> CommandResult<CohortView> {
    state.create_cohort(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_participations(
    state: tauri::State<'_, DesktopState>,
    project_id: String,
    experiment_id: String,
) -> CommandResult<Vec<ParticipationView>> {
    state
        .list_participations(&project_id, &experiment_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn enroll_animal(
    state: tauri::State<'_, DesktopState>,
    input: EnrollAnimalInput,
) -> CommandResult<ParticipationView> {
    state.enroll_animal(input).await.map_err(Into::into)
}

#[tauri::command]
async fn complete_participation(
    state: tauri::State<'_, DesktopState>,
    input: LifecycleTransitionInput,
) -> CommandResult<ParticipationView> {
    state
        .complete_participation(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn withdraw_participation(
    state: tauri::State<'_, DesktopState>,
    input: LifecycleTransitionInput,
) -> CommandResult<ParticipationView> {
    state
        .withdraw_participation(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_procedures(
    state: tauri::State<'_, DesktopState>,
    experiment_id: String,
) -> CommandResult<Vec<ProcedureView>> {
    state
        .list_procedures(&experiment_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_procedure(
    state: tauri::State<'_, DesktopState>,
    input: CreateProcedureInput,
) -> CommandResult<ProcedureView> {
    state.create_procedure(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_experiments(
    state: tauri::State<'_, DesktopState>,
) -> CommandResult<Vec<ExperimentView>> {
    state.list_experiments().await.map_err(Into::into)
}

#[tauri::command]
async fn list_data_jobs(state: tauri::State<'_, DesktopState>) -> CommandResult<Vec<DataJobView>> {
    state.list_data_jobs().await.map_err(Into::into)
}

#[tauri::command]
async fn get_workspace_settings(
    state: tauri::State<'_, DesktopState>,
) -> CommandResult<WorkspaceSettingsView> {
    state.get_workspace_settings().await.map_err(Into::into)
}

#[tauri::command]
async fn save_workspace_settings(
    state: tauri::State<'_, DesktopState>,
    input: SaveWorkspaceSettingsInput,
) -> CommandResult<WorkspaceSettingsView> {
    state
        .save_workspace_settings(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
fn get_ai_settings(state: tauri::State<'_, DesktopState>) -> CommandResult<AiSettingsView> {
    state.get_ai_settings().map_err(Into::into)
}

#[tauri::command]
async fn save_ai_settings(
    state: tauri::State<'_, DesktopState>,
    input: SaveAiSettingsInput,
) -> CommandResult<AiSettingsView> {
    state.save_ai_settings(input).await.map_err(Into::into)
}

#[tauri::command]
async fn clear_ai_api_key(state: tauri::State<'_, DesktopState>) -> CommandResult<AiSettingsView> {
    state.clear_ai_api_key().await.map_err(Into::into)
}

#[tauri::command]
async fn list_ai_model_profiles(
    state: tauri::State<'_, DesktopState>,
    include_archived: Option<bool>,
) -> CommandResult<Vec<AiModelProfileView>> {
    state
        .list_ai_model_profiles(include_archived.unwrap_or(false))
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn get_ai_model_profile(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
) -> CommandResult<AiModelProfileView> {
    state.get_ai_model_profile(id).await.map_err(Into::into)
}

#[tauri::command]
async fn create_ai_model_profile(
    state: tauri::State<'_, DesktopState>,
    input: SaveAiModelProfileInput,
) -> CommandResult<AiModelProfileView> {
    state
        .create_ai_model_profile(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn update_ai_model_profile(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
    input: SaveAiModelProfileInput,
) -> CommandResult<AiModelProfileView> {
    state
        .update_ai_model_profile(id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn validate_ai_model_profile(
    state: tauri::State<'_, DesktopState>,
    input: ValidateAiModelProfileInput,
) -> CommandResult<AiModelValidationResult> {
    state
        .validate_ai_model_profile(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn clear_ai_model_profile_key(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
) -> CommandResult<AiModelProfileView> {
    state
        .clear_ai_model_profile_key(id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn archive_ai_model_profile(
    state: tauri::State<'_, DesktopState>,
    id: uuid::Uuid,
    expected_revision: i64,
) -> CommandResult<AiModelProfileView> {
    state
        .archive_ai_model_profile(id, expected_revision)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn get_ai_model_defaults(
    state: tauri::State<'_, DesktopState>,
) -> CommandResult<AiModelDefaultsView> {
    state.get_ai_model_defaults().await.map_err(Into::into)
}

#[tauri::command]
async fn save_ai_model_defaults(
    state: tauri::State<'_, DesktopState>,
    input: SaveAiModelDefaultsInput,
) -> CommandResult<AiModelDefaultsView> {
    state
        .save_ai_model_defaults(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn declare_ai_full_startup(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopAiState>,
) -> CommandResult<()> {
    let state = state.inner().clone();
    match tauri::async_runtime::spawn_blocking(move || {
        state.confirm_full_startup(|| {
            app.dialog()
                .message(
                    "Full 仅扩大本次桌面启动内、当前会话的普通操作委托；不会扩大项目权限，也不会绕过科研签署、高风险确认或草稿审批。",
                )
                .title("确认启用 MuriArc AI Full")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "确认启用".to_owned(),
                    "取消".to_owned(),
                ))
                .blocking_show()
        })
    })
    .await
    {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(DesktopAiError::AutonomyDeclarationRequired.into()),
    }
}

#[tauri::command]
async fn start_ai_conversation(
    state: tauri::State<'_, DesktopAiState>,
    input: DesktopConversationStartInput,
) -> CommandResult<AssistantConversationStartResponse> {
    state.start_conversation(input).await.map_err(Into::into)
}

#[tauri::command]
async fn ai_turn(
    state: tauri::State<'_, DesktopAiState>,
    input: AssistantTurnRequest,
) -> CommandResult<AssistantTurnResponse> {
    state.turn(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_private_ai_images(
    state: tauri::State<'_, DesktopAiState>,
    conversation_id: Option<uuid::Uuid>,
    project_id: Option<uuid::Uuid>,
) -> CommandResult<Vec<PrivateImageView>> {
    state
        .list_private_images(conversation_id, project_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn upload_private_ai_image(
    state: tauri::State<'_, DesktopAiState>,
    input: UploadPrivateAiImageInput,
) -> CommandResult<PrivateImageView> {
    state.upload_private_image(input).await.map_err(Into::into)
}

#[tauri::command]
async fn read_private_ai_image(
    state: tauri::State<'_, DesktopAiState>,
    id: uuid::Uuid,
) -> CommandResult<PrivateImageContent> {
    state.read_private_image(id).await.map_err(Into::into)
}

#[tauri::command]
async fn archive_private_ai_image(
    state: tauri::State<'_, DesktopAiState>,
    id: uuid::Uuid,
    input: ArchivePrivateAiImageInput,
) -> CommandResult<PrivateImageView> {
    state
        .archive_private_image(id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_ai_extractions(
    state: tauri::State<'_, DesktopAiState>,
    project_id: Option<uuid::Uuid>,
) -> CommandResult<Vec<muriarc_core::AiExtractionDraft>> {
    state
        .list_ai_extractions(project_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn create_ai_extraction(
    state: tauri::State<'_, DesktopAiState>,
    input: CreateAiExtractionInput,
) -> CommandResult<muriarc_core::AiExtractionDraft> {
    state.create_ai_extraction(input).await.map_err(Into::into)
}

#[tauri::command]
async fn approve_ai_extraction(
    state: tauri::State<'_, DesktopAiState>,
    id: uuid::Uuid,
    input: ApproveAiExtractionInput,
) -> CommandResult<muriarc_core::AppliedAiExtraction> {
    state
        .approve_ai_extraction(id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn reject_ai_extraction(
    state: tauri::State<'_, DesktopAiState>,
    id: uuid::Uuid,
    input: RejectAiExtractionInput,
) -> CommandResult<muriarc_core::AiExtractionDraft> {
    state
        .reject_ai_extraction(id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_ai_conversations(
    state: tauri::State<'_, DesktopAiState>,
    project_id: Option<String>,
    title_query: Option<String>,
    archive: Option<AiConversationArchiveFilter>,
    limit: Option<u32>,
) -> CommandResult<Vec<AssistantConversationSummary>> {
    let project_id = project_id.as_deref().map(parse_uuid).transpose()?;
    state
        .list_conversations(
            project_id,
            title_query,
            archive.unwrap_or_default(),
            limit.unwrap_or(50),
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn update_ai_conversation(
    state: tauri::State<'_, DesktopAiState>,
    conversation_id: String,
    input: DesktopConversationUpdateInput,
) -> CommandResult<AssistantConversationSummary> {
    state
        .update_conversation(parse_uuid(&conversation_id)?, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn get_ai_conversation(
    state: tauri::State<'_, DesktopAiState>,
    conversation_id: String,
    limit: Option<u32>,
) -> CommandResult<AssistantConversationDetail> {
    state
        .get_conversation(parse_uuid(&conversation_id)?, limit.unwrap_or(200))
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn upload_ai_source(
    state: tauri::State<'_, DesktopDataState>,
    input: UploadAiSourceInput,
) -> CommandResult<AiSourceView> {
    state.upload_ai_source(input).await.map_err(Into::into)
}

#[tauri::command]
async fn list_ai_sources(
    state: tauri::State<'_, DesktopDataState>,
    input: ListAiSourcesInput,
) -> CommandResult<Vec<AiSourceView>> {
    state.list_ai_sources(input).await.map_err(Into::into)
}

#[tauri::command]
async fn archive_ai_source(
    state: tauri::State<'_, DesktopDataState>,
    source_id: String,
    input: ArchiveAiSourceInput,
) -> CommandResult<AiSourceView> {
    state
        .archive_ai_source(&source_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn delete_ai_source(
    state: tauri::State<'_, DesktopDataState>,
    source_id: String,
) -> CommandResult<()> {
    state.delete_ai_source(&source_id).await.map_err(Into::into)
}

#[tauri::command]
async fn get_ai_autonomy(
    state: tauri::State<'_, DesktopAiState>,
    conversation_id: String,
) -> CommandResult<AiAutonomyView> {
    state
        .get_autonomy(parse_uuid(&conversation_id)?)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn set_ai_autonomy(
    state: tauri::State<'_, DesktopAiState>,
    conversation_id: String,
    input: DesktopAutonomyInput,
) -> CommandResult<AiAutonomyView> {
    state
        .set_autonomy(parse_uuid(&conversation_id)?, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn list_ai_drafts(
    state: tauri::State<'_, DesktopAiState>,
    project_id: Option<String>,
    status: Option<DraftStatus>,
) -> CommandResult<Vec<WriteDraftSummary>> {
    let project_id = project_id.as_deref().map(parse_uuid).transpose()?;
    state
        .list_drafts(project_id, status)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn get_ai_draft(
    state: tauri::State<'_, DesktopAiState>,
    draft_id: String,
) -> CommandResult<WriteDraftSummary> {
    state
        .get_draft(parse_uuid(&draft_id)?)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn decide_ai_draft(
    state: tauri::State<'_, DesktopAiState>,
    draft_id: String,
    input: DesktopDraftDecisionInput,
) -> CommandResult<DraftDecisionResponse> {
    state
        .decide_draft(parse_uuid(&draft_id)?, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
async fn preview_data_import(
    state: tauri::State<'_, DesktopDataState>,
    input: PreviewDataImportInput,
) -> CommandResult<muriarc_data::AnimalImportPreviewResponse> {
    state.preview_import(input).await.map_err(Into::into)
}

#[tauri::command]
async fn remap_data_import(
    state: tauri::State<'_, DesktopDataState>,
    input: RemapDataImportInput,
) -> CommandResult<muriarc_data::AnimalImportPreviewResponse> {
    state.remap_import(input).await.map_err(Into::into)
}

#[tauri::command]
async fn confirm_data_import(
    state: tauri::State<'_, DesktopDataState>,
    input: ConfirmDataImportInput,
) -> CommandResult<ImportReceiptView> {
    state.confirm_import(input).await.map_err(Into::into)
}

#[tauri::command]
async fn cancel_data_import(
    state: tauri::State<'_, DesktopDataState>,
    input: CancelDataImportInput,
) -> CommandResult<()> {
    state.cancel_import(input).await.map_err(Into::into)
}

#[tauri::command]
fn get_animal_import_schema(
    state: tauri::State<'_, DesktopDataState>,
) -> CommandResult<muriarc_importer::AnimalImportSchema> {
    Ok(state.animal_import_schema())
}

#[tauri::command]
fn get_animal_import_template(
    state: tauri::State<'_, DesktopDataState>,
    input: AnimalImportTemplateInput,
) -> CommandResult<AnimalImportTemplateView> {
    state.animal_import_template(input).map_err(Into::into)
}

#[tauri::command]
async fn create_data_export(
    state: tauri::State<'_, DesktopDataState>,
    input: CreateDataExportInput,
) -> CommandResult<DataArtifactView> {
    state.create_export(input).await.map_err(Into::into)
}

#[tauri::command]
async fn create_data_snapshot(
    state: tauri::State<'_, DesktopDataState>,
    input: CreateDataSnapshotInput,
) -> CommandResult<DataArtifactView> {
    state.create_snapshot(input).await.map_err(Into::into)
}

#[tauri::command]
async fn read_data_artifact(
    state: tauri::State<'_, DesktopDataState>,
    job_id: String,
) -> CommandResult<DataArtifactView> {
    state.read_artifact(&job_id).await.map_err(Into::into)
}

#[tauri::command]
async fn list_attachments(
    state: tauri::State<'_, DesktopDataState>,
    input: AttachmentScopeInput,
) -> CommandResult<Vec<AttachmentView>> {
    state.list_attachments(input).await.map_err(Into::into)
}

#[tauri::command]
async fn upload_attachment(
    state: tauri::State<'_, DesktopDataState>,
    input: UploadAttachmentInput,
) -> CommandResult<AttachmentView> {
    state.upload_attachment(input).await.map_err(Into::into)
}

#[tauri::command]
async fn download_attachment(
    state: tauri::State<'_, DesktopDataState>,
    id: String,
) -> CommandResult<AttachmentDownloadView> {
    state.download_attachment(&id).await.map_err(Into::into)
}

#[tauri::command]
async fn delete_attachment(
    state: tauri::State<'_, DesktopDataState>,
    input: DeleteAttachmentInput,
) -> CommandResult<AttachmentView> {
    state.delete_attachment(input).await.map_err(Into::into)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = app_data_dir.join("muriarc.sqlite3");
            let settings = SettingsService::for_app_data(&app_data_dir);
            let state = tauri::async_runtime::block_on(DesktopState::initialize_with_settings(
                &database_path,
                settings.clone(),
            ))?;
            let data_state = tauri::async_runtime::block_on(DesktopDataState::initialize(
                &database_path,
                &app_data_dir,
            ))?;
            let ai_state = tauri::async_runtime::block_on(DesktopAiState::initialize(
                data_state.clone(),
                settings,
            ))?;
            app.manage(state);
            app.manage(data_state);
            app.manage(ai_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_context,
            list_cages,
            create_cage,
            create_animal,
            list_animals,
            get_animal,
            get_animal_detail,
            create_animal_sample,
            create_pedigree_relation,
            list_gene_loci,
            gene_locus_references,
            archive_gene_locus,
            restore_gene_locus,
            create_gene_locus,
            list_alleles,
            allele_references,
            archive_allele,
            restore_allele,
            create_allele,
            list_genotypes,
            create_genotype,
            list_genotype_definitions,
            genotype_definition_references,
            archive_genotype_definition,
            restore_genotype_definition,
            create_genotype_definition,
            list_genotyping_records,
            create_genotyping_record,
            void_genotyping_record,
            correct_genotyping_record,
            list_breeding_lines,
            create_breeding_line,
            list_colonies_v2,
            create_colony_v2,
            list_breeding_pairs,
            create_breeding_pair,
            retire_breeding_pair,
            list_mating_events,
            create_mating_event,
            list_litters,
            create_litter,
            list_animal_drafts,
            register_animal_draft,
            predict_breeding,
            list_experiment_events,
            create_experiment_event,
            list_observation_definitions,
            create_observation_definition,
            list_observations,
            record_observation,
            list_observation_values,
            revise_observation,
            move_animals,
            create_project,
            list_projects,
            list_published_templates,
            create_published_template,
            create_experiment,
            complete_experiment,
            cancel_experiment,
            list_cohorts,
            create_cohort,
            list_participations,
            enroll_animal,
            complete_participation,
            withdraw_participation,
            list_procedures,
            create_procedure,
            list_experiments,
            list_data_jobs,
            get_workspace_settings,
            save_workspace_settings,
            get_ai_settings,
            save_ai_settings,
            clear_ai_api_key,
            list_ai_model_profiles,
            get_ai_model_profile,
            create_ai_model_profile,
            update_ai_model_profile,
            validate_ai_model_profile,
            clear_ai_model_profile_key,
            archive_ai_model_profile,
            get_ai_model_defaults,
            save_ai_model_defaults,
            declare_ai_full_startup,
            start_ai_conversation,
            ai_turn,
            list_private_ai_images,
            upload_private_ai_image,
            read_private_ai_image,
            archive_private_ai_image,
            list_ai_extractions,
            create_ai_extraction,
            approve_ai_extraction,
            reject_ai_extraction,
            list_ai_conversations,
            update_ai_conversation,
            get_ai_conversation,
            upload_ai_source,
            list_ai_sources,
            archive_ai_source,
            delete_ai_source,
            get_ai_autonomy,
            set_ai_autonomy,
            list_ai_drafts,
            get_ai_draft,
            decide_ai_draft,
            preview_data_import,
            remap_data_import,
            confirm_data_import,
            cancel_data_import,
            get_animal_import_schema,
            get_animal_import_template,
            create_data_export,
            create_data_snapshot,
            read_data_artifact,
            list_attachments,
            upload_attachment,
            download_attachment,
            delete_attachment,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run MuriArc desktop application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_context_uses_stable_product_identity() {
        let context = app_context();
        assert_eq!(context.product_name, env!("MURIARC_PRODUCT_NAME"));
        assert_eq!(context.mode, "local");
        assert_eq!(context.api_version, "v1");
    }

    #[test]
    fn storage_errors_are_not_exposed_to_the_frontend() {
        let error = CommandError::from(DesktopError::Store(muriarc_core::StoreError::Database(
            "database path with private details".to_owned(),
        )));
        assert_eq!(error.code, "storage_error");
        assert!(!error.message.contains("private details"));
    }
}
