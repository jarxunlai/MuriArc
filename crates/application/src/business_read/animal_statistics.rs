//! Fixed-schema animal counts for whole-project or lab-registry questions.
//!
//! The model-facing list reads are paginated; answering "how many animals"
//! by paging through every row is slow and can exceed a model's input budget.
//! This read counts distinct animals in memory over existing Store reads and
//! returns only totals and bounded group counts, never animal rows.

use std::collections::{BTreeMap, BTreeSet};

use muriarc_core::{
    AnimalFilter, AnimalStatus, CurrentGenotypingRecordFilter, CurrentGenotypingRecordOverview,
    EntityType, GenotypingState, Sex,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    BusinessReadError, BusinessReadResult, BusinessReadService, BusinessSourceRef,
    ReadPermissionState, RecordScope,
};

pub const DEFAULT_STATISTICS_GROUPS: u32 = 50;
pub const MAX_STATISTICS_GROUPS: u32 = 200;
pub const MAX_STATISTICS_GROUP_DIMENSIONS: usize = 2;
pub const MAX_STATISTICS_GENOTYPE_FILTERS: usize = 4;
/// Scope guard: larger scopes must be narrowed (for example to one project).
pub const MAX_STATISTICS_ANIMALS: usize = 50_000;
const GENOTYPE_PAGE: u32 = 1_000;
const MAX_GENOTYPE_PAGES: u32 = 100;
const MAX_GENOTYPE_NAME_CHARS: usize = 128;
const MAX_AVAILABLE_DEFINITIONS: usize = 50;
const NONE_VALUE: &str = "(none)";
const NO_CAGE: &str = "(no cage)";
const UNLISTED_CAGE: &str = "(unlisted cage)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimalStatisticsDimension {
    Status,
    Sex,
    Strain,
    Cage,
    GenotypeDefinition,
    GenotypingState,
}

impl AnimalStatisticsDimension {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Sex => "sex",
            Self::Strain => "strain",
            Self::Cage => "cage",
            Self::GenotypeDefinition => "genotype_definition",
            Self::GenotypingState => "genotyping_state",
        }
    }

    /// Genotype dimensions describe current records, and an animal may have
    /// several, so it can appear in more than one group.
    const fn is_genotype(self) -> bool {
        matches!(self, Self::GenotypeDefinition | Self::GenotypingState)
    }
}

