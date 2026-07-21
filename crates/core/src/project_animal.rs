use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RecordMeta;

/// Explicitly grants one project access to an animal in the lab registry.
///
/// Experiment participation is intentionally separate: an animal may be
/// available to a project before enrollment, and completed experiment history
/// must not continue to define authorization implicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAnimalAssignment {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub project_id: Uuid,
    pub animal_id: Uuid,
    pub assigned_by: Option<Uuid>,
    pub reason: Option<String>,
    pub meta: RecordMeta,
}

impl ProjectAnimalAssignment {
    pub fn new(
        lab_id: Uuid,
        project_id: Uuid,
        animal_id: Uuid,
        assigned_by: Option<Uuid>,
        reason: Option<String>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            lab_id,
            project_id,
            animal_id,
            assigned_by,
            reason,
            meta: RecordMeta::new(now),
        }
    }

    pub fn soft_delete(&mut self, now: DateTime<Utc>) {
        if self.meta.deleted_at.is_none() {
            self.meta.soft_delete(now);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAnimalAssignmentRemoval {
    pub assignment_id: Uuid,
    pub expected_revision: i64,
}
