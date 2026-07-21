use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateAlleleCommand, CreateGeneLocusCommand, CreateGenotypeCommand, CreatePedigreeCommand,
    CreateSampleCommand, create_allele as create_allele_use_case,
    create_gene_locus as create_gene_locus_use_case, create_genotype as create_genotype_use_case,
    create_pedigree as create_pedigree_use_case, create_sample as create_sample_use_case,
};
use muriarc_core::{
    Allele, Animal, AnimalEvent, AnimalEventKind, AnimalFilter, AnimalProjectRef, AnimalStatus,
    Attachment, AuditAction, AuditFilter, GeneLocus, Genotype, Measurement, MeasurementFilter,
    MeasurementValue, MuriArcStore, ParentType, ParticipationFilter, ParticipationStatus,
    ProvenanceFilter, ProvenanceSource, RecordStatus, Sample, SampleFilter, Sex, WriteSource,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::application::{DesktopError, DesktopState};

const DETAIL_LIMIT: usize = 500;
const MAX_PROJECTS_PER_ANIMAL: usize = 64;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnimalDetailView {
    timeline: Vec<TimelineEventView>,
    experiments: Vec<AnimalExperimentView>,
    measurements: Vec<MeasurementView>,
    pedigree: Vec<PedigreeRelationView>,
    samples: Vec<SampleView>,
    attachments: Vec<AttachmentMetadataView>,
    audit_visible: bool,
    audits: Vec<AuditSummaryView>,
    provenance: Vec<ProvenanceSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnimalExperimentView {
    project_id: String,
    project_name: String,
    experiment_id: String,
    experiment_name: String,
    experiment_status: String,
    cohort_id: Option<String>,
    cohort_name: Option<String>,
    participation_id: String,
    participation_status: String,
    enrolled_at: String,
    exited_at: Option<String>,
    revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeasurementView {
    id: String,
    project_id: String,
    experiment_id: Option<String>,
    key: String,
    label: String,
    value: MeasurementValue,
    unit: Option<String>,
    measured_at: String,
    status: String,
    revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleView {
    id: String,
    project_id: String,
    experiment_id: Option<String>,
    sample_type: String,
    quantity: Option<f64>,
    unit: Option<String>,
    location: Option<String>,
    collected_at: String,
    revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeneLocusView {
    id: String,
    symbol: String,
    description: Option<String>,
    revision: i64,
}

impl From<GeneLocus> for GeneLocusView {
    fn from(value: GeneLocus) -> Self {
        Self {
            id: value.id.to_string(),
            symbol: value.symbol,
            description: value.description,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlleleView {
    id: String,
    locus_id: String,
    symbol: String,
    description: Option<String>,
    is_wild_type: bool,
    revision: i64,
}

impl From<Allele> for AlleleView {
    fn from(value: Allele) -> Self {
        Self {
            id: value.id.to_string(),
            locus_id: value.locus_id.to_string(),
            symbol: value.symbol,
            description: value.description,
            is_wild_type: value.is_wild_type,
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GenotypeView {
    id: String,
    animal_id: String,
    locus_id: String,
    allele_1_id: Option<String>,
    allele_2_id: Option<String>,
    assessed_at: Option<String>,
    revision: i64,
}

impl From<Genotype> for GenotypeView {
    fn from(value: Genotype) -> Self {
        Self {
            id: value.id.to_string(),
            animal_id: value.animal_id.to_string(),
            locus_id: value.locus_id.to_string(),
            allele_1_id: value.allele_1_id.map(|id| id.to_string()),
            allele_2_id: value.allele_2_id.map(|id| id.to_string()),
            assessed_at: value.assessed_at.map(|at| at.to_rfc3339()),
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum PedigreeDirection {
    Parent,
    Offspring,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PedigreeRelationView {
    id: String,
    direction: PedigreeDirection,
    parent_type: ParentType,
    related_animal: RelatedAnimalView,
    revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelatedAnimalView {
    id: String,
    code: String,
    sex: String,
    strain: Option<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentMetadataView {
    id: String,
    project_id: Option<String>,
    entity_type: String,
    entity_id: String,
    file_name: String,
    media_type: Option<String>,
    size_bytes: i64,
    sha256: String,
    version: i32,
    content_href: String,
    created_at: String,
    revision: i64,
}

impl From<Attachment> for AttachmentMetadataView {
    fn from(value: Attachment) -> Self {
        Self {
            id: value.id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            entity_type: value.entity_type,
            entity_id: value.entity_id.to_string(),
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
            version: value.version,
            content_href: String::new(),
            created_at: value.meta.created_at.to_rfc3339(),
            revision: value.meta.revision,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditSummaryView {
    id: String,
    action: AuditAction,
    actor: String,
    source: WriteSource,
    reason: Option<String>,
    occurred_at: String,
    revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvenanceSummaryView {
    id: String,
    source: ProvenanceSource,
    actor: Option<String>,
    recorded_at: String,
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAnimalSampleInput {
    pub animal_id: String,
    pub project_id: String,
    pub experiment_id: Option<String>,
    pub sample_type: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub location: Option<String>,
    pub collected_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreatePedigreeInput {
    pub project_id: Option<String>,
    pub animal_id: String,
    pub parent_id: String,
    pub parent_type: ParentType,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateGeneLocusInput {
    pub project_id: Option<String>,
    pub symbol: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAlleleInput {
    pub project_id: Option<String>,
    pub locus_id: String,
    pub symbol: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_wild_type: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateGenotypeInput {
    pub project_id: Option<String>,
    pub animal_id: String,
    pub locus_id: String,
    pub allele_1_id: Option<String>,
    pub allele_2_id: Option<String>,
    pub assessed_at: Option<String>,
}

impl DesktopState {
    pub(crate) async fn get_animal_detail(
        &self,
        animal_id: &str,
        project_id: Option<&str>,
    ) -> Result<AnimalDetailView, DesktopError> {
        let animal_id = parse_id("animal", animal_id)?;
        let project_id = project_id.map(|id| parse_id("project", id)).transpose()?;
        let animal = self.read_store().get_animal(animal_id).await?;
        if animal.lab_id != self.lab_id() {
            return Err(muriarc_core::StoreError::NotFound {
                entity: "animal",
                id: animal_id,
            }
            .into());
        }
        let projects = animal_projects(self, &animal, project_id).await?;
        if projects.len() > MAX_PROJECTS_PER_ANIMAL {
            return Err(muriarc_core::StoreError::Validation(format!(
                "动物关联项目超过 {MAX_PROJECTS_PER_ANIMAL} 个，请选择项目上下文"
            ))
            .into());
        }

        let cages = self
            .read_store()
            .list_cages(self.lab_id())
            .await?
            .into_iter()
            .map(|cage| (cage.id, cage.display_id))
            .collect::<BTreeMap<_, _>>();
        let operator = self
            .read_store()
            .get_user(self.user_id())
            .await?
            .display_name;
        let mut events = self.read_store().list_animal_events(animal_id).await?;
        if let Some(project_id) = project_id {
            events.retain(|event| event.project_id == Some(project_id));
        }
        events.reverse();
        events.truncate(DETAIL_LIMIT);
        let timeline = events
            .into_iter()
            .map(|event| timeline_event(event, &cages, self.user_id(), &operator))
            .collect();

        let mut experiments = Vec::new();
        let mut measurements = Vec::new();
        let mut samples = Vec::new();
        for project in &projects {
            for participation in self
                .read_store()
                .list_participations(&ParticipationFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(animal_id),
                    cohort_id: None,
                })
                .await?
            {
                if experiments.len() >= DETAIL_LIMIT {
                    break;
                }
                let experiment = self
                    .read_store()
                    .get_experiment(participation.experiment_id)
                    .await?;
                let cohort = match participation.cohort_id {
                    Some(cohort_id) => self
                        .read_store()
                        .list_cohorts(experiment.id)
                        .await?
                        .into_iter()
                        .find(|cohort| cohort.id == cohort_id),
                    None => None,
                };
                experiments.push(AnimalExperimentView {
                    project_id: project.id.to_string(),
                    project_name: project.name.clone(),
                    experiment_id: experiment.id.to_string(),
                    experiment_name: experiment.name,
                    experiment_status: format!("{:?}", experiment.status).to_ascii_lowercase(),
                    cohort_id: cohort.as_ref().map(|cohort| cohort.id.to_string()),
                    cohort_name: cohort.map(|cohort| cohort.name),
                    participation_id: participation.id.to_string(),
                    participation_status: participation_status(participation.status),
                    enrolled_at: participation.enrolled_at.to_rfc3339(),
                    exited_at: participation.exited_at.map(|value| value.to_rfc3339()),
                    revision: participation.meta.revision,
                });
            }

            let mut project_measurements = self
                .read_store()
                .list_measurements(&MeasurementFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(animal_id),
                })
                .await?;
            project_measurements.reverse();
            measurements.extend(project_measurements.into_iter().map(measurement_view));
            measurements.truncate(DETAIL_LIMIT);

            let mut project_samples = self
                .read_store()
                .list_samples(&SampleFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: Some(animal_id),
                })
                .await?;
            project_samples.reverse();
            samples.extend(project_samples.into_iter().map(sample_view));
            samples.truncate(DETAIL_LIMIT);
        }
        experiments.sort_by(|left, right| right.enrolled_at.cmp(&left.enrolled_at));
        measurements.sort_by(|left, right| right.measured_at.cmp(&left.measured_at));
        samples.sort_by(|left, right| right.collected_at.cmp(&left.collected_at));

        let pedigree = pedigree_views(self, animal_id, project_id).await?;
        let mut attachments = self
            .read_store()
            .list_attachments(self.lab_id(), "animal", animal_id)
            .await?;
        attachments.retain(|attachment| match project_id {
            Some(project_id) => attachment.project_id == Some(project_id),
            None => true,
        });
        attachments.reverse();
        attachments.truncate(DETAIL_LIMIT);

        let mut audit_rows = self
            .read_store()
            .list_audit_entries(&AuditFilter {
                lab_id: self.lab_id(),
                project_id,
                entity_id: Some(animal_id),
            })
            .await?;
        audit_rows.reverse();
        audit_rows.truncate(DETAIL_LIMIT);
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
            .map(|entry| AuditSummaryView {
                id: entry.id.to_string(),
                action: entry.action,
                actor: entry.actor.display_name,
                source: entry.source,
                reason: entry.reason,
                occurred_at: entry.occurred_at.to_rfc3339(),
                revision: entry
                    .after
                    .as_ref()
                    .or(entry.before.as_ref())
                    .and_then(record_revision),
            })
            .collect();

        let mut provenance_rows = self
            .read_store()
            .list_provenance(&ProvenanceFilter {
                lab_id: self.lab_id(),
                project_id,
                entity_type: Some(muriarc_core::EntityType::Animal),
                entity_id: Some(animal_id),
                source: None,
            })
            .await?;
        provenance_rows.reverse();
        provenance_rows.truncate(DETAIL_LIMIT);
        let provenance = provenance_rows
            .into_iter()
            .map(|entry| ProvenanceSummaryView {
                id: entry.id.to_string(),
                source: entry.source,
                actor: entry
                    .actor_user_id
                    .and_then(|actor_id| actor_names.get(&actor_id).cloned()),
                recorded_at: entry.recorded_at.to_rfc3339(),
                request_id: entry.request_id,
            })
            .collect();

        Ok(AnimalDetailView {
            timeline,
            experiments,
            measurements,
            pedigree,
            samples,
            attachments: attachments.into_iter().map(Into::into).collect(),
            audit_visible: true,
            audits,
            provenance,
        })
    }

    pub(crate) async fn create_animal_sample(
        &self,
        input: CreateAnimalSampleInput,
    ) -> Result<SampleView, DesktopError> {
        let animal_id = parse_id("animal", &input.animal_id)?;
        let project_id = parse_id("project", &input.project_id)?;
        let animal = self.read_store().get_animal(animal_id).await?;
        let project = self.read_store().get_project(project_id).await?;
        if animal.lab_id != self.lab_id() || project.lab_id != self.lab_id() {
            return Err(muriarc_core::StoreError::NotFound {
                entity: "animal",
                id: animal_id,
            }
            .into());
        }
        let visible = self
            .read_store()
            .list_animals_by_ids(self.lab_id(), Some(project_id), &[animal_id])
            .await?;
        if visible.is_empty() {
            return Err(muriarc_core::StoreError::Validation(
                "动物尚未参与所选项目实验".to_owned(),
            )
            .into());
        }
        let experiment_id = input
            .experiment_id
            .as_deref()
            .map(|id| parse_id("experiment", id))
            .transpose()?;
        if let Some(experiment_id) = experiment_id {
            let experiment = self.read_store().get_experiment(experiment_id).await?;
            if experiment.lab_id != self.lab_id() || experiment.project_id != project_id {
                return Err(
                    muriarc_core::StoreError::Validation("实验不属于所选项目".to_owned()).into(),
                );
            }
        }
        let collected_at = input
            .collected_at
            .as_deref()
            .map(parse_datetime)
            .transpose()?
            .unwrap_or_else(Utc::now);
        let now = Utc::now();
        let audit = self.audit("create_animal_sample").await?;
        let sample = create_sample_use_case(
            self.read_store(),
            CreateSampleCommand {
                lab_id: self.lab_id(),
                project_id,
                experiment_id,
                animal_id,
                sample_type: input.sample_type,
                quantity: input.quantity,
                unit: input.unit,
                location: input.location,
                collected_at,
                now,
            },
            &audit,
        )
        .await?;
        Ok(sample_view(sample))
    }

    pub(crate) async fn list_gene_loci(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<GeneLocusView>, DesktopError> {
        validate_project_scope(self, project_id).await?;
        Ok(self
            .read_store()
            .list_gene_loci(self.lab_id())
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub(crate) async fn create_gene_locus(
        &self,
        input: CreateGeneLocusInput,
    ) -> Result<GeneLocusView, DesktopError> {
        validate_project_scope(self, input.project_id.as_deref()).await?;
        let audit = self.audit("create_gene_locus").await?;
        let locus = create_gene_locus_use_case(
            self.read_store(),
            CreateGeneLocusCommand {
                lab_id: self.lab_id(),
                symbol: input.symbol,
                description: input.description,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(locus.into())
    }

    pub(crate) async fn list_alleles(
        &self,
        locus_id: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<AlleleView>, DesktopError> {
        validate_project_scope(self, project_id).await?;
        let locus = visible_locus(self, parse_id("gene_locus", locus_id)?).await?;
        Ok(self
            .read_store()
            .list_alleles(locus.id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub(crate) async fn create_allele(
        &self,
        input: CreateAlleleInput,
    ) -> Result<AlleleView, DesktopError> {
        validate_project_scope(self, input.project_id.as_deref()).await?;
        let locus_id = parse_id("gene_locus", &input.locus_id)?;
        visible_locus(self, locus_id).await?;
        let audit = self.audit("create_allele").await?;
        let allele = create_allele_use_case(
            self.read_store(),
            CreateAlleleCommand {
                locus_id,
                symbol: input.symbol,
                description: input.description,
                is_wild_type: input.is_wild_type,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(allele.into())
    }

    pub(crate) async fn list_genotypes(
        &self,
        animal_id: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<GenotypeView>, DesktopError> {
        let animal_id = parse_id("animal", animal_id)?;
        let project_id = validate_project_scope(self, project_id).await?;
        validate_animal_scope(self, animal_id, project_id).await?;
        Ok(self
            .read_store()
            .list_genotypes(animal_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub(crate) async fn create_genotype(
        &self,
        input: CreateGenotypeInput,
    ) -> Result<GenotypeView, DesktopError> {
        let animal_id = parse_id("animal", &input.animal_id)?;
        let project_id = validate_project_scope(self, input.project_id.as_deref()).await?;
        validate_animal_scope(self, animal_id, project_id).await?;
        let locus_id = parse_id("gene_locus", &input.locus_id)?;
        visible_locus(self, locus_id).await?;
        let assessed_at = input
            .assessed_at
            .as_deref()
            .map(|value| parse_rfc3339("genotype.assessed_at", value))
            .transpose()?;
        let allele_1_id = input
            .allele_1_id
            .as_deref()
            .map(|id| parse_id("allele_1", id))
            .transpose()?;
        let allele_2_id = input
            .allele_2_id
            .as_deref()
            .map(|id| parse_id("allele_2", id))
            .transpose()?;
        let audit = self.audit("create_genotype").await?;
        let genotype = create_genotype_use_case(
            self.read_store(),
            CreateGenotypeCommand {
                animal_id,
                locus_id,
                allele_1_id,
                allele_2_id,
                assessed_at,
                project_id,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(genotype.into())
    }

    pub(crate) async fn create_pedigree_relation(
        &self,
        input: CreatePedigreeInput,
    ) -> Result<PedigreeRelationView, DesktopError> {
        let animal_id = parse_id("animal", &input.animal_id)?;
        let parent_id = parse_id("parent", &input.parent_id)?;
        if animal_id == parent_id {
            return Err(
                muriarc_core::StoreError::Validation("动物不能是自己的父母".to_owned()).into(),
            );
        }
        let project_id = input
            .project_id
            .as_deref()
            .map(|id| parse_id("project", id))
            .transpose()?;
        let animals = self
            .read_store()
            .list_animals_by_ids(self.lab_id(), project_id, &[animal_id, parent_id])
            .await?;
        if animals.len() != 2 {
            return Err(muriarc_core::StoreError::Validation(
                "动物或父母在当前范围内不可见".to_owned(),
            )
            .into());
        }
        let parent = animals
            .iter()
            .find(|animal| animal.id == parent_id)
            .expect("validated parent visibility");
        let audit = self.audit("create_pedigree_relation").await?;
        let pedigree = create_pedigree_use_case(
            self.read_store(),
            CreatePedigreeCommand {
                animal_id,
                parent_id,
                parent_type: input.parent_type,
                now: Utc::now(),
            },
            &audit,
        )
        .await?;
        Ok(PedigreeRelationView {
            id: pedigree.id.to_string(),
            direction: PedigreeDirection::Parent,
            parent_type: pedigree.parent_type,
            related_animal: related_animal(parent),
            revision: pedigree.meta.revision,
        })
    }
}

async fn animal_projects(
    state: &DesktopState,
    animal: &Animal,
    project_id: Option<Uuid>,
) -> Result<Vec<AnimalProjectRef>, DesktopError> {
    let overview = state
        .read_store()
        .list_animal_overviews(
            &AnimalFilter {
                lab_id: state.lab_id(),
                query: Some(animal.display_id.clone()),
                ..AnimalFilter::default()
            },
            0,
            500,
        )
        .await?
        .into_iter()
        .find(|candidate| candidate.animal.id == animal.id)
        .ok_or(muriarc_core::StoreError::NotFound {
            entity: "animal",
            id: animal.id,
        })?;
    match project_id {
        Some(project_id) => overview
            .projects
            .into_iter()
            .find(|project| project.id == project_id)
            .map(|project| vec![project])
            .ok_or_else(|| {
                muriarc_core::StoreError::NotFound {
                    entity: "animal",
                    id: animal.id,
                }
                .into()
            }),
        None => Ok(overview.projects),
    }
}

async fn pedigree_views(
    state: &DesktopState,
    animal_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Vec<PedigreeRelationView>, DesktopError> {
    let relations = state.read_store().list_related_pedigrees(animal_id).await?;
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
    let animals = state
        .read_store()
        .list_animals_by_ids(state.lab_id(), project_id, &related_ids)
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
            let animal = animals.get(&related_id)?;
            Some(PedigreeRelationView {
                id: relation.id.to_string(),
                direction,
                parent_type: relation.parent_type,
                related_animal: related_animal(animal),
                revision: relation.meta.revision,
            })
        })
        .collect())
}

fn related_animal(animal: &Animal) -> RelatedAnimalView {
    RelatedAnimalView {
        id: animal.id.to_string(),
        code: animal.display_id.clone(),
        sex: match animal.sex {
            Sex::Male => "male",
            Sex::Female => "female",
            Sex::Unknown => "unknown",
        }
        .to_owned(),
        strain: animal.strain.clone(),
        status: match animal.current_status {
            AnimalStatus::InExperiment | AnimalStatus::Sampled => "experiment",
            AnimalStatus::Deceased
            | AnimalStatus::Euthanized
            | AnimalStatus::Lost
            | AnimalStatus::Archived => "archived",
            AnimalStatus::Planned | AnimalStatus::Alive => "active",
        }
        .to_owned(),
    }
}

fn measurement_view(value: Measurement) -> MeasurementView {
    MeasurementView {
        id: value.id.to_string(),
        project_id: value.project_id.to_string(),
        experiment_id: value.experiment_id.map(|id| id.to_string()),
        key: value.key,
        label: value.label,
        value: value.value,
        unit: value.unit,
        measured_at: value.measured_at.to_rfc3339(),
        status: match value.status {
            RecordStatus::Draft => "draft",
            RecordStatus::Signed => "signed",
        }
        .to_owned(),
        revision: value.meta.revision,
    }
}

fn sample_view(value: Sample) -> SampleView {
    SampleView {
        id: value.id.to_string(),
        project_id: value.project_id.to_string(),
        experiment_id: value.experiment_id.map(|id| id.to_string()),
        sample_type: value.sample_type,
        quantity: value.quantity,
        unit: value.unit,
        location: value.location,
        collected_at: value.collected_at.to_rfc3339(),
        revision: value.meta.revision,
    }
}

fn timeline_event(
    event: AnimalEvent,
    cages: &BTreeMap<Uuid, String>,
    local_user_id: Uuid,
    operator_name: &str,
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
            operator_name
        } else {
            "MuriArc"
        }
        .to_owned(),
    }
}

fn participation_status(status: ParticipationStatus) -> String {
    match status {
        ParticipationStatus::Enrolled => "enrolled",
        ParticipationStatus::Completed => "completed",
        ParticipationStatus::Withdrawn => "withdrawn",
    }
    .to_owned()
}

fn parse_id(field: &'static str, value: &str) -> Result<Uuid, DesktopError> {
    Uuid::parse_str(value).map_err(|_| DesktopError::InvalidId { field })
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, DesktopError> {
    parse_rfc3339("sample.collected_at", value)
}

fn parse_rfc3339(field: &'static str, value: &str) -> Result<DateTime<Utc>, DesktopError> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| DesktopError::InvalidDate { field })
}

async fn validate_project_scope(
    state: &DesktopState,
    project_id: Option<&str>,
) -> Result<Option<Uuid>, DesktopError> {
    let project_id = project_id.map(|id| parse_id("project", id)).transpose()?;
    if let Some(project_id) = project_id {
        let project = state.read_store().get_project(project_id).await?;
        if project.lab_id != state.lab_id() {
            return Err(muriarc_core::StoreError::NotFound {
                entity: "project",
                id: project_id,
            }
            .into());
        }
    }
    Ok(project_id)
}

async fn visible_locus(state: &DesktopState, locus_id: Uuid) -> Result<GeneLocus, DesktopError> {
    let locus = state.read_store().get_gene_locus(locus_id).await?;
    if locus.lab_id != state.lab_id() {
        return Err(muriarc_core::StoreError::NotFound {
            entity: "gene_locus",
            id: locus_id,
        }
        .into());
    }
    Ok(locus)
}

async fn validate_animal_scope(
    state: &DesktopState,
    animal_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<(), DesktopError> {
    let animal = state.read_store().get_animal(animal_id).await?;
    if animal.lab_id != state.lab_id() {
        return Err(muriarc_core::StoreError::NotFound {
            entity: "animal",
            id: animal_id,
        }
        .into());
    }
    if project_id.is_some()
        && state
            .read_store()
            .list_animals_by_ids(state.lab_id(), project_id, &[animal_id])
            .await?
            .is_empty()
    {
        return Err(
            muriarc_core::StoreError::Validation("动物在当前项目范围内不可见".to_owned()).into(),
        );
    }
    Ok(())
}

fn record_revision(value: &serde_json::Value) -> Option<i64> {
    value
        .get("meta")
        .and_then(|meta| meta.get("revision"))
        .and_then(serde_json::Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{
        AnimalIdentifierScopeInput, CreateAnimalInput, CreateExperimentInput, CreateProjectInput,
        CreateTemplateInput, EnrollAnimalInput,
    };
    use muriarc_core::{EntityType, FieldValueType, LOCAL_LAB_ID, LOCAL_USER_ID, ProvenanceFilter};
    use serde::Serialize;
    use tempfile::{TempDir, tempdir};

    fn serialized_id(value: &impl Serialize) -> String {
        serde_json::to_value(value).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn seeded_state(
        animal_codes: &[&str],
    ) -> (TempDir, DesktopState, String, String, Vec<String>) {
        let temp = tempdir().unwrap();
        let state = DesktopState::initialize(temp.path().join("muriarc.sqlite3"))
            .await
            .unwrap();
        let project = state
            .create_project(CreateProjectInput {
                name: "谱系与样本测试".to_owned(),
                description: "desktop animal detail test".to_owned(),
            })
            .await
            .unwrap();
        let project_id = serialized_id(&project);
        let template = state
            .create_published_template(CreateTemplateInput {
                name: "常规观察".to_owned(),
                description: "desktop animal detail test".to_owned(),
                field_key: "body_weight".to_owned(),
                field_label: "体重".to_owned(),
                field_value_type: FieldValueType::Number,
                field_unit: "g".to_owned(),
            })
            .await
            .unwrap();
        let experiment = state
            .create_experiment(CreateExperimentInput {
                project_id: project_id.clone(),
                template_version_id: serialized_id(&template),
                name: "详情测试实验".to_owned(),
                description: String::new(),
                start_date: Some("2026-07-19".to_owned()),
            })
            .await
            .unwrap();
        let experiment_id = serialized_id(&experiment);
        let mut animal_ids = Vec::with_capacity(animal_codes.len());
        for code in animal_codes {
            let animal = state
                .create_animal(CreateAnimalInput {
                    display_id: (*code).to_owned(),
                    identifier_scope: AnimalIdentifierScopeInput::Project,
                    project_id: Some(project_id.clone()),
                    cage_id: None,
                    sex: Sex::Unknown,
                    strain: "C57BL/6J".to_owned(),
                    birth_date: None,
                })
                .await
                .unwrap();
            let animal_id = serialized_id(&animal);
            state
                .enroll_animal(EnrollAnimalInput {
                    experiment_id: experiment_id.clone(),
                    animal_id: animal_id.clone(),
                    cohort_id: None,
                })
                .await
                .unwrap();
            animal_ids.push(animal_id);
        }
        (temp, state, project_id, experiment_id, animal_ids)
    }

    #[tokio::test]
    async fn sample_write_is_atomic_audited_and_visible_in_detail() {
        let (_temp, state, project_id, experiment_id, animals) =
            seeded_state(&["SAMPLE-001"]).await;
        let animal_id = &animals[0];
        let animal_uuid = Uuid::parse_str(animal_id).unwrap();

        let rejected = state
            .create_animal_sample(CreateAnimalSampleInput {
                animal_id: animal_id.clone(),
                project_id: project_id.clone(),
                experiment_id: Some(experiment_id.clone()),
                sample_type: "lung tissue".to_owned(),
                quantity: Some(12.5),
                unit: None,
                location: None,
                collected_at: None,
            })
            .await;
        assert!(rejected.is_err());
        assert!(
            state
                .read_store()
                .list_samples(&SampleFilter {
                    project_id: Uuid::parse_str(&project_id).unwrap(),
                    experiment_id: None,
                    animal_id: Some(animal_uuid),
                })
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !state
                .read_store()
                .list_animal_events(animal_uuid)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(event.kind, AnimalEventKind::SampleCollected { .. }))
        );

        let sample = state
            .create_animal_sample(CreateAnimalSampleInput {
                animal_id: animal_id.clone(),
                project_id: project_id.clone(),
                experiment_id: Some(experiment_id),
                sample_type: "lung tissue".to_owned(),
                quantity: Some(12.5),
                unit: Some("mg".to_owned()),
                location: Some("-80C/A/Box-3/A2".to_owned()),
                collected_at: Some("2026-07-19T08:30:00Z".to_owned()),
            })
            .await
            .unwrap();
        let sample_id = Uuid::parse_str(&sample.id).unwrap();

        let stored = state.read_store().get_sample(sample_id).await.unwrap();
        assert_eq!(stored.sample_type, "lung tissue");
        assert_eq!(stored.quantity, Some(12.5));
        assert!(
            state
                .read_store()
                .list_animal_events(animal_uuid)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(
                    event.kind,
                    AnimalEventKind::SampleCollected { sample_id: id, .. } if id == sample_id
                ))
        );
        let audits = state
            .read_store()
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: Some(Uuid::parse_str(&project_id).unwrap()),
                entity_id: Some(sample_id),
            })
            .await
            .unwrap();
        assert!(audits.iter().any(|entry| {
            entry.entity_type == EntityType::Sample
                && entry.action == AuditAction::Create
                && entry.actor.user_id == Some(LOCAL_USER_ID)
                && entry.source == WriteSource::Desktop
        }));
        let provenance = state
            .read_store()
            .list_provenance(&ProvenanceFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: Some(Uuid::parse_str(&project_id).unwrap()),
                entity_type: Some(EntityType::Sample),
                entity_id: Some(sample_id),
                source: None,
            })
            .await
            .unwrap();
        assert_eq!(provenance.len(), 1);

        let detail = state
            .get_animal_detail(animal_id, Some(&project_id))
            .await
            .unwrap();
        assert!(detail.samples.iter().any(|row| row.id == sample.id));
        assert!(
            detail
                .timeline
                .iter()
                .any(|event| event.event_type == "sampling")
        );
    }

    #[tokio::test]
    async fn genotype_write_is_scoped_audited_and_visible_in_timeline() {
        let (_temp, state, project_id, _experiment_id, animals) =
            seeded_state(&["GENOTYPE-001"]).await;
        let animal_id = &animals[0];
        let animal_uuid = Uuid::parse_str(animal_id).unwrap();

        let locus = state
            .create_gene_locus(CreateGeneLocusInput {
                project_id: Some(project_id.clone()),
                symbol: "GeneA".to_owned(),
                description: Some("mechanosensor".to_owned()),
            })
            .await
            .unwrap();
        let wild_type = state
            .create_allele(CreateAlleleInput {
                project_id: Some(project_id.clone()),
                locus_id: locus.id.clone(),
                symbol: "+".to_owned(),
                description: None,
                is_wild_type: true,
            })
            .await
            .unwrap();
        let flox = state
            .create_allele(CreateAlleleInput {
                project_id: Some(project_id.clone()),
                locus_id: locus.id.clone(),
                symbol: "flox".to_owned(),
                description: None,
                is_wild_type: false,
            })
            .await
            .unwrap();
        let genotype = state
            .create_genotype(CreateGenotypeInput {
                project_id: Some(project_id.clone()),
                animal_id: animal_id.clone(),
                locus_id: locus.id.clone(),
                allele_1_id: Some(wild_type.id.clone()),
                allele_2_id: Some(flox.id.clone()),
                assessed_at: Some("2026-07-19T09:00:00Z".to_owned()),
            })
            .await
            .unwrap();

        assert_eq!(
            state.list_gene_loci(Some(&project_id)).await.unwrap().len(),
            1
        );
        assert_eq!(
            state
                .list_alleles(&locus.id, Some(&project_id))
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            state
                .list_genotypes(animal_id, Some(&project_id))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            state
                .read_store()
                .list_animal_events(animal_uuid)
                .await
                .unwrap()
                .iter()
                .any(|event| {
                    event.project_id == Some(Uuid::parse_str(&project_id).unwrap())
                        && matches!(
                            &event.kind,
                            AnimalEventKind::Genotyped { genotype_ids }
                                if genotype_ids == &vec![Uuid::parse_str(&genotype.id).unwrap()]
                        )
                })
        );
        let detail = state
            .get_animal_detail(animal_id, Some(&project_id))
            .await
            .unwrap();
        assert!(
            detail
                .timeline
                .iter()
                .any(|event| event.event_type == "genotype")
        );

        for (entity_type, entity_id, scoped_project) in [
            (
                EntityType::GeneLocus,
                Uuid::parse_str(&locus.id).unwrap(),
                None,
            ),
            (
                EntityType::Allele,
                Uuid::parse_str(&wild_type.id).unwrap(),
                None,
            ),
            (
                EntityType::Genotype,
                Uuid::parse_str(&genotype.id).unwrap(),
                Some(Uuid::parse_str(&project_id).unwrap()),
            ),
        ] {
            let provenance = state
                .read_store()
                .list_provenance(&ProvenanceFilter {
                    lab_id: LOCAL_LAB_ID,
                    project_id: scoped_project,
                    entity_type: Some(entity_type),
                    entity_id: Some(entity_id),
                    source: None,
                })
                .await
                .unwrap();
            assert_eq!(provenance.len(), 1);
            assert_eq!(provenance[0].source, ProvenanceSource::Human);
        }

        let other_project = state
            .create_project(CreateProjectInput {
                name: "不可见基因型项目".to_owned(),
                description: String::new(),
            })
            .await
            .unwrap();
        let rejected = state
            .create_genotype(CreateGenotypeInput {
                project_id: Some(serialized_id(&other_project)),
                animal_id: animal_id.clone(),
                locus_id: locus.id,
                allele_1_id: Some(wild_type.id),
                allele_2_id: Some(flox.id),
                assessed_at: None,
            })
            .await;
        assert!(rejected.is_err());
        assert_eq!(
            state
                .list_genotypes(animal_id, Some(&project_id))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn pedigree_write_is_audited_and_visible_from_both_directions() {
        let (_temp, state, project_id, _experiment_id, animals) =
            seeded_state(&["OFFSPRING-001", "PARENT-001"]).await;
        let offspring_id = &animals[0];
        let parent_id = &animals[1];

        let rejected = state
            .create_pedigree_relation(CreatePedigreeInput {
                project_id: Some(project_id.clone()),
                animal_id: offspring_id.clone(),
                parent_id: offspring_id.clone(),
                parent_type: ParentType::Unknown,
            })
            .await;
        assert!(rejected.is_err());

        let relation = state
            .create_pedigree_relation(CreatePedigreeInput {
                project_id: Some(project_id.clone()),
                animal_id: offspring_id.clone(),
                parent_id: parent_id.clone(),
                parent_type: ParentType::Father,
            })
            .await
            .unwrap();
        let relation_id = Uuid::parse_str(&relation.id).unwrap();
        let stored = state.read_store().get_pedigree(relation_id).await.unwrap();
        assert_eq!(stored.parent_type, ParentType::Father);

        let offspring_detail = state
            .get_animal_detail(offspring_id, Some(&project_id))
            .await
            .unwrap();
        assert!(offspring_detail.pedigree.iter().any(|row| {
            row.id == relation.id && matches!(row.direction, PedigreeDirection::Parent)
        }));
        let parent_detail = state
            .get_animal_detail(parent_id, Some(&project_id))
            .await
            .unwrap();
        assert!(parent_detail.pedigree.iter().any(|row| {
            row.id == relation.id && matches!(row.direction, PedigreeDirection::Offspring)
        }));

        let audits = state
            .read_store()
            .list_audit_entries(&AuditFilter {
                lab_id: LOCAL_LAB_ID,
                project_id: None,
                entity_id: Some(relation_id),
            })
            .await
            .unwrap();
        assert!(audits.iter().any(|entry| {
            entry.entity_type == EntityType::Pedigree
                && entry.action == AuditAction::Create
                && entry.actor.user_id == Some(LOCAL_USER_ID)
                && entry.source == WriteSource::Desktop
        }));
    }
}