/// Counts distinct animals in one authorized scope.
///
/// Filters combine with AND. `genotype_definitions` matches current Genetics
/// v2 definition names case- and whitespace-insensitively; an animal matches
/// only when it has a current record for every listed definition.
/// `genotyping_state` restricts which current records are considered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimalStatisticsRequest {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub animal_status: Option<AnimalStatus>,
    #[serde(default)]
    pub sex: Option<Sex>,
    #[serde(default)]
    pub cage_id: Option<Uuid>,
    #[serde(default)]
    pub genotype_definitions: Vec<String>,
    #[serde(default)]
    pub genotyping_state: Option<GenotypingState>,
    #[serde(default)]
    pub group_by: Vec<AnimalStatisticsDimension>,
    #[serde(default)]
    pub max_groups: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalStatisticsGroup {
    /// Dimension name to display value, for example `{"sex": "female"}`.
    pub key: BTreeMap<String, String>,
    /// Identifiers for entity-valued dimensions (cage, genotype definition).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ids: BTreeMap<String, Uuid>,
    /// Distinct matching animals in this group.
    pub animals: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalStatistics {
    pub record_scope: RecordScope,
    /// Every non-deleted animal in the scope, before filters.
    pub animals_in_scope: usize,
    /// Distinct animals that satisfy every filter.
    pub matching_animals: usize,
    pub group_by: Vec<AnimalStatisticsDimension>,
    /// Largest groups first, at most `max_groups`.
    pub groups: Vec<AnimalStatisticsGroup>,
    pub groups_total: usize,
    pub groups_truncated: bool,
    /// True when an animal can be counted in several groups, so group sizes
    /// may sum to more than `matching_animals`.
    pub multi_valued_groups: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched_genotype_definitions: Vec<String>,
    /// Definition names present in scope; returned only for unmatched names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_genotype_definitions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub permission_state: ReadPermissionState,
}

struct GenotypeFact {
    definition_id: Uuid,
    definition_name: String,
    state: GenotypingState,
}

impl BusinessReadService {
    pub async fn animal_statistics(
        &self,
        request: AnimalStatisticsRequest,
    ) -> Result<BusinessReadResult<AnimalStatistics>, BusinessReadError> {
        let max_groups = validate_request(&request)?;
        let project = match request.project_id {
            Some(project_id) => Some(self.authorize_project(project_id).await?),
            None => {
                self.require_lab_registry()
                    .map_err(|_| BusinessReadError::Rejected("project_required"))?;
                None
            }
        };
        let lab_id = self.access.lab_id;
        let mut animals = self
            .store
            .list_animals(&AnimalFilter {
                lab_id,
                project_id: request.project_id,
                ..AnimalFilter::default()
            })
            .await?;
        animals.retain(|animal| animal.lab_id == lab_id);
        if animals.len() > MAX_STATISTICS_ANIMALS {
            return Err(BusinessReadError::Rejected("statistics_scope_too_large"));
        }
        let animals_in_scope = animals.len();
        animals.retain(|animal| {
            request
                .animal_status
                .is_none_or(|status| animal.current_status == status)
                && request.sex.is_none_or(|sex| animal.sex == sex)
                && request
                    .cage_id
                    .is_none_or(|cage_id| animal.current_cage_id == Some(cage_id))
        });

        let genotype_grouping = request
            .group_by
            .iter()
            .any(|dimension| dimension.is_genotype());
        let needs_genotypes = genotype_grouping
            || !request.genotype_definitions.is_empty()
            || request.genotyping_state.is_some();
        let facts = if needs_genotypes {
            self.current_genotype_facts(request.project_id, request.genotyping_state)
                .await?
        } else {
            BTreeMap::new()
        };

        // Resolve requested definition names against the lab's definitions.
        let mut unmatched = Vec::new();
        let mut required_definitions = Vec::new();
        if !request.genotype_definitions.is_empty() {
            let definitions = self
                .store
                .list_genotype_definitions_including_archived(lab_id)
                .await?;
            for name in &request.genotype_definitions {
                let wanted = normalized_name(name);
                let ids = definitions
                    .iter()
                    .filter(|definition| {
                        definition.lab_id == lab_id && normalized_name(&definition.name) == wanted
                    })
                    .map(|definition| definition.id)
                    .collect::<BTreeSet<_>>();
                if ids.is_empty() {
                    unmatched.push(name.trim().to_owned());
                }
                required_definitions.push(ids);
            }
        }
        let matched_definition_ids = required_definitions
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();

        let mut matching = Vec::new();
        for animal in &animals {
            let animal_facts = facts.get(&animal.id).map(Vec::as_slice).unwrap_or(&[]);
            // A state filter alone keeps animals with a current record in
            // that state.
            let passes_genotype_filter = if required_definitions.is_empty() {
                request.genotyping_state.is_none() || !animal_facts.is_empty()
            } else {
                required_definitions.iter().all(|ids| {
                    animal_facts
                        .iter()
                        .any(|fact| ids.contains(&fact.definition_id))
                })
            };
            if passes_genotype_filter {
                // With a definition filter, genotype dimensions describe only
                // the matched records.
                let grouped_facts = animal_facts
                    .iter()
                    .filter(|fact| {
                        matched_definition_ids.is_empty()
                            || matched_definition_ids.contains(&fact.definition_id)
                    })
                    .collect::<Vec<_>>();
                matching.push((animal, grouped_facts));
            }
        }

        let cage_labels = if request.group_by.contains(&AnimalStatisticsDimension::Cage) {
            let cages = match &project {
                Some(project) => {
                    self.store
                        .list_cages_for_project(lab_id, project.id)
                        .await?
                }
                None => self.store.list_cages(lab_id).await?,
            };
            cages
                .into_iter()
                .filter(|cage| cage.lab_id == lab_id)
                .map(|cage| (cage.id, (cage.display_id, cage.meta.revision)))
                .collect::<BTreeMap<_, _>>()
        } else {
            BTreeMap::new()
        };

        let mut counts = BTreeMap::<Vec<(String, String, Option<Uuid>)>, usize>::new();
        if !request.group_by.is_empty() {
            for (animal, animal_facts) in &matching {
                let fact_choices = if genotype_grouping && !animal_facts.is_empty() {
                    animal_facts
                        .iter()
                        .map(|fact| Some(*fact))
                        .collect::<Vec<_>>()
                } else {
                    vec![None]
                };
                let keys = fact_choices
                    .into_iter()
                    .map(|fact| {
                        request
                            .group_by
                            .iter()
                            .map(|dimension| {
                                let (value, id) =
                                    dimension_value(*dimension, animal, fact, &cage_labels);
                                (dimension.as_str().to_owned(), value, id)
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<BTreeSet<_>>();
                for key in keys {
                    *counts.entry(key).or_default() += 1;
                }
            }
        }
        let groups_total = counts.len();
        let mut ordered = counts.into_iter().collect::<Vec<_>>();
        ordered.sort_by(|(left_key, left), (right_key, right)| {
            right.cmp(left).then_with(|| left_key.cmp(right_key))
        });
        ordered.truncate(max_groups);
        let groups = ordered
            .into_iter()
            .map(|(key, animals)| AnimalStatisticsGroup {
                ids: key
                    .iter()
                    .filter_map(|(dimension, _, id)| id.map(|id| (dimension.clone(), id)))
                    .collect(),
                key: key
                    .into_iter()
                    .map(|(dimension, value, _)| (dimension, value))
                    .collect(),
                animals,
            })
            .collect::<Vec<_>>();

        let mut available = Vec::new();
        let mut note = None;
        if !unmatched.is_empty() {
            available = facts
                .values()
                .flatten()
                .map(|fact| fact.definition_name.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(MAX_AVAILABLE_DEFINITIONS)
                .collect();
            note = Some(format!(
                "No current genotype definition is named {}. A definition can combine several \
                 loci (for example \"Prox1 Cre+/Cre-; Ai9 tdT+/tdT-\"); retry with an exact name \
                 from available_genotype_definitions or group_by genotype_definition.",
                unmatched
                    .iter()
                    .map(|name| format!("\"{name}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let mut sources = Vec::new();
        if let Some(project) = &project {
            sources.push(BusinessSourceRef::new(
                EntityType::Project,
                project.id,
                Some(project.meta.revision),
            ));
        }
        let mut cited = BTreeSet::new();
        for id in &matched_definition_ids {
            if cited.insert(*id) {
                sources.push(BusinessSourceRef::new(
                    EntityType::GenotypeDefinition,
                    *id,
                    None,
                ));
            }
        }
        for group in &groups {
            for (dimension, id) in &group.ids {
                if !cited.insert(*id) {
                    continue;
                }
                if dimension == AnimalStatisticsDimension::Cage.as_str() {
                    let revision = cage_labels.get(id).map(|(_, revision)| *revision);
                    sources.push(BusinessSourceRef::new(EntityType::Cage, *id, revision));
                } else {
                    sources.push(BusinessSourceRef::new(
                        EntityType::GenotypeDefinition,
                        *id,
                        None,
                    ));
                }
            }
        }

        Ok(BusinessReadResult {
            data: AnimalStatistics {
                record_scope: self.scope(request.project_id),
                animals_in_scope,
                matching_animals: matching.len(),
                group_by: request.group_by,
                groups_truncated: groups.len() < groups_total,
                groups,
                groups_total,
                multi_valued_groups: genotype_grouping,
                unmatched_genotype_definitions: unmatched,
                available_genotype_definitions: available,
                note,
                permission_state: ReadPermissionState::Granted,
            },
            sources,
        })
    }

    /// Loads current Genetics v2 facts for the scope with bounded pages.
    async fn current_genotype_facts(
        &self,
        project_id: Option<Uuid>,
        state: Option<GenotypingState>,
    ) -> Result<BTreeMap<Uuid, Vec<GenotypeFact>>, BusinessReadError> {
        let filter = CurrentGenotypingRecordFilter {
            lab_id: self.access.lab_id,
            project_id,
            animal_id: None,
            state,
        };
        let mut facts = BTreeMap::<Uuid, Vec<GenotypeFact>>::new();
        for page in 0..MAX_GENOTYPE_PAGES {
            let records = self
                .store
                .list_current_genotyping_record_overviews(
                    &filter,
                    page * GENOTYPE_PAGE,
                    GENOTYPE_PAGE,
                )
                .await?;
            let complete = records.len() < GENOTYPE_PAGE as usize;
            for CurrentGenotypingRecordOverview {
                record,
                genotype_definition_name,
                ..
            } in records
            {
                if record.lab_id == self.access.lab_id {
                    facts
                        .entry(record.animal_id)
                        .or_default()
                        .push(GenotypeFact {
                            definition_id: record.genotype_definition_id,
                            definition_name: genotype_definition_name,
                            state: record.state,
                        });
                }
            }
            if complete {
                return Ok(facts);
            }
        }
        Err(BusinessReadError::Rejected("statistics_scope_too_large"))
    }
}

fn validate_request(request: &AnimalStatisticsRequest) -> Result<usize, BusinessReadError> {
    let max_groups = request.max_groups.unwrap_or(DEFAULT_STATISTICS_GROUPS);
    if !(1..=MAX_STATISTICS_GROUPS).contains(&max_groups) {
        return Err(BusinessReadError::Rejected(
            "statistics_max_groups_out_of_range",
        ));
    }
    let distinct_dimensions = request.group_by.iter().collect::<BTreeSet<_>>();
    if request.group_by.len() > MAX_STATISTICS_GROUP_DIMENSIONS
        || distinct_dimensions.len() != request.group_by.len()
    {
        return Err(BusinessReadError::Rejected("statistics_group_by_invalid"));
    }
    let names = request
        .genotype_definitions
        .iter()
        .map(|name| normalized_name(name))
        .collect::<BTreeSet<_>>();
    if request.genotype_definitions.len() > MAX_STATISTICS_GENOTYPE_FILTERS
        || names.len() != request.genotype_definitions.len()
        || request.genotype_definitions.iter().any(|name| {
            name.trim().is_empty()
                || name.chars().count() > MAX_GENOTYPE_NAME_CHARS
                || name.chars().any(char::is_control)
        })
    {
        return Err(BusinessReadError::Rejected(
            "statistics_genotype_definitions_invalid",
        ));
    }
    Ok(max_groups as usize)
}

/// Case- and whitespace-insensitive key; full-width separators are folded.
fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| match character {
            '；' => ';',
            '，' => ',',
            '／' => '/',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn dimension_value(
    dimension: AnimalStatisticsDimension,
    animal: &muriarc_core::Animal,
    fact: Option<&GenotypeFact>,
    cage_labels: &BTreeMap<Uuid, (String, i64)>,
) -> (String, Option<Uuid>) {
    match dimension {
        AnimalStatisticsDimension::Status => (wire_value(animal.current_status), None),
        AnimalStatisticsDimension::Sex => (wire_value(animal.sex), None),
        AnimalStatisticsDimension::Strain => (
            animal
                .strain
                .as_deref()
                .map(str::trim)
                .filter(|strain| !strain.is_empty())
                .unwrap_or(NONE_VALUE)
                .to_owned(),
            None,
        ),
        AnimalStatisticsDimension::Cage => match animal.current_cage_id {
            None => (NO_CAGE.to_owned(), None),
            Some(cage_id) => match cage_labels.get(&cage_id) {
                Some((label, _)) => (label.clone(), Some(cage_id)),
                None => (UNLISTED_CAGE.to_owned(), None),
            },
        },
        AnimalStatisticsDimension::GenotypeDefinition => match fact {
            Some(fact) => (fact.definition_name.clone(), Some(fact.definition_id)),
            None => (NONE_VALUE.to_owned(), None),
        },
        AnimalStatisticsDimension::GenotypingState => match fact {
            Some(fact) => (wire_value(fact.state), None),
            None => (NONE_VALUE.to_owned(), None),
        },
    }
}

fn wire_value(value: impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| NONE_VALUE.to_owned())
}
