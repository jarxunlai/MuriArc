use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnimalDirectory {
    by_display_id: BTreeMap<String, BTreeSet<Uuid>>,
    by_animal_id: BTreeMap<Uuid, BTreeSet<String>>,
}

impl AnimalDirectory {
    pub fn from_entries<S>(
        entries: impl IntoIterator<Item = (S, Uuid)>,
    ) -> Result<Self, CatalogError>
    where
        S: Into<String>,
    {
        let mut directory = Self::default();
        for (display_id, animal_id) in entries {
            directory.insert(display_id, animal_id)?;
        }
        Ok(directory)
    }

    pub fn insert(
        &mut self,
        display_id: impl Into<String>,
        animal_id: Uuid,
    ) -> Result<(), CatalogError> {
        let display_id = display_id.into().trim().to_owned();
        if display_id.is_empty() {
            return Err(CatalogError::EmptyAnimalDisplayId);
        }
        self.by_display_id
            .entry(display_id.clone())
            .or_default()
            .insert(animal_id);
        self.by_animal_id
            .entry(animal_id)
            .or_default()
            .insert(display_id);
        Ok(())
    }

    pub fn resolve(&self, display_id: &str) -> AnimalResolution {
        match self.by_display_id.get(display_id.trim()) {
            None => AnimalResolution::Unknown,
            Some(ids) if ids.len() == 1 => {
                AnimalResolution::Unique(*ids.first().expect("one id is present"))
            }
            Some(_) => AnimalResolution::Ambiguous,
        }
    }

    pub fn contains(&self, display_id: &str) -> bool {
        self.by_display_id.contains_key(display_id.trim())
    }

    pub fn contains_id(&self, animal_id: Uuid) -> bool {
        self.by_animal_id.contains_key(&animal_id)
    }

    pub fn display_id(&self, animal_id: Uuid) -> Option<&str> {
        self.by_animal_id
            .get(&animal_id)
            .and_then(|display_ids| display_ids.first())
            .map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.by_display_id.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimalResolution {
    Unknown,
    Unique(Uuid),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementValueType {
    Number,
    Text,
    Boolean,
    Date,
    Category,
}

impl MeasurementValueType {
    pub fn parse_label(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "number" | "numeric" | "float" | "decimal" => Some(Self::Number),
            "text" | "string" => Some(Self::Text),
            "boolean" | "bool" => Some(Self::Boolean),
            "date" => Some(Self::Date),
            "category" | "categorical" => Some(Self::Category),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementDefinition {
    key: String,
    value_type: MeasurementValueType,
    allowed_units: BTreeSet<String>,
    unit_required: bool,
}

impl MeasurementDefinition {
    pub fn new<S>(
        key: impl Into<String>,
        value_type: MeasurementValueType,
        allowed_units: impl IntoIterator<Item = S>,
        unit_required: bool,
    ) -> Result<Self, CatalogError>
    where
        S: Into<String>,
    {
        let key = key.into().trim().to_owned();
        if key.is_empty() {
            return Err(CatalogError::EmptyMeasurementKey);
        }
        let mut units = BTreeSet::new();
        for unit in allowed_units {
            let unit = unit.into().trim().to_owned();
            if unit.is_empty() {
                return Err(CatalogError::EmptyUnit);
            }
            units.insert(unit);
        }
        if unit_required && units.is_empty() {
            return Err(CatalogError::RequiredUnitSetIsEmpty);
        }
        Ok(Self {
            key,
            value_type,
            allowed_units: units,
            unit_required,
        })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub const fn value_type(&self) -> MeasurementValueType {
        self.value_type
    }

    pub fn allowed_units(&self) -> &BTreeSet<String> {
        &self.allowed_units
    }

    pub const fn unit_required(&self) -> bool {
        self.unit_required
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeasurementCatalog {
    definitions: BTreeMap<String, MeasurementDefinition>,
}

impl MeasurementCatalog {
    pub fn new(
        definitions: impl IntoIterator<Item = MeasurementDefinition>,
    ) -> Result<Self, CatalogError> {
        let mut by_key = BTreeMap::new();
        for definition in definitions {
            let key = definition.key.clone();
            if by_key.insert(key.clone(), definition).is_some() {
                return Err(CatalogError::DuplicateMeasurementKey(key));
            }
        }
        Ok(Self {
            definitions: by_key,
        })
    }

    pub fn get(&self, key: &str) -> Option<&MeasurementDefinition> {
        self.definitions.get(key.trim())
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("animal display identifier must not be empty")]
    EmptyAnimalDisplayId,
    #[error("measurement key must not be empty")]
    EmptyMeasurementKey,
    #[error("measurement catalog contains a duplicate key: {0}")]
    DuplicateMeasurementKey(String),
    #[error("unit names must not be empty")]
    EmptyUnit,
    #[error("a unit-required measurement must define at least one allowed unit")]
    RequiredUnitSetIsEmpty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_display_ids_are_explicitly_ambiguous() {
        let directory =
            AnimalDirectory::from_entries([("M001", Uuid::new_v4()), ("M001", Uuid::new_v4())])
                .unwrap();
        assert_eq!(directory.resolve("M001"), AnimalResolution::Ambiguous);
    }

    #[test]
    fn required_units_need_an_allowlist() {
        assert!(matches!(
            MeasurementDefinition::new(
                "weight",
                MeasurementValueType::Number,
                Vec::<String>::new(),
                true
            ),
            Err(CatalogError::RequiredUnitSetIsEmpty)
        ));
    }
}
