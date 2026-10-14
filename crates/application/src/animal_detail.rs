//! Animal detail read model shared by the Server REST API and the Desktop.
//!
//! Transports authenticate and decide which projects and sections the caller
//! may see; this use case owns assembly, scope consistency, bounds and ordering
//! so both runtimes return the same facts for the same request.

use std::collections::{BTreeMap, BTreeSet};

use muriarc_core::{
    Animal, AnimalEvent, AnimalFilter, AnimalProjectRef, Attachment, AuditEntry, AuditFilter,
    Cohort, EntityType, Experiment, Measurement, MeasurementFilter, MuriArcStore, Participation,
    ParticipationFilter, Pedigree, Provenance, ProvenanceFilter, Sample, SampleFilter, StoreError,
};
use uuid::Uuid;

use crate::{ApplicationError, ApplicationResult};

pub const MAX_ANIMAL_DETAIL_PROJECTS: usize = 64;
pub const MAX_ANIMAL_DETAIL_PEDIGREE_EDGES: usize = 500;
const PROJECT_LOOKUP_LIMIT: u32 = 500;

/// A detail request whose visibility has already been authorized by the
/// transport. The use case never widens the listed projects or sections.
#[derive(Debug, Clone)]
pub struct AnimalDetailQuery {
    pub lab_id: Uuid,
    pub animal_id: Uuid,
    /// Selected project context; restricts events, pedigree, attachments and audit.
    pub project_id: Option<Uuid>,
    /// Projects whose experiment participation the caller may read.
    pub projects: Vec<AnimalProjectRef>,
    pub measurement_project_ids: BTreeSet<Uuid>,
    pub sample_project_ids: BTreeSet<Uuid>,
    pub include_attachments: bool,
    pub include_audit: bool,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimalDetail {
    /// Newest first.
    pub events: Vec<AnimalEvent>,
    /// Newest enrollment first.
    pub experiments: Vec<AnimalExperimentEntry>,
    pub measurements: Vec<Measurement>,
    pub pedigree: Vec<AnimalPedigreeEntry>,
    pub samples: Vec<Sample>,
    pub attachments: Vec<Attachment>,
    pub audits: Vec<AnimalAuditEntry>,
    pub provenance: Vec<AnimalProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimalExperimentEntry {
    pub project: AnimalProjectRef,
    pub experiment: Experiment,
    pub participation: Participation,
    pub cohort: Option<Cohort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimalPedigreeDirection {
    Parent,
    Offspring,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimalPedigreeEntry {
    pub relation: Pedigree,
    pub direction: AnimalPedigreeDirection,
    pub related_animal: Animal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimalAuditEntry {
    pub entry: AuditEntry,
    /// Record revision captured by the audit snapshot, when present.
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimalProvenanceEntry {
    pub record: Provenance,
    /// Display name of the actor when it also appears in the visible audit rows.
    pub actor_name: Option<String>,
}

/// Projects the animal is currently assigned to.
pub async fn animal_project_refs(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    animal: &Animal,
) -> ApplicationResult<Vec<AnimalProjectRef>> {
    let candidates = store
        .list_animal_overviews(
            &AnimalFilter {
                lab_id,
                query: Some(animal.display_id.clone()),
                ..AnimalFilter::default()
            },
            0,
            PROJECT_LOOKUP_LIMIT,
        )
        .await?;
    Ok(candidates
        .into_iter()
        .find(|candidate| candidate.animal.id == animal.id)
        .map(|candidate| candidate.projects)
        .unwrap_or_default())
}

pub async fn load_animal_detail(
    store: &dyn MuriArcStore,
    query: &AnimalDetailQuery,
) -> ApplicationResult<AnimalDetail> {
    if query.projects.len() > MAX_ANIMAL_DETAIL_PROJECTS {
        return Err(ApplicationError::Validation(format!(
            "animal is linked to more than {MAX_ANIMAL_DETAIL_PROJECTS} projects; select a project context"
        )));
    }
    let limit = query.limit;

    let mut events = store.list_animal_events(query.animal_id).await?;
    if let Some(project_id) = query.project_id {
        events.retain(|event| event.project_id == Some(project_id));
    }
    events.reverse();
    events.truncate(limit);

    let mut experiments = Vec::new();
    let mut measurements = Vec::new();
    let mut samples = Vec::new();
    for project in &query.projects {
        let participations = store
            .list_participations(&ParticipationFilter {
                project_id: project.id,
                experiment_id: None,
                animal_id: Some(query.animal_id),
                cohort_id: None,
            })
            .await?;
        for participation in participations {
            if experiments.len() >= limit {
                break;
            }
            let experiment = store.get_experiment(participation.experiment_id).await?;
            if experiment.lab_id != query.lab_id || experiment.project_id != project.id {
                return Err(StoreError::NotFound {
                    entity: "experiment",
                    id: experiment.id,
                }
                .into());
            }
            let cohort = match participation.cohort_id {
                Some(cohort_id) => store
                    .list_cohorts(experiment.id)
                    .await?
                    .into_iter()
                    .find(|cohort| cohort.id == cohort_id),
                None => None,
            };
            experiments.push(AnimalExperimentEntry {
                project: project.clone(),
                experiment,
                participation,
                cohort,
            });
        }

        if measurements.len() < limit && query.measurement_project_ids.contains(&project.id) {
            let mut rows = store
                .list_measurements(&MeasurementFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(query.animal_id),
                })
                .await?;
            rows.reverse();
            measurements.extend(rows);
            measurements.truncate(limit);
        }
        if samples.len() < limit && query.sample_project_ids.contains(&project.id) {
            let mut rows = store
                .list_samples(&SampleFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(query.animal_id),
                })
                .await?;
            rows.reverse();
            samples.extend(rows);
            samples.truncate(limit);
        }
    }
    experiments.sort_by_key(|entry| std::cmp::Reverse(entry.participation.enrolled_at));
    experiments.truncate(limit);
    measurements.sort_by_key(|measurement| std::cmp::Reverse(measurement.measured_at));
    samples.sort_by_key(|sample| std::cmp::Reverse(sample.collected_at));

    let pedigree = load_pedigree(store, query).await?;

    let attachments = if query.include_attachments {
        let mut rows = store
            .list_attachments(query.lab_id, "animal", query.animal_id)
            .await?;
        if let Some(project_id) = query.project_id {
            rows.retain(|attachment| attachment.project_id == Some(project_id));
        }
        rows.reverse();
        rows.truncate(limit);
        rows
    } else {
        Vec::new()
    };

    let (audits, provenance) = if query.include_audit {
        load_audit(store, query).await?
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(AnimalDetail {
        events,
        experiments,
        measurements,
        pedigree,
        samples,
        attachments,
        audits,
        provenance,
    })
}

async fn load_pedigree(
    store: &dyn MuriArcStore,
    query: &AnimalDetailQuery,
) -> ApplicationResult<Vec<AnimalPedigreeEntry>> {
    let relations = store.list_related_pedigrees(query.animal_id).await?;
    if relations.len() > MAX_ANIMAL_DETAIL_PEDIGREE_EDGES {
        return Err(ApplicationError::Validation(format!(
            "pedigree contains more than {MAX_ANIMAL_DETAIL_PEDIGREE_EDGES} active edges"
        )));
    }
    let related_id = |relation: &Pedigree| {
        if relation.animal_id == query.animal_id {
            (AnimalPedigreeDirection::Parent, relation.parent_id)
        } else {
            (AnimalPedigreeDirection::Offspring, relation.animal_id)
        }
    };
    let related_ids = relations
        .iter()
        .map(|relation| related_id(relation).1)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    // Relatives outside the selected project context are invisible, not errors.
    let related = store
        .list_animals_by_ids(query.lab_id, query.project_id, &related_ids)
        .await?
        .into_iter()
        .map(|animal| (animal.id, animal))
        .collect::<BTreeMap<_, _>>();
    Ok(relations
        .into_iter()
        .filter_map(|relation| {
            let (direction, id) = related_id(&relation);
            let related_animal = related.get(&id)?.clone();
            Some(AnimalPedigreeEntry {
                relation,
                direction,
                related_animal,
            })
        })
        .collect())
}

async fn load_audit(
    store: &dyn MuriArcStore,
    query: &AnimalDetailQuery,
) -> ApplicationResult<(Vec<AnimalAuditEntry>, Vec<AnimalProvenanceEntry>)> {
    let mut audit_rows = store
        .list_audit_entries(&AuditFilter {
            lab_id: query.lab_id,
            project_id: query.project_id,
            entity_id: Some(query.animal_id),
        })
        .await?;
    audit_rows.reverse();
    audit_rows.truncate(query.limit);
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
            AnimalAuditEntry { entry, revision }
        })
        .collect();

    let mut provenance_rows = store
        .list_provenance(&ProvenanceFilter {
            lab_id: query.lab_id,
            project_id: query.project_id,
            entity_type: Some(EntityType::Animal),
            entity_id: Some(query.animal_id),
            source: None,
        })
        .await?;
    provenance_rows.reverse();
    provenance_rows.truncate(query.limit);
    let provenance = provenance_rows
        .into_iter()
        .map(|record| AnimalProvenanceEntry {
            actor_name: record
                .actor_user_id
                .and_then(|actor_id| actor_names.get(&actor_id).cloned()),
            record,
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
