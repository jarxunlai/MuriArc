use axum::{Json, Router, extract::State, routing::get};
use muriarc_application::{
    AnimalAuditEntry, AnimalDetailQuery, AnimalPedigreeDirection, AnimalPedigreeEntry,
    MAX_ANIMAL_DETAIL_PROJECTS, animal_project_refs, load_animal_detail,
};
use muriarc_core::{
    Animal, AnimalEvent, AnimalFilter, AnimalOverview, AnimalProjectRef, AnimalStatus, Attachment,
    AuditAction, Experiment, Measurement, ParentType, Participation, ParticipationStatus,
    Permission, ProvenanceSource, RecordStatus, Sample, Sex, WriteSource,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, collection, item, scope,
    store,
    validation::{collection_limit, validation},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/animal-overviews", get(list_overviews))
        .route("/animals/{id}/detail", get(get_detail))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverviewQuery {
    project_id: Option<Uuid>,
    cage_id: Option<Uuid>,
    status: Option<AnimalStatus>,
    q: Option<String>,
    offset: Option<u32>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct AnimalOverviewView {
    animal: Animal,
    genotype: String,
    projects: Vec<AnimalProjectRef>,
    latest_weight: Option<muriarc_core::LatestAnimalWeight>,
}

impl From<AnimalOverview> for AnimalOverviewView {
    fn from(overview: AnimalOverview) -> Self {
        Self {
            animal: overview.animal,
            genotype: if overview.genotype_labels.is_empty() {
                "待确认".to_owned()
            } else {
                overview.genotype_labels.join(" · ")
            },
            projects: overview.projects,
            latest_weight: overview.latest_weight,
        }
    }
}

async fn list_overviews(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<OverviewQuery>,
) -> Result<Json<CollectionResponse<AnimalOverviewView>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let limit = collection_limit(query.limit, &metadata)?;
    let offset = query.offset.unwrap_or(0);
    if offset > 100_000 {
        return Err(validation("offset must not exceed 100000", &metadata));
    }
    let query_text = query
        .q
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if query_text.as_ref().is_some_and(|value| value.len() > 256) {
        return Err(validation("query must not exceed 256 bytes", &metadata));
    }

    let rows = store(
        state.store.list_animal_overviews(
            &AnimalFilter {
                lab_id: principal.lab_id,
                project_id: query.project_id,
                cage_id: query.cage_id,
                status: query.status,
                query: query_text,
            },
            offset,
            limit as u32,
        ),
        &metadata,
    )
    .await?
    .into_iter()
    .map(Into::into)
    .collect();
    Ok(collection(rows, &metadata))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailQuery {
    project_id: Option<Uuid>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct AnimalExperimentView {
    project: AnimalProjectRef,
    experiment: ExperimentSummary,
    participation: ParticipationSummary,
    cohort: Option<CohortSummary>,
}

#[derive(Debug, Serialize)]
struct ExperimentSummary {
    id: Uuid,
    name: String,
    status: muriarc_core::ExperimentStatus,
    starts_at: Option<chrono::DateTime<chrono::Utc>>,
    ends_at: Option<chrono::DateTime<chrono::Utc>>,
    revision: i64,
}

impl From<Experiment> for ExperimentSummary {
    fn from(value: Experiment) -> Self {
        Self {
            id: value.id,
            name: value.name,
            status: value.status,
            starts_at: value.starts_at,
            ends_at: value.ends_at,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Serialize)]
struct ParticipationSummary {
    id: Uuid,
    status: ParticipationStatus,
    enrolled_at: chrono::DateTime<chrono::Utc>,
    exited_at: Option<chrono::DateTime<chrono::Utc>>,
    revision: i64,
}

impl From<Participation> for ParticipationSummary {
    fn from(value: Participation) -> Self {
        Self {
            id: value.id,
            status: value.status,
            enrolled_at: value.enrolled_at,
            exited_at: value.exited_at,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Serialize)]
struct CohortSummary {
    id: Uuid,
    name: String,
}

#[derive(Debug, Serialize)]
struct MeasurementView {
    id: Uuid,
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    key: String,
    label: String,
    value: muriarc_core::MeasurementValue,
    unit: Option<String>,
    measured_at: chrono::DateTime<chrono::Utc>,
    status: RecordStatus,
    revision: i64,
}

impl From<Measurement> for MeasurementView {
    fn from(value: Measurement) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            experiment_id: value.experiment_id,
            key: value.key,
            label: value.label,
            value: value.value,
            unit: value.unit,
            measured_at: value.measured_at,
            status: value.status,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Serialize)]
struct SampleView {
    id: Uuid,
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    sample_type: String,
    quantity: Option<f64>,
    unit: Option<String>,
    location: Option<String>,
    collected_at: chrono::DateTime<chrono::Utc>,
    revision: i64,
}

impl From<Sample> for SampleView {
    fn from(value: Sample) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            experiment_id: value.experiment_id,
            sample_type: value.sample_type,
            quantity: value.quantity,
            unit: value.unit,
            location: value.location,
            collected_at: value.collected_at,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PedigreeDirection {
    Parent,
    Offspring,
}

#[derive(Debug, Serialize)]
struct PedigreeRelationView {
    id: Uuid,
    direction: PedigreeDirection,
    parent_type: ParentType,
    related_animal: RelatedAnimalView,
    revision: i64,
}

#[derive(Debug, Serialize)]
struct RelatedAnimalView {
    id: Uuid,
    display_id: String,
    sex: Sex,
    strain: Option<String>,
    current_status: AnimalStatus,
}

#[derive(Debug, Serialize)]
struct AttachmentMetadataView {
    id: Uuid,
    project_id: Option<Uuid>,
    entity_type: String,
    entity_id: Uuid,
    file_name: String,
    media_type: Option<String>,
    size_bytes: i64,
    sha256: String,
    version: i32,
    content_href: String,
    created_at: chrono::DateTime<chrono::Utc>,
    revision: i64,
}

#[derive(Debug, Serialize)]
struct AuditSummaryView {
    id: Uuid,
    action: AuditAction,
    actor: String,
    source: WriteSource,
    reason: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
    revision: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ProvenanceSummaryView {
    id: Uuid,
    source: ProvenanceSource,
    actor: Option<String>,
    recorded_at: chrono::DateTime<chrono::Utc>,
    request_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct AnimalDetailView {
    events: Vec<AnimalEvent>,
    experiments: Vec<AnimalExperimentView>,
    measurements: Vec<MeasurementView>,
    pedigree: Vec<PedigreeRelationView>,
    samples: Vec<SampleView>,
    attachments: Vec<AttachmentMetadataView>,
    audit_visible: bool,
    audits: Vec<AuditSummaryView>,
    provenance: Vec<ProvenanceSummaryView>,
}

async fn get_detail(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(animal_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<DetailQuery>,
) -> Result<Json<ItemResponse<AnimalDetailView>>, ApiError> {
    let animal = scope::animal_with_permission(
        &state,
        &principal,
        &metadata,
        animal_id,
        query.project_id,
        Permission::ReadAnimal,
    )
    .await?;
    let limit = collection_limit(query.limit, &metadata)?;
    let projects =
        detail_projects(&state, &principal, &metadata, &animal, query.project_id).await?;
    // Reject oversized scopes before issuing one permission check per project.
    if projects.len() > MAX_ANIMAL_DETAIL_PROJECTS {
        return Err(validation(
            format!(
                "animal is linked to more than {MAX_ANIMAL_DETAIL_PROJECTS} projects; select a project context"
            ),
            &metadata,
        ));
    }
    for project in &projects {
        scope::project_with_permission(
            &state,
            &principal,
            &metadata,
            project.id,
            Permission::ReadExperiment,
        )
        .await?;
    }
    let readable = |permission| {
        projects
            .iter()
            .filter(|project| principal.can(permission, Some(project.id)))
            .map(|project| project.id)
            .collect()
    };
    let audit_visible = principal.can(Permission::ReadAudit, query.project_id);
    let detail_query = AnimalDetailQuery {
        lab_id: principal.lab_id,
        animal_id,
        project_id: query.project_id,
        measurement_project_ids: readable(Permission::ReadMeasurement),
        sample_project_ids: readable(Permission::ReadSample),
        projects,
        include_attachments: principal.can(Permission::ReadAttachment, query.project_id),
        include_audit: audit_visible,
        limit,
    };
    let detail = application(
        load_animal_detail(state.store.as_ref(), &detail_query),
        &metadata,
    )
    .await?;

    Ok(item(
        AnimalDetailView {
            events: detail.events,
            experiments: detail
                .experiments
                .into_iter()
                .map(|entry| AnimalExperimentView {
                    project: entry.project,
                    experiment: entry.experiment.into(),
                    participation: entry.participation.into(),
                    cohort: entry.cohort.map(|cohort| CohortSummary {
                        id: cohort.id,
                        name: cohort.name,
                    }),
                })
                .collect(),
            measurements: detail.measurements.into_iter().map(Into::into).collect(),
            pedigree: detail.pedigree.into_iter().map(pedigree_view).collect(),
            samples: detail.samples.into_iter().map(Into::into).collect(),
            attachments: detail
                .attachments
                .into_iter()
                .map(attachment_view)
                .collect(),
            audit_visible,
            audits: detail.audits.into_iter().map(audit_view).collect(),
            provenance: detail
                .provenance
                .into_iter()
                .map(|entry| ProvenanceSummaryView {
                    id: entry.record.id,
                    source: entry.record.source,
                    actor: entry.actor_name,
                    recorded_at: entry.record.recorded_at,
                    request_id: entry.record.request_id,
                })
                .collect(),
        },
        &metadata,
    ))
}

async fn detail_projects(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    animal: &Animal,
    project_id: Option<Uuid>,
) -> Result<Vec<AnimalProjectRef>, ApiError> {
    if let Some(project_id) = project_id {
        let project = scope::project_with_permission(
            state,
            principal,
            metadata,
            project_id,
            Permission::ReadAnimal,
        )
        .await?;
        return Ok(vec![AnimalProjectRef {
            id: project.id,
            name: project.name,
        }]);
    }
    application(
        animal_project_refs(state.store.as_ref(), principal.lab_id, animal),
        metadata,
    )
    .await
}

fn pedigree_view(entry: AnimalPedigreeEntry) -> PedigreeRelationView {
    let animal = entry.related_animal;
    PedigreeRelationView {
        id: entry.relation.id,
        direction: match entry.direction {
            AnimalPedigreeDirection::Parent => PedigreeDirection::Parent,
            AnimalPedigreeDirection::Offspring => PedigreeDirection::Offspring,
        },
        parent_type: entry.relation.parent_type,
        related_animal: RelatedAnimalView {
            id: animal.id,
            display_id: animal.display_id,
            sex: animal.sex,
            strain: animal.strain,
            current_status: animal.current_status,
        },
        revision: entry.relation.meta.revision,
    }
}

fn attachment_view(attachment: Attachment) -> AttachmentMetadataView {
    AttachmentMetadataView {
        id: attachment.id,
        project_id: attachment.project_id,
        entity_type: attachment.entity_type,
        entity_id: attachment.entity_id,
        file_name: attachment.file_name,
        media_type: attachment.media_type,
        size_bytes: attachment.size_bytes,
        sha256: attachment.sha256,
        version: attachment.version,
        content_href: format!("/api/v1/attachments/{}/content", attachment.id),
        created_at: attachment.meta.created_at,
        revision: attachment.meta.revision,
    }
}

fn audit_view(audit: AnimalAuditEntry) -> AuditSummaryView {
    let entry = audit.entry;
    AuditSummaryView {
        id: entry.id,
        action: entry.action,
        actor: entry.actor.display_name,
        source: entry.source,
        reason: entry.reason,
        occurred_at: entry.occurred_at,
        revision: audit.revision,
    }
}
