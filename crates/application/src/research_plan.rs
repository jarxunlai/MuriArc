use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const RESEARCH_GROUPING_SCHEMA_VERSION: u32 = 1;
/// Keeps one plan below SQLite's bounded-query and human-review limits.
pub const MAX_GROUPING_CANDIDATES: usize = 200;
pub const MAX_GROUPING_COHORTS: usize = 20;
pub const MAX_GROUPING_FACTORS: usize = 16;

/// One authorized animal snapshot used to create a deterministic grouping
/// preview. Values are typed planning inputs, not arbitrary persisted fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupingCandidate {
    pub animal_id: Uuid,
    pub expected_revision: i64,
    #[serde(default)]
    pub strata: BTreeMap<String, String>,
    #[serde(default)]
    pub covariates: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentGroupingRequest {
    pub project_id: Uuid,
    pub expected_project_revision: i64,
    pub experiment_id: Uuid,
    pub expected_experiment_revision: i64,
    pub seed: u64,
    pub cohort_names: Vec<String>,
    #[serde(default)]
    pub stratify_by: Vec<String>,
    #[serde(default)]
    pub balance_by: Vec<String>,
    pub candidates: Vec<GroupingCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupingAssignment {
    pub animal_id: Uuid,
    pub expected_revision: i64,
    pub cohort_index: usize,
    pub cohort_name: String,
    pub stratum: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingExclusion {
    pub animal_id: Uuid,
    pub expected_revision: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortBalanceSummary {
    pub cohort_index: usize,
    pub cohort_name: String,
    pub animal_count: usize,
    pub stratum_counts: BTreeMap<String, usize>,
    pub covariate_means: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentGroupingPlan {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub expected_project_revision: i64,
    pub experiment_id: Uuid,
    pub expected_experiment_revision: i64,
    pub seed: u64,
    pub input_snapshot_sha256: String,
    pub cohort_names: Vec<String>,
    pub stratify_by: Vec<String>,
    pub balance_by: Vec<String>,
    pub assignments: Vec<GroupingAssignment>,
    pub exclusions: Vec<GroupingExclusion>,
    pub balance_summary: Vec<CohortBalanceSummary>,
    /// Applying grouping changes scientific experiment state and therefore
    /// always remains behind an explicit researcher signature.
    pub requires_researcher_signature: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResearchPlanError {
    #[error("grouping scope or expected revision is invalid")]
    InvalidScope,
    #[error("grouping requires between two and twenty unique cohort names")]
    InvalidCohorts,
    #[error("grouping candidates are empty, duplicated or outside the bounded limit")]
    InvalidCandidates,
    #[error("grouping factors are invalid or outside the bounded limit")]
    InvalidFactors,
    #[error("grouping candidate data is invalid")]
    InvalidCandidateData,
    #[error("grouping plan serialization failed")]
    Serialization,
}

/// Produces a reproducible, review-only experiment grouping plan.
///
/// The same normalized candidate snapshot and seed always produce the same
/// assignment. Application/Store code must re-read every revision before
/// applying the plan and must reject already-enrolled animals.
pub fn build_experiment_grouping_plan(
    mut request: ExperimentGroupingRequest,
) -> Result<ExperimentGroupingPlan, ResearchPlanError> {
    validate_request(&request)?;
    request.cohort_names = request
        .cohort_names
        .into_iter()
        .map(|name| name.trim().to_owned())
        .collect();
    request.stratify_by.sort();
    request.balance_by.sort();
    request
        .candidates
        .sort_by_key(|candidate| candidate.animal_id);
    let input_snapshot_sha256 = snapshot_hash(&request)?;

    let mut exclusions = Vec::new();
    let mut eligible_by_stratum: BTreeMap<String, Vec<GroupingCandidate>> = BTreeMap::new();
    for candidate in request.candidates {
        if let Some(reason) = candidate
            .exclusion_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        {
            exclusions.push(GroupingExclusion {
                animal_id: candidate.animal_id,
                expected_revision: candidate.expected_revision,
                reason: reason.to_owned(),
            });
            continue;
        }
        let key = stratum_key(&candidate, &request.stratify_by);
        eligible_by_stratum.entry(key).or_default().push(candidate);
    }

    let cohort_count = request.cohort_names.len();
    let mut assignments = Vec::new();
    let mut overall_counts = vec![0_usize; cohort_count];
    let mut stratum_counts = vec![BTreeMap::<String, usize>::new(); cohort_count];
    let mut covariate_sums = vec![BTreeMap::<String, f64>::new(); cohort_count];
    let mut covariate_counts = vec![BTreeMap::<String, usize>::new(); cohort_count];

    for (stratum_key, mut candidates) in eligible_by_stratum {
        candidates.sort_by(|left, right| {
            candidate_rank(request.seed, &stratum_key, left.animal_id)
                .cmp(&candidate_rank(request.seed, &stratum_key, right.animal_id))
                .then_with(|| left.animal_id.cmp(&right.animal_id))
        });
        for candidate in candidates {
            let cohort_index = choose_cohort(
                request.seed,
                &stratum_key,
                &candidate,
                &request.balance_by,
                &overall_counts,
                &stratum_counts,
                &covariate_sums,
            );
            overall_counts[cohort_index] += 1;
            *stratum_counts[cohort_index]
                .entry(stratum_key.clone())
                .or_default() += 1;
            for key in &request.balance_by {
                if let Some(value) = candidate.covariates.get(key) {
                    *covariate_sums[cohort_index].entry(key.clone()).or_default() += value;
                    *covariate_counts[cohort_index]
                        .entry(key.clone())
                        .or_default() += 1;
                }
            }
            assignments.push(GroupingAssignment {
                animal_id: candidate.animal_id,
                expected_revision: candidate.expected_revision,
                cohort_index,
                cohort_name: request.cohort_names[cohort_index].clone(),
                stratum: request
                    .stratify_by
                    .iter()
                    .map(|key| {
                        (
                            key.clone(),
                            candidate
                                .strata
                                .get(key)
                                .cloned()
                                .unwrap_or_else(|| "<missing>".to_owned()),
                        )
                    })
                    .collect(),
            });
        }
    }
    assignments.sort_by_key(|assignment| assignment.animal_id);
    exclusions.sort_by_key(|exclusion| exclusion.animal_id);

    let balance_summary = request
        .cohort_names
        .iter()
        .enumerate()
        .map(|(index, name)| CohortBalanceSummary {
            cohort_index: index,
            cohort_name: name.clone(),
            animal_count: overall_counts[index],
            stratum_counts: stratum_counts[index].clone(),
            covariate_means: request
                .balance_by
                .iter()
                .filter_map(|key| {
                    let count = covariate_counts[index]
                        .get(key)
                        .copied()
                        .unwrap_or_default();
                    (count > 0).then(|| {
                        (
                            key.clone(),
                            covariate_sums[index].get(key).copied().unwrap_or_default()
                                / count as f64,
                        )
                    })
                })
                .collect(),
        })
        .collect();

    Ok(ExperimentGroupingPlan {
        schema_version: RESEARCH_GROUPING_SCHEMA_VERSION,
        project_id: request.project_id,
        expected_project_revision: request.expected_project_revision,
        experiment_id: request.experiment_id,
        expected_experiment_revision: request.expected_experiment_revision,
        seed: request.seed,
        input_snapshot_sha256,
        cohort_names: request.cohort_names,
        stratify_by: request.stratify_by,
        balance_by: request.balance_by,
        assignments,
        exclusions,
        balance_summary,
        requires_researcher_signature: true,
    })
}

fn validate_request(request: &ExperimentGroupingRequest) -> Result<(), ResearchPlanError> {
    if request.project_id.is_nil()
        || request.experiment_id.is_nil()
        || request.expected_project_revision <= 0
        || request.expected_experiment_revision <= 0
    {
        return Err(ResearchPlanError::InvalidScope);
    }
    if !(2..=MAX_GROUPING_COHORTS).contains(&request.cohort_names.len()) {
        return Err(ResearchPlanError::InvalidCohorts);
    }
    let cohort_names = request
        .cohort_names
        .iter()
        .map(|name| name.trim())
        .collect::<BTreeSet<_>>();
    if cohort_names.len() != request.cohort_names.len()
        || cohort_names
            .iter()
            .any(|name| name.is_empty() || name.chars().count() > 256)
    {
        return Err(ResearchPlanError::InvalidCohorts);
    }
    if request.candidates.is_empty() || request.candidates.len() > MAX_GROUPING_CANDIDATES {
        return Err(ResearchPlanError::InvalidCandidates);
    }
    let ids = request
        .candidates
        .iter()
        .map(|candidate| candidate.animal_id)
        .collect::<BTreeSet<_>>();
    if ids.len() != request.candidates.len() {
        return Err(ResearchPlanError::InvalidCandidates);
    }
    validate_factor_names(&request.stratify_by)?;
    validate_factor_names(&request.balance_by)?;
    for candidate in &request.candidates {
        if candidate.animal_id.is_nil()
            || candidate.expected_revision <= 0
            || candidate.strata.len() > MAX_GROUPING_FACTORS
            || candidate.covariates.len() > MAX_GROUPING_FACTORS
            || candidate
                .covariates
                .values()
                .any(|value| !value.is_finite())
            || candidate.exclusion_reason.as_ref().is_some_and(|reason| {
                reason.trim().is_empty()
                    || reason.chars().count() > 512
                    || reason.chars().any(|character| {
                        character.is_control() && !matches!(character, '\n' | '\t')
                    })
            })
        {
            return Err(ResearchPlanError::InvalidCandidateData);
        }
        for (key, value) in &candidate.strata {
            if !valid_factor_name(key) || value.chars().count() > 256 {
                return Err(ResearchPlanError::InvalidCandidateData);
            }
        }
        if candidate
            .covariates
            .keys()
            .any(|key| !valid_factor_name(key))
        {
            return Err(ResearchPlanError::InvalidCandidateData);
        }
    }
    Ok(())
}

fn validate_factor_names(values: &[String]) -> Result<(), ResearchPlanError> {
    if values.len() > MAX_GROUPING_FACTORS
        || values.iter().any(|value| !valid_factor_name(value))
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
    {
        Err(ResearchPlanError::InvalidFactors)
    } else {
        Ok(())
    }
}

fn valid_factor_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn snapshot_hash(request: &ExperimentGroupingRequest) -> Result<String, ResearchPlanError> {
    let encoded = serde_json::to_vec(request).map_err(|_| ResearchPlanError::Serialization)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn stratum_key(candidate: &GroupingCandidate, stratify_by: &[String]) -> String {
    if stratify_by.is_empty() {
        return "all".to_owned();
    }
    stratify_by
        .iter()
        .map(|key| {
            format!(
                "{key}={}",
                candidate
                    .strata
                    .get(key)
                    .map(String::as_str)
                    .unwrap_or("<missing>")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn candidate_rank(seed: u64, stratum: &str, animal_id: Uuid) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(seed.to_be_bytes());
    hasher.update(stratum.as_bytes());
    hasher.update(animal_id.as_bytes());
    hasher.finalize().into()
}

fn choose_cohort(
    seed: u64,
    stratum: &str,
    candidate: &GroupingCandidate,
    balance_by: &[String],
    overall_counts: &[usize],
    stratum_counts: &[BTreeMap<String, usize>],
    covariate_sums: &[BTreeMap<String, f64>],
) -> usize {
    (0..overall_counts.len())
        .min_by(|left, right| {
            let left_score = cohort_score(
                *left,
                seed,
                stratum,
                candidate,
                balance_by,
                overall_counts,
                stratum_counts,
                covariate_sums,
            );
            let right_score = cohort_score(
                *right,
                seed,
                stratum,
                candidate,
                balance_by,
                overall_counts,
                stratum_counts,
                covariate_sums,
            );
            compare_score(&left_score, &right_score)
        })
        .expect("at least two cohorts are validated")
}

#[allow(clippy::too_many_arguments)]
fn cohort_score(
    index: usize,
    seed: u64,
    stratum: &str,
    candidate: &GroupingCandidate,
    balance_by: &[String],
    overall_counts: &[usize],
    stratum_counts: &[BTreeMap<String, usize>],
    covariate_sums: &[BTreeMap<String, f64>],
) -> (usize, usize, u64, [u8; 32]) {
    let stratum_count = stratum_counts[index]
        .get(stratum)
        .copied()
        .unwrap_or_default();
    let magnitude = balance_by
        .iter()
        .filter_map(|key| candidate.covariates.get(key).map(|value| (key, value)))
        .map(|(key, value)| {
            (covariate_sums[index].get(key).copied().unwrap_or_default() + value).abs()
        })
        .sum::<f64>();
    let magnitude_rank = if magnitude.is_finite() {
        magnitude.to_bits()
    } else {
        u64::MAX
    };
    let mut hasher = Sha256::new();
    hasher.update(seed.to_be_bytes());
    hasher.update(stratum.as_bytes());
    hasher.update(candidate.animal_id.as_bytes());
    hasher.update((index as u64).to_be_bytes());
    (
        stratum_count,
        overall_counts[index],
        magnitude_rank,
        hasher.finalize().into(),
    )
}

fn compare_score(
    left: &(usize, usize, u64, [u8; 32]),
    right: &(usize, usize, u64, [u8; 32]),
) -> Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(&right.2))
        .then_with(|| left.3.cmp(&right.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(seed: u64) -> ExperimentGroupingRequest {
        ExperimentGroupingRequest {
            project_id: Uuid::from_u128(1),
            expected_project_revision: 2,
            experiment_id: Uuid::from_u128(2),
            expected_experiment_revision: 3,
            seed,
            cohort_names: vec!["Control".to_owned(), "Treatment".to_owned()],
            stratify_by: vec!["sex".to_owned()],
            balance_by: vec!["weight".to_owned()],
            candidates: (10..18)
                .map(|id| GroupingCandidate {
                    animal_id: Uuid::from_u128(id),
                    expected_revision: 1,
                    strata: BTreeMap::from([(
                        "sex".to_owned(),
                        if id.is_multiple_of(2) {
                            "female"
                        } else {
                            "male"
                        }
                        .to_owned(),
                    )]),
                    covariates: BTreeMap::from([("weight".to_owned(), id as f64)]),
                    exclusion_reason: (id == 17).then_some("predefined exclusion".to_owned()),
                })
                .collect(),
        }
    }

    #[test]
    fn same_snapshot_and_seed_are_reproducible() {
        let left = build_experiment_grouping_plan(request(42)).unwrap();
        let right = build_experiment_grouping_plan(request(42)).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.assignments.len(), 7);
        assert_eq!(left.exclusions.len(), 1);
        assert!(left.requires_researcher_signature);
        assert_eq!(left.input_snapshot_sha256.len(), 64);
    }

    #[test]
    fn strata_and_total_sizes_are_balanced() {
        let plan = build_experiment_grouping_plan(request(7)).unwrap();
        let sizes = plan
            .balance_summary
            .iter()
            .map(|summary| summary.animal_count)
            .collect::<Vec<_>>();
        assert!(sizes[0].abs_diff(sizes[1]) <= 1);
        for stratum in ["sex=female", "sex=male"] {
            let counts = plan
                .balance_summary
                .iter()
                .map(|summary| {
                    summary
                        .stratum_counts
                        .get(stratum)
                        .copied()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();
            assert!(counts[0].abs_diff(counts[1]) <= 1);
        }
    }

    #[test]
    fn duplicate_animals_and_unsafe_factor_names_are_rejected() {
        let mut duplicate = request(1);
        duplicate.candidates.push(duplicate.candidates[0].clone());
        assert_eq!(
            build_experiment_grouping_plan(duplicate),
            Err(ResearchPlanError::InvalidCandidates)
        );

        let mut unsafe_factor = request(1);
        unsafe_factor.stratify_by = vec!["raw.sql".to_owned()];
        assert_eq!(
            build_experiment_grouping_plan(unsafe_factor),
            Err(ResearchPlanError::InvalidFactors)
        );
    }
}
