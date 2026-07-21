use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Animal;

/// Project membership attached to an animal through an experiment
/// participation. This is deliberately a compact read model rather than a
/// second source of truth for projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalProjectRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatestAnimalWeight {
    pub value: f64,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
}

/// Bounded, list-friendly animal projection shared by SQLite and PostgreSQL.
///
/// Store adapters populate this projection with a fixed number of batched
/// queries. HTTP and desktop callers must not enrich a page one animal at a
/// time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimalOverview {
    pub animal: Animal,
    pub genotype_labels: Vec<String>,
    pub projects: Vec<AnimalProjectRef>,
    pub latest_weight: Option<LatestAnimalWeight>,
}
