use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, RecordMeta, require_non_empty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenotypeComponentMode {
    Diploid,
    Hemizygous,
    TransgenePresence,
    Conditional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypeComponent {
    pub id: Uuid,
    pub genotype_definition_id: Uuid,
    pub locus_id: Uuid,
    pub allele_1_id: Uuid,
    pub allele_2_id: Option<Uuid>,
    pub mode: GenotypeComponentMode,
    pub display_order: i32,
    pub meta: RecordMeta,
}

impl GenotypeComponent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        genotype_definition_id: Uuid,
        locus_id: Uuid,
        allele_1_id: Uuid,
        allele_2_id: Option<Uuid>,
        mode: GenotypeComponentMode,
        display_order: i32,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let component = Self {
            id: Uuid::new_v4(),
            genotype_definition_id,
            locus_id,
            allele_1_id,
            allele_2_id,
            mode,
            display_order,
            meta: RecordMeta::new(now),
        };
        component.validate()?;
        Ok(component)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let valid = !self.id.is_nil()
            && !self.genotype_definition_id.is_nil()
            && !self.locus_id.is_nil()
            && !self.allele_1_id.is_nil()
            && self.display_order >= 0
            && match self.mode {
                GenotypeComponentMode::Diploid | GenotypeComponentMode::Conditional => {
                    self.allele_2_id.is_some_and(|id| !id.is_nil())
                }
                GenotypeComponentMode::Hemizygous | GenotypeComponentMode::TransgenePresence => {
                    self.allele_2_id.is_none()
                }
            };
        if valid {
            Ok(())
        } else {
            Err(DomainError::InvalidGenotypeComponent)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypeDefinition {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub components: Vec<GenotypeComponent>,
    pub meta: RecordMeta,
}

impl GenotypeDefinition {
    pub fn new(
        lab_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("genotype_definition.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            name: name.trim().to_owned(),
            description: None,
            components: Vec::new(),
            meta: RecordMeta::new(now),
        })
    }

    pub fn replace_components(
        &mut self,
        components: Vec<GenotypeComponent>,
    ) -> Result<(), DomainError> {
        if components.is_empty()
            || components
                .iter()
                .any(|component| component.genotype_definition_id != self.id)
        {
            return Err(DomainError::InvalidGenotypeDefinition);
        }
        let mut ids = HashSet::with_capacity(components.len());
        let mut loci = HashSet::with_capacity(components.len());
        let mut display_orders = HashSet::with_capacity(components.len());
        for component in &components {
            component.validate()?;
            if !ids.insert(component.id)
                || !loci.insert(component.locus_id)
                || !display_orders.insert(component.display_order)
            {
                return Err(DomainError::InvalidGenotypeDefinition);
            }
        }
        self.components = components;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenotypingState {
    Unknown,
    Expected,
    Confirmed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingRecord {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub genotype_definition_id: Uuid,
    pub state: GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub meta: RecordMeta,
}

impl GenotypingRecord {
    pub fn new(
        lab_id: Uuid,
        animal_id: Uuid,
        genotype_definition_id: Uuid,
        state: GenotypingState,
        assessed_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let record = Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id: None,
            animal_id,
            genotype_definition_id,
            state,
            assessed_at,
            method: None,
            notes: None,
            meta: RecordMeta::new(now),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.lab_id.is_nil()
            || self.animal_id.is_nil()
            || self.genotype_definition_id.is_nil()
            || (matches!(
                self.state,
                GenotypingState::Confirmed | GenotypingState::Rejected
            ) && self.assessed_at.is_none())
        {
            Err(DomainError::InvalidGenotypingRecord)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_gene_definition_requires_explicit_valid_components() {
        let now = Utc::now();
        let lab_id = Uuid::new_v4();
        let mut definition = GenotypeDefinition::new(lab_id, "Cre conditional", now).unwrap();
        let first = GenotypeComponent::new(
            definition.id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            GenotypeComponentMode::Conditional,
            0,
            now,
        )
        .unwrap();
        let second = GenotypeComponent::new(
            definition.id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            GenotypeComponentMode::TransgenePresence,
            1,
            now,
        )
        .unwrap();
        definition.replace_components(vec![first, second]).unwrap();
        assert_eq!(definition.components.len(), 2);
    }

    #[test]
    fn confirmed_test_requires_assessment_time() {
        let error = GenotypingRecord::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            GenotypingState::Confirmed,
            None,
            Utc::now(),
        )
        .unwrap_err();
        assert_eq!(error, DomainError::InvalidGenotypingRecord);
    }
}
