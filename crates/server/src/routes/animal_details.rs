use std::collections::{BTreeMap, BTreeSet};

use axum::{Json, Router, extract::State, routing::get};
use muriarc_core::{
    Animal, AnimalEvent, AnimalFilter, AnimalOverview, AnimalProjectRef, AnimalStatus, AuditAction,
    AuditFilter, EntityType, Experiment, Measurement, MeasurementFilter, ParentType, Participation,
    ParticipationFilter, ParticipationStatus, Permission, ProvenanceFilter, ProvenanceSource,
    RecordStatus, Sample, SampleFilter, Sex, WriteSource,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiPath, ApiQuery, CollectionResponse, ItemResponse, collection, item, scope, store,
    validation::{collection_limit, truncate, validation},
};

const MAX_PROJECTS_PER_ANIMAL: usize = 64;
const MAX_PEDIGREE_EDGES: usize = 500;

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
    if projects.len() > MAX_PROJECTS_PER_ANIMAL {
        return Err(validation(
            format!(
                "animal is linked to more than {MAX_PROJECTS_PER_ANIMAL} projects; select a project context"
            ),
            &metadata,
        ));
    }

    let mut events = store(state.store.list_animal_events(animal_id), &metadata).await?;
    if let Some(project_id) = query.project_id {
        events.retain(|event| event.project_id == Some(project_id));
    }
    events.reverse();
    truncate(&mut events, limit);

    let mut experiments = Vec::new();
    let mut measurements: Vec<MeasurementView> = Vec::new();
    let mut samples: Vec<SampleView> = Vec::new();
    for project in &projects {
        scope::project_with_permission(
            &state,
            &principal,
            &metadata,
            project.id,
            Permission::ReadExperiment,
        )
        .await?;
        let participations = store(
            state.store.list_participations(&ParticipationFilter {
                project_id: project.id,
                experiment_id: None,
                animal_id: Some(animal_id),
                cohort_id: None,
            }),
            &metadata,
        )
        .await?;
        for participation in participations {
            if experiments.len() >= limit {
                break;
            }
            let experiment = store(
                state.store.get_experiment(participation.experiment_id),
                &metadata,
            )
            .await?;
            if experiment.lab_id != principal.lab_id || experiment.project_id != project.id {
                return Err(ApiError::not_found("experiment was not found")
                    .with_request_id(metadata.request_id.clone()));
            }
            let cohort = match participation.cohort_id {
                Some(cohort_id) => store(state.store.list_cohorts(experiment.id), &metadata)
                    .await?
                    .into_iter()
                    .find(|cohort| cohort.id == cohort_id)
                    .map(|cohort| CohortSummary {
                        id: cohort.id,
                        name: cohort.name,
                    }),
                None => None,
            };
            experiments.push(AnimalExperimentView {
                project: project.clone(),
                experiment: experiment.into(),
                participation: participation.into(),
                cohort,
            });
        }

        if measurements.len() < limit
            && principal.can(Permission::ReadMeasurement, Some(project.id))
        {
            let mut rows = store(
                state.store.list_measurements(&MeasurementFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(animal_id),
                }),
                &metadata,
            )
            .await?;
            rows.reverse();
            measurements.extend(rows.into_iter().map(Into::into));
            truncate(&mut measurements, limit);
        }
        if samples.len() < limit && principal.can(Permission::ReadSample, Some(project.id)) {
            let mut rows = store(
                state.store.list_samples(&SampleFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(animal_id),
                }),
                &metadata,
            )
            .await?;
            rows.reverse();
            samples.extend(rows.into_iter().map(Into::into));
            truncate(&mut samples, limit);
        }
    }
    experiments.sort_by(|left, right| {
        right
            .participation
            .enrolled_at
            .cmp(&left.participation.enrolled_at)
    });
    truncate(&mut experiments, limit);
    measurements.sort_by_key(|measurement| std::cmp::Reverse(measurement.measured_at));
    samples.sort_by_key(|sample| std::cmp::Reverse(sample.collected_at));

    let pedigree =
        pedigree_view(&state, &principal, &metadata, animal_id, query.project_id).await?;

    let attachments = if principal.can(Permission::ReadAttachment, query.project_id) {
        let mut rows = store(
            state
                .store
                .list_attachments(principal.lab_id, "animal", animal_id),
            &metadata,
        )
        .await?;
        rows.retain(|attachment| match query.project_id {
            Some(project_id) => attachment.project_id == Some(project_id),
            None => true,
        });
        rows.reverse();
        truncate(&mut rows, limit);
        rows.into_iter()
            .map(|attachment| AttachmentMetadataView {
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
            })
            .collect()
    } else {
        Vec::new()
    };

    let audit_visible = principal.can(Permission::ReadAudit, query.project_id);
    let (audits, provenance) = if audit_visible {
        audit_views(
            &state,
            &principal,
            &metadata,
            animal_id,
            query.project_id,
            limit,
        )
        .await?
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(item(
        AnimalDetailView {
            events,
            experiments,
            measurements,
            pedigree,
            samples,
            attachments,
            audit_visible,
            audits,
            provenance,
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

    let candidates = store(
        state.store.list_animal_overviews(
            &AnimalFilter {
                lab_id: principal.lab_id,
                query: Some(animal.display_id.clone()),
                ..AnimalFilter::default()
            },
            0,
            500,
        ),
        metadata,
    )
    .await?;
    Ok(candidates
        .into_iter()
        .find(|candidate| candidate.animal.id == animal.id)
        .map(|candidate| candidate.projects)
        .unwrap_or_default())
}

async fn pedigree_view(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    animal_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Vec<PedigreeRelationView>, ApiError> {
    let relations = store(state.store.list_related_pedigrees(animal_id), metadata).await?;
    if relations.len() > MAX_PEDIGREE_EDGES {
        return Err(validation(
            format!("pedigree contains more than {MAX_PEDIGREE_EDGES} active edges"),
            metadata,
        ));
    }
    let related_ids = relations
        .iter()
        .map(|relation| {
            if relation.animal_id == animal_id {
                relation.parent_id
            } else {
                relation.animal_id
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let related = store(
        state
            .store
            .list_animals_by_ids(principal.lab_id, project_id, &related_ids),
        metadata,
    )
    .await?
    .into_iter()
    .map(|animal| (animal.id, animal))
    .collect::<BTreeMap<_, _>>();

    Ok(relations
        .into_iter()
        .filter_map(|relation| {
            let (direction, related_id) = if relation.animal_id == animal_id {
                (PedigreeDirection::Parent, relation.parent_id)
            } else {
                (PedigreeDirection::Offspring, relation.animal_id)
            };
            let animal = related.get(&related_id)?;
            Some(PedigreeRelationView {
                id: relation.id,
                direction,
                parent_type: relation.parent_type,
                related_animal: RelatedAnimalView {
                    id: animal.id,
                    display_id: animal.display_id.clone(),
                    sex: animal.sex,
                    strain: animal.strain.clone(),
                    current_status: animal.current_status,
                },
                revision: relation.meta.revision,
            })
        })
        .collect())
}

async fn audit_views(
    state: &AppState,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
    animal_id: Uuid,
    project_id: Option<Uuid>,
    limit: usize,
) -> Result<(Vec<AuditSummaryView>, Vec<ProvenanceSummaryView>), ApiError> {
    let mut audit_rows = store(
        state.store.list_audit_entries(&AuditFilter {
            lab_id: principal.lab_id,
            project_id,
            entity_id: Some(animal_id),
        }),
        metadata,
    )
    .await?;
    audit_rows.reverse();
    truncate(&mut audit_rows, limit);
    let actor_names = audit_rows
        .iter()
        .filter_map(|entry| {
            entry
                .actor
                .user_id
                .map(|id| (id, entry.actor.display_name.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let audits = audit_rows
        .into_iter()
        .map(|entry| {
            let revision = entry
                .after
                .as_ref()
                .or(entry.before.as_ref())
                .and_then(record_revision);
            AuditSummaryView {
                id: entry.id,
                action: entry.action,
                actor: entry.actor.display_name,
                source: entry.source,
                reason: entry.reason,
                occurred_at: entry.occurred_at,
                revision,
            }
        })
        .collect();

    let mut provenance_rows = store(
        state.store.list_provenance(&ProvenanceFilter {
            lab_id: principal.lab_id,
            project_id,
            entity_type: Some(EntityType::Animal),
            entity_id: Some(animal_id),
            source: None,
        }),
        metadata,
    )
    .await?;
    provenance_rows.reverse();
    truncate(&mut provenance_rows, limit);
    let provenance = provenance_rows
        .into_iter()
        .map(|entry| ProvenanceSummaryView {
            id: entry.id,
            source: entry.source,
            actor: entry
                .actor_user_id
                .and_then(|actor_id| actor_names.get(&actor_id).cloned()),
            recorded_at: entry.recorded_at,
            request_id: entry.request_id,
        })
        .collect();
    Ok((audits, provenance))
}

fn record_revision(value: &serde_json::Value) -> Option<i64> {
    value
        .get("meta")
        .and_then(|meta| meta.get("revision"))
        .and_then(serde_json::Value::as_i64)
}
