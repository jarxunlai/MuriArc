use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DomainError, GenotypeComponent, GenotypeComponentMode, GenotypeDefinition, RecordMeta, Sex,
    require_non_empty,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreedingLine {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub genotype_definition_ids: Vec<Uuid>,
    pub meta: RecordMeta,
}

impl BreedingLine {
    pub fn new(
        lab_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("breeding_line.name", &name)?;
        if lab_id.is_nil() {
            return Err(DomainError::InvalidBreedingLine);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            name: name.trim().to_owned(),
            description: None,
            genotype_definition_ids: Vec::new(),
            meta: RecordMeta::new(now),
        })
    }

    pub fn replace_genotype_definitions(&mut self, ids: Vec<Uuid>) -> Result<(), DomainError> {
        let mut unique = HashSet::with_capacity(ids.len());
        if ids.is_empty() || ids.iter().any(|id| id.is_nil() || !unique.insert(*id)) {
            return Err(DomainError::InvalidBreedingLine);
        }
        self.genotype_definition_ids = ids;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Colony {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub breeding_line_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub meta: RecordMeta,
}

impl Colony {
    pub fn new(
        lab_id: Uuid,
        breeding_line_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("colony.name", &name)?;
        if lab_id.is_nil() || breeding_line_id.is_nil() {
            return Err(DomainError::InvalidColony);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            breeding_line_id,
            name: name.trim().to_owned(),
            description: None,
            meta: RecordMeta::new(now),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreedingPairStatus {
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreedingMemberRole {
    Male,
    Female,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreedingPairMember {
    pub id: Uuid,
    pub breeding_pair_id: Uuid,
    pub animal_id: Uuid,
    pub role: BreedingMemberRole,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

impl BreedingPairMember {
    pub fn new(
        breeding_pair_id: Uuid,
        animal_id: Uuid,
        role: BreedingMemberRole,
        joined_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let member = Self {
            id: Uuid::new_v4(),
            breeding_pair_id,
            animal_id,
            role,
            joined_at,
            left_at: None,
            meta: RecordMeta::new(now),
        };
        member.validate()?;
        Ok(member)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.breeding_pair_id.is_nil()
            || self.animal_id.is_nil()
            || self.left_at.is_some_and(|left_at| left_at < self.joined_at)
        {
            Err(DomainError::InvalidBreedingMember)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreedingPair {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub colony_id: Uuid,
    pub name: String,
    pub status: BreedingPairStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub members: Vec<BreedingPairMember>,
    pub meta: RecordMeta,
}

impl BreedingPair {
    pub fn new(
        lab_id: Uuid,
        colony_id: Uuid,
        name: impl Into<String>,
        started_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("breeding_pair.name", &name)?;
        if lab_id.is_nil() || colony_id.is_nil() {
            return Err(DomainError::InvalidBreedingPair);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            colony_id,
            name: name.trim().to_owned(),
            status: BreedingPairStatus::Active,
            started_at,
            ended_at: None,
            members: Vec::new(),
            meta: RecordMeta::new(now),
        })
    }

    pub fn replace_members(&mut self, members: Vec<BreedingPairMember>) -> Result<(), DomainError> {
        let mut animals = HashSet::with_capacity(members.len());
        let mut male_count = 0usize;
        let mut female_count = 0usize;
        for member in &members {
            member.validate()?;
            if member.breeding_pair_id != self.id
                || member.left_at.is_some()
                || !animals.insert(member.animal_id)
            {
                return Err(DomainError::InvalidBreedingPair);
            }
            match member.role {
                BreedingMemberRole::Male => male_count += 1,
                BreedingMemberRole::Female => female_count += 1,
            }
        }
        if male_count != 1 || female_count == 0 {
            return Err(DomainError::InvalidBreedingPair);
        }
        self.members = members;
        Ok(())
    }

    pub fn retire(&mut self, ended_at: DateTime<Utc>) -> Result<(), DomainError> {
        if self.status != BreedingPairStatus::Active || ended_at < self.started_at {
            return Err(DomainError::BreedingPairNotActive);
        }
        self.status = BreedingPairStatus::Retired;
        self.ended_at = Some(ended_at);
        for member in &mut self.members {
            if member.left_at.is_none() {
                member.left_at = Some(ended_at);
                member.meta.touch(ended_at);
            }
        }
        self.meta.touch(ended_at);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatingEvent {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub breeding_pair_id: Uuid,
    pub male_animal_id: Uuid,
    pub female_animal_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub meta: RecordMeta,
}

impl MatingEvent {
    pub fn new(
        lab_id: Uuid,
        breeding_pair_id: Uuid,
        male_animal_id: Uuid,
        female_animal_id: Uuid,
        occurred_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if lab_id.is_nil()
            || breeding_pair_id.is_nil()
            || male_animal_id.is_nil()
            || female_animal_id.is_nil()
            || male_animal_id == female_animal_id
        {
            return Err(DomainError::InvalidMatingEvent);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            breeding_pair_id,
            male_animal_id,
            female_animal_id,
            occurred_at,
            notes: None,
            meta: RecordMeta::new(now),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Litter {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub mating_event_id: Uuid,
    pub born_on: NaiveDate,
    pub size_total: i32,
    pub size_alive: i32,
    pub notes: Option<String>,
    pub meta: RecordMeta,
}

impl Litter {
    pub fn new(
        lab_id: Uuid,
        mating_event_id: Uuid,
        born_on: NaiveDate,
        size_total: i32,
        size_alive: i32,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if lab_id.is_nil()
            || mating_event_id.is_nil()
            || size_total < 0
            || size_alive < 0
            || size_alive > size_total
        {
            return Err(DomainError::InvalidLitter);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            mating_event_id,
            born_on,
            size_total,
            size_alive,
            notes: None,
            meta: RecordMeta::new(now),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimalDraftStatus {
    Pending,
    Registered,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalDraft {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub litter_id: Uuid,
    pub temporary_label: String,
    pub sex: Sex,
    pub birth_date: NaiveDate,
    pub status: AnimalDraftStatus,
    pub registered_animal_id: Option<Uuid>,
    pub meta: RecordMeta,
}

impl AnimalDraft {
    pub fn new(
        lab_id: Uuid,
        litter_id: Uuid,
        temporary_label: impl Into<String>,
        sex: Sex,
        birth_date: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let temporary_label = temporary_label.into();
        require_non_empty("animal_draft.temporary_label", &temporary_label)?;
        if lab_id.is_nil() || litter_id.is_nil() {
            return Err(DomainError::InvalidAnimalDraft);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            litter_id,
            temporary_label: temporary_label.trim().to_owned(),
            sex,
            birth_date,
            status: AnimalDraftStatus::Pending,
            registered_animal_id: None,
            meta: RecordMeta::new(now),
        })
    }

    pub fn mark_registered(
        &mut self,
        animal_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if self.status != AnimalDraftStatus::Pending || animal_id.is_nil() {
            return Err(DomainError::AnimalDraftNotPending);
        }
        self.status = AnimalDraftStatus::Registered;
        self.registered_animal_id = Some(animal_id);
        self.meta.touch(now);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MendelianOutcome {
    pub paternal_allele_id: Option<Uuid>,
    pub maternal_allele_id: Option<Uuid>,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocusPrediction {
    pub locus_id: Uuid,
    pub outcomes: Vec<MendelianOutcome>,
}

pub fn predict_mendelian(
    male: &GenotypeDefinition,
    female: &GenotypeDefinition,
) -> Result<Vec<LocusPrediction>, DomainError> {
    if male.lab_id != female.lab_id || male.components.is_empty() || female.components.is_empty() {
        return Err(DomainError::IncompatibleBreedingPrediction);
    }
    let male_components: BTreeMap<_, _> = male.components.iter().map(|c| (c.locus_id, c)).collect();
    let female_components: BTreeMap<_, _> =
        female.components.iter().map(|c| (c.locus_id, c)).collect();
    let loci: BTreeSet<_> = male_components
        .keys()
        .chain(female_components.keys())
        .copied()
        .collect();
    let mut predictions = Vec::with_capacity(loci.len());
    for locus_id in loci {
        let paternal = gametes(male_components.get(&locus_id).copied());
        let maternal = gametes(female_components.get(&locus_id).copied());
        let mut merged = BTreeMap::<(Option<Uuid>, Option<Uuid>), f64>::new();
        for (paternal_allele_id, paternal_probability) in &paternal {
            for (maternal_allele_id, maternal_probability) in &maternal {
                *merged
                    .entry((*paternal_allele_id, *maternal_allele_id))
                    .or_default() += paternal_probability * maternal_probability;
            }
        }
        predictions.push(LocusPrediction {
            locus_id,
            outcomes: merged
                .into_iter()
                .map(
                    |((paternal_allele_id, maternal_allele_id), probability)| MendelianOutcome {
                        paternal_allele_id,
                        maternal_allele_id,
                        probability,
                    },
                )
                .collect(),
        });
    }
    Ok(predictions)
}

fn gametes(component: Option<&GenotypeComponent>) -> Vec<(Option<Uuid>, f64)> {
    let Some(component) = component else {
        return vec![(None, 1.0)];
    };
    let alleles = match component.mode {
        GenotypeComponentMode::Diploid | GenotypeComponentMode::Conditional => {
            vec![
                (Some(component.allele_1_id), 0.5),
                (component.allele_2_id, 0.5),
            ]
        }
        GenotypeComponentMode::Hemizygous | GenotypeComponentMode::TransgenePresence => {
            vec![(Some(component.allele_1_id), 0.5), (None, 0.5)]
        }
    };
    let mut merged = BTreeMap::<Option<Uuid>, f64>::new();
    for (allele, probability) in alleles {
        *merged.entry(allele).or_default() += probability;
    }
    merged.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_male_and_multiple_females_are_valid() {
        let now = Utc::now();
        let mut pair =
            BreedingPair::new(Uuid::new_v4(), Uuid::new_v4(), "Pair 1", now, now).unwrap();
        let members = [
            BreedingMemberRole::Male,
            BreedingMemberRole::Female,
            BreedingMemberRole::Female,
        ]
        .into_iter()
        .map(|role| BreedingPairMember::new(pair.id, Uuid::new_v4(), role, now, now).unwrap())
        .collect();
        pair.replace_members(members).unwrap();
        assert_eq!(pair.members.len(), 3);
    }

    #[test]
    fn mendelian_cross_is_deterministic_and_sums_to_one() {
        let now = Utc::now();
        let lab_id = Uuid::new_v4();
        let locus_id = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut male = GenotypeDefinition::new(lab_id, "male", now).unwrap();
        male.replace_components(vec![
            GenotypeComponent::new(
                male.id,
                locus_id,
                a,
                Some(b),
                GenotypeComponentMode::Diploid,
                0,
                now,
            )
            .unwrap(),
        ])
        .unwrap();
        let mut female = GenotypeDefinition::new(lab_id, "female", now).unwrap();
        female
            .replace_components(vec![
                GenotypeComponent::new(
                    female.id,
                    locus_id,
                    a,
                    Some(b),
                    GenotypeComponentMode::Diploid,
                    0,
                    now,
                )
                .unwrap(),
            ])
            .unwrap();
        let prediction = predict_mendelian(&male, &female).unwrap();
        assert_eq!(prediction.len(), 1);
        assert_eq!(prediction[0].outcomes.len(), 4);
        let total: f64 = prediction[0]
            .outcomes
            .iter()
            .map(|outcome| outcome.probability)
            .sum();
        assert!((total - 1.0).abs() < f64::EPSILON);
    }
}
