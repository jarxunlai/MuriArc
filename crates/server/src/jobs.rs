use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use muriarc_core::{AuditContext, Job, JobFilter, MuriArcStore, StoreError};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct JobCreateOutcome {
    pub job: Job,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobRepositoryError {
    #[error("idempotency key is already used by a different job request")]
    IdempotencyConflict,
    #[error("job {0} was not found")]
    NotFound(Uuid),
    #[error("job repository is unavailable")]
    Unavailable,
}

#[async_trait]
pub trait JobRepository: Send + Sync {
    async fn create(
        &self,
        job: Job,
        audit: AuditContext,
    ) -> Result<JobCreateOutcome, JobRepositoryError>;

    async fn get(&self, id: Uuid) -> Result<Job, JobRepositoryError>;

    async fn list(&self, lab_id: Uuid) -> Result<Vec<Job>, JobRepositoryError>;

    async fn update(
        &self,
        job: Job,
        expected_revision: i64,
        audit: AuditContext,
    ) -> Result<(), JobRepositoryError>;
}

#[derive(Debug, Clone)]
struct StoredJob {
    job: Job,
    #[allow(dead_code)]
    audit: AuditContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyScope {
    lab_id: Uuid,
    user_id: Uuid,
    key: String,
}

#[derive(Debug, Default)]
struct MemoryState {
    jobs: HashMap<Uuid, StoredJob>,
    idempotency: HashMap<IdempotencyScope, Uuid>,
}

#[derive(Clone)]
pub struct StoreJobRepository {
    store: Arc<dyn MuriArcStore>,
}

impl StoreJobRepository {
    pub fn new(store: Arc<dyn MuriArcStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl JobRepository for StoreJobRepository {
    async fn create(
        &self,
        job: Job,
        audit: AuditContext,
    ) -> Result<JobCreateOutcome, JobRepositoryError> {
        if let Some(existing) = self
            .store
            .find_job_by_idempotency(job.lab_id, job.created_by, &job.idempotency_key)
            .await
            .map_err(map_store_error)?
        {
            return matching_outcome(existing, &job);
        }

        match self.store.create_job(&job, &audit).await {
            Ok(()) => Ok(JobCreateOutcome { job, created: true }),
            Err(StoreError::Conflict(_)) => {
                let existing = self
                    .store
                    .find_job_by_idempotency(job.lab_id, job.created_by, &job.idempotency_key)
                    .await
                    .map_err(map_store_error)?
                    .ok_or(JobRepositoryError::Unavailable)?;
                matching_outcome(existing, &job)
            }
            Err(error) => Err(map_store_error(error)),
        }
    }

    async fn get(&self, id: Uuid) -> Result<Job, JobRepositoryError> {
        self.store.get_job(id).await.map_err(map_store_error)
    }

    async fn list(&self, lab_id: Uuid) -> Result<Vec<Job>, JobRepositoryError> {
        self.store
            .list_jobs(&JobFilter {
                lab_id,
                project_id: None,
                created_by: None,
            })
            .await
            .map_err(map_store_error)
    }

    async fn update(
        &self,
        job: Job,
        expected_revision: i64,
        audit: AuditContext,
    ) -> Result<(), JobRepositoryError> {
        self.store
            .update_job(&job, expected_revision, &audit)
            .await
            .map_err(map_store_error)
    }
}

fn matching_outcome(
    existing: Job,
    requested: &Job,
) -> Result<JobCreateOutcome, JobRepositoryError> {
    if same_request(&existing, requested) {
        Ok(JobCreateOutcome {
            job: existing,
            created: false,
        })
    } else {
        Err(JobRepositoryError::IdempotencyConflict)
    }
}

fn map_store_error(error: StoreError) -> JobRepositoryError {
    match error {
        StoreError::NotFound { id, .. } => JobRepositoryError::NotFound(id),
        StoreError::Conflict(_) => JobRepositoryError::IdempotencyConflict,
        error => {
            tracing::error!(error = %error, "persistent job repository failed");
            JobRepositoryError::Unavailable
        }
    }
}

#[derive(Debug, Default)]
pub struct InMemoryJobRepository {
    state: RwLock<MemoryState>,
}

#[async_trait]
impl JobRepository for InMemoryJobRepository {
    async fn create(
        &self,
        job: Job,
        audit: AuditContext,
    ) -> Result<JobCreateOutcome, JobRepositoryError> {
        let scope = IdempotencyScope {
            lab_id: job.lab_id,
            user_id: job.created_by,
            key: job.idempotency_key.clone(),
        };
        let mut state = self.state.write().await;

        if let Some(existing_id) = state.idempotency.get(&scope) {
            let existing = state
                .jobs
                .get(existing_id)
                .ok_or(JobRepositoryError::Unavailable)?;
            if same_request(&existing.job, &job) {
                return Ok(JobCreateOutcome {
                    job: existing.job.clone(),
                    created: false,
                });
            }
            return Err(JobRepositoryError::IdempotencyConflict);
        }

        state.idempotency.insert(scope, job.id);
        state.jobs.insert(
            job.id,
            StoredJob {
                job: job.clone(),
                audit,
            },
        );
        Ok(JobCreateOutcome { job, created: true })
    }

    async fn get(&self, id: Uuid) -> Result<Job, JobRepositoryError> {
        self.state
            .read()
            .await
            .jobs
            .get(&id)
            .map(|stored| stored.job.clone())
            .ok_or(JobRepositoryError::NotFound(id))
    }

    async fn list(&self, lab_id: Uuid) -> Result<Vec<Job>, JobRepositoryError> {
        let mut jobs = self
            .state
            .read()
            .await
            .jobs
            .values()
            .filter(|stored| stored.job.lab_id == lab_id)
            .map(|stored| stored.job.clone())
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| job.meta.created_at);
        jobs.reverse();
        Ok(jobs)
    }

    async fn update(
        &self,
        job: Job,
        expected_revision: i64,
        audit: AuditContext,
    ) -> Result<(), JobRepositoryError> {
        let mut state = self.state.write().await;
        let stored = state
            .jobs
            .get_mut(&job.id)
            .ok_or(JobRepositoryError::NotFound(job.id))?;
        if stored.job.meta.revision != expected_revision
            || job.meta.revision != expected_revision + 1
        {
            return Err(JobRepositoryError::IdempotencyConflict);
        }
        stored.job = job;
        stored.audit = audit;
        Ok(())
    }
}

fn same_request(left: &Job, right: &Job) -> bool {
    left.lab_id == right.lab_id
        && left.project_id == right.project_id
        && left.created_by == right.created_by
        && left.kind == right.kind
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use muriarc_core::{Actor, JobKind, JobStatus, RecordMeta, WriteSource};

    use super::*;

    fn job(idempotency_key: &str, kind: JobKind) -> Job {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
        Job {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            project_id: Some(Uuid::new_v4()),
            created_by: Uuid::new_v4(),
            kind,
            status: JobStatus::Queued,
            idempotency_key: idempotency_key.into(),
            progress_current: 0,
            progress_total: None,
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        }
    }

    fn audit(user_id: Uuid) -> AuditContext {
        AuditContext {
            actor: Actor::human(user_id, "Researcher"),
            source: WriteSource::Web,
            request_id: Some("request-1".into()),
            reason: None,
        }
    }

    #[tokio::test]
    async fn exact_retry_returns_existing_job() {
        let repository = InMemoryJobRepository::default();
        let first = job("same-key", JobKind::Import);
        let mut retry = first.clone();
        retry.id = Uuid::new_v4();

        let created = repository
            .create(first.clone(), audit(first.created_by))
            .await
            .unwrap();
        let repeated = repository
            .create(retry, audit(first.created_by))
            .await
            .unwrap();

        assert!(created.created);
        assert!(!repeated.created);
        assert_eq!(repeated.job.id, first.id);
    }

    #[tokio::test]
    async fn reused_key_with_different_request_conflicts() {
        let repository = InMemoryJobRepository::default();
        let first = job("same-key", JobKind::Import);
        let mut conflicting = first.clone();
        conflicting.id = Uuid::new_v4();
        conflicting.kind = JobKind::Export;

        repository
            .create(first.clone(), audit(first.created_by))
            .await
            .unwrap();
        let error = repository
            .create(conflicting, audit(first.created_by))
            .await
            .unwrap_err();

        assert_eq!(error, JobRepositoryError::IdempotencyConflict);
    }
}
