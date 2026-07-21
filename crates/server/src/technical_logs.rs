use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_core::{ActorType, AuditContext, WriteSource};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_MAX_ROWS: i64 = 20_000;
const DEFAULT_MIN_RETENTION_DAYS: i32 = 30;

#[derive(Debug, Clone)]
pub struct TechnicalLogEvent {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub user_id: Option<Uuid>,
    pub request_id: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalLogPolicyView {
    pub max_rows: i64,
    pub min_retention_days: i32,
    pub revision: i64,
}

impl Default for TechnicalLogPolicyView {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_MAX_ROWS,
            min_retention_days: DEFAULT_MIN_RETENTION_DAYS,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalLogCleanupPreview {
    pub total_rows: i64,
    pub eligible_rows: i64,
    pub cutoff: DateTime<Utc>,
    pub policy_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveTechnicalLogPolicyInput {
    pub max_rows: i64,
    pub min_retention_days: i32,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TechnicalLogError {
    #[error("technical log settings are invalid")]
    Validation,
    #[error("technical log policy changed before cleanup")]
    Conflict,
    #[error("technical log storage is unavailable")]
    Unavailable,
}

fn validate_policy_input(input: &SaveTechnicalLogPolicyInput) -> Result<(), TechnicalLogError> {
    if !(1_000..=1_000_000).contains(&input.max_rows)
        || !(1..=3_650).contains(&input.min_retention_days)
        || input.expected_revision < 0
    {
        Err(TechnicalLogError::Validation)
    } else {
        Ok(())
    }
}

#[async_trait]
pub trait TechnicalLogService: Send + Sync {
    async fn record(&self, event: TechnicalLogEvent) -> Result<(), TechnicalLogError>;
    async fn get_policy(&self, lab_id: Uuid) -> Result<TechnicalLogPolicyView, TechnicalLogError>;
    async fn save_policy(
        &self,
        lab_id: Uuid,
        input: SaveTechnicalLogPolicyInput,
        audit: &AuditContext,
    ) -> Result<TechnicalLogPolicyView, TechnicalLogError>;
    async fn preview_cleanup(
        &self,
        lab_id: Uuid,
    ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError>;
    async fn cleanup(
        &self,
        lab_id: Uuid,
        expected_policy_revision: i64,
        expected_eligible_rows: i64,
        audit: &AuditContext,
    ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError>;
}

#[derive(Debug, Default)]
pub struct DisabledTechnicalLogService;

#[async_trait]
impl TechnicalLogService for DisabledTechnicalLogService {
    async fn record(&self, _event: TechnicalLogEvent) -> Result<(), TechnicalLogError> {
        Ok(())
    }

    async fn get_policy(&self, _lab_id: Uuid) -> Result<TechnicalLogPolicyView, TechnicalLogError> {
        Err(TechnicalLogError::Unavailable)
    }

    async fn save_policy(
        &self,
        _lab_id: Uuid,
        _input: SaveTechnicalLogPolicyInput,
        _audit: &AuditContext,
    ) -> Result<TechnicalLogPolicyView, TechnicalLogError> {
        Err(TechnicalLogError::Unavailable)
    }

    async fn preview_cleanup(
        &self,
        _lab_id: Uuid,
    ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError> {
        Err(TechnicalLogError::Unavailable)
    }

    async fn cleanup(
        &self,
        _lab_id: Uuid,
        _expected_policy_revision: i64,
        _expected_eligible_rows: i64,
        _audit: &AuditContext,
    ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError> {
        Err(TechnicalLogError::Unavailable)
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use muriarc_store_postgres::PostgresStore;
    use serde_json::json;
    use sqlx::{Postgres, Row, Transaction};

    use super::*;

    #[derive(Debug, Clone)]
    pub struct PostgresTechnicalLogService {
        store: PostgresStore,
    }

    impl PostgresTechnicalLogService {
        pub fn new(store: PostgresStore) -> Self {
            Self { store }
        }

        async fn preview_in(
            &self,
            transaction: &mut Transaction<'_, Postgres>,
            lab_id: Uuid,
        ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError> {
            let policy = sqlx::query(
                "SELECT max_rows, min_retention_days, revision FROM technical_log_policies WHERE lab_id = $1",
            )
            .bind(lab_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| TechnicalLogError::Unavailable)?;
            let (max_rows, min_retention_days, revision) =
                policy.map_or((DEFAULT_MAX_ROWS, DEFAULT_MIN_RETENTION_DAYS, 0), |row| {
                    (
                        row.try_get("max_rows").unwrap_or(DEFAULT_MAX_ROWS),
                        row.try_get("min_retention_days")
                            .unwrap_or(DEFAULT_MIN_RETENTION_DAYS),
                        row.try_get("revision").unwrap_or(0),
                    )
                });
            let cutoff = Utc::now() - chrono::Duration::days(i64::from(min_retention_days));
            let total_rows: i64 = sqlx::query_scalar(
                "SELECT count(*)::bigint FROM technical_log_events WHERE lab_id = $1",
            )
            .bind(lab_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| TechnicalLogError::Unavailable)?;
            let eligible_rows: i64 = sqlx::query_scalar(
                "WITH ranked AS (SELECT occurred_at, row_number() OVER (ORDER BY occurred_at DESC, id DESC) AS row_number FROM technical_log_events WHERE lab_id = $1) SELECT count(*)::bigint FROM ranked WHERE row_number > $2 AND occurred_at < $3",
            )
            .bind(lab_id)
            .bind(max_rows)
            .bind(cutoff)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| TechnicalLogError::Unavailable)?;
            Ok(TechnicalLogCleanupPreview {
                total_rows,
                eligible_rows,
                cutoff,
                policy_revision: revision,
            })
        }

        async fn write_cleanup_audit(
            transaction: &mut Transaction<'_, Postgres>,
            lab_id: Uuid,
            deleted_rows: i64,
            audit: Option<&AuditContext>,
        ) -> Result<(), TechnicalLogError> {
            let (actor_type, actor_user_id, actor_display_name, source, request_id, reason) =
                match audit {
                    Some(audit) => (
                        actor_type_name(audit.actor.actor_type),
                        audit.actor.user_id,
                        audit.actor.display_name.as_str(),
                        source_name(audit.source),
                        audit.request_id.as_deref(),
                        audit.reason.as_deref(),
                    ),
                    None => (
                        "system",
                        None,
                        "Technical log retention",
                        "system",
                        None,
                        Some("automatic technical log retention"),
                    ),
                };
            sqlx::query("INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1,$2,NULL,'technical_log_policy',$2,'cleanup',$3,$4,$5,$6,$7,$8,NULL,$9,now())")
                .bind(Uuid::new_v4()).bind(lab_id).bind(actor_type).bind(actor_user_id)
                .bind(actor_display_name).bind(source).bind(request_id).bind(reason)
                .bind(json!({ "deletedRows": deleted_rows }))
                .execute(&mut **transaction).await.map_err(|_| TechnicalLogError::Unavailable)?;
            Ok(())
        }

        async fn delete_eligible(
            transaction: &mut Transaction<'_, Postgres>,
            lab_id: Uuid,
            cutoff: DateTime<Utc>,
        ) -> Result<i64, TechnicalLogError> {
            let deleted = sqlx::query("WITH ranked AS (SELECT id, occurred_at, row_number() OVER (ORDER BY occurred_at DESC, id DESC) AS row_number FROM technical_log_events WHERE lab_id = $1), eligible AS (SELECT id FROM ranked WHERE row_number > COALESCE((SELECT max_rows FROM technical_log_policies WHERE lab_id = $1), 20000) AND occurred_at < $2) DELETE FROM technical_log_events t USING eligible e WHERE t.id = e.id")
                .bind(lab_id).bind(cutoff).execute(&mut **transaction).await
                .map_err(|_| TechnicalLogError::Unavailable)?.rows_affected();
            Ok(i64::try_from(deleted).unwrap_or(i64::MAX))
        }
    }

    #[async_trait]
    impl TechnicalLogService for PostgresTechnicalLogService {
        async fn record(&self, event: TechnicalLogEvent) -> Result<(), TechnicalLogError> {
            let mut transaction = self
                .store
                .pool()
                .begin()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            sqlx::query("INSERT INTO technical_log_events (id, lab_id, user_id, request_id, method, path, status_code, occurred_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(event.id).bind(event.lab_id).bind(event.user_id).bind(event.request_id)
                .bind(event.method).bind(event.path).bind(i32::from(event.status_code)).bind(event.occurred_at)
                .execute(&mut *transaction).await.map_err(|_| TechnicalLogError::Unavailable)?;
            let preview = self.preview_in(&mut transaction, event.lab_id).await?;
            if preview.eligible_rows > 0 {
                let deleted =
                    Self::delete_eligible(&mut transaction, event.lab_id, preview.cutoff).await?;
                if deleted > 0 {
                    Self::write_cleanup_audit(&mut transaction, event.lab_id, deleted, None)
                        .await?;
                }
            }
            transaction
                .commit()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)
        }

        async fn get_policy(
            &self,
            lab_id: Uuid,
        ) -> Result<TechnicalLogPolicyView, TechnicalLogError> {
            let row = sqlx::query("SELECT max_rows, min_retention_days, revision FROM technical_log_policies WHERE lab_id = $1")
                .bind(lab_id).fetch_optional(self.store.pool()).await.map_err(|_| TechnicalLogError::Unavailable)?;
            Ok(row.map_or_else(TechnicalLogPolicyView::default, |row| {
                TechnicalLogPolicyView {
                    max_rows: row.try_get("max_rows").unwrap_or(DEFAULT_MAX_ROWS),
                    min_retention_days: row
                        .try_get("min_retention_days")
                        .unwrap_or(DEFAULT_MIN_RETENTION_DAYS),
                    revision: row.try_get("revision").unwrap_or(0),
                }
            }))
        }

        async fn save_policy(
            &self,
            lab_id: Uuid,
            input: SaveTechnicalLogPolicyInput,
            audit: &AuditContext,
        ) -> Result<TechnicalLogPolicyView, TechnicalLogError> {
            validate_policy_input(&input)?;
            if audit.actor.actor_type != ActorType::Human || audit.actor.user_id.is_none() {
                return Err(TechnicalLogError::Validation);
            }
            let actor = audit.actor.user_id.ok_or(TechnicalLogError::Validation)?;
            let mut transaction = self
                .store
                .pool()
                .begin()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            let current: Option<i64> = sqlx::query_scalar(
                "SELECT revision FROM technical_log_policies WHERE lab_id = $1 FOR UPDATE",
            )
            .bind(lab_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| TechnicalLogError::Unavailable)?;
            if current.unwrap_or(0) != input.expected_revision {
                return Err(TechnicalLogError::Conflict);
            }
            let revision = input.expected_revision + 1;
            sqlx::query("INSERT INTO technical_log_policies (lab_id, max_rows, min_retention_days, updated_by, created_at, updated_at, revision) VALUES ($1,$2,$3,$4,now(),now(),1) ON CONFLICT (lab_id) DO UPDATE SET max_rows = EXCLUDED.max_rows, min_retention_days = EXCLUDED.min_retention_days, updated_by = EXCLUDED.updated_by, updated_at = now(), revision = technical_log_policies.revision + 1")
                .bind(lab_id).bind(input.max_rows).bind(input.min_retention_days).bind(actor)
                .execute(&mut *transaction).await.map_err(|_| TechnicalLogError::Unavailable)?;
            sqlx::query("INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1,$2,NULL,'technical_log_policy',$2,'update','human',$3,$4,$5,$6,$7,NULL,$8,now())")
                .bind(Uuid::new_v4()).bind(lab_id).bind(actor).bind(&audit.actor.display_name)
                .bind(source_name(audit.source)).bind(&audit.request_id).bind(&audit.reason)
                .bind(json!({ "maxRows": input.max_rows, "minRetentionDays": input.min_retention_days, "revision": revision }))
                .execute(&mut *transaction).await.map_err(|_| TechnicalLogError::Unavailable)?;
            transaction
                .commit()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            Ok(TechnicalLogPolicyView {
                max_rows: input.max_rows,
                min_retention_days: input.min_retention_days,
                revision,
            })
        }

        async fn preview_cleanup(
            &self,
            lab_id: Uuid,
        ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError> {
            let mut transaction = self
                .store
                .pool()
                .begin()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            self.preview_in(&mut transaction, lab_id).await
        }

        async fn cleanup(
            &self,
            lab_id: Uuid,
            expected_policy_revision: i64,
            expected_eligible_rows: i64,
            audit: &AuditContext,
        ) -> Result<TechnicalLogCleanupPreview, TechnicalLogError> {
            if audit.actor.actor_type != ActorType::Human
                || audit.actor.user_id.is_none()
                || expected_policy_revision < 0
                || expected_eligible_rows < 0
            {
                return Err(TechnicalLogError::Validation);
            }
            let mut transaction = self
                .store
                .pool()
                .begin()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            let preview = self.preview_in(&mut transaction, lab_id).await?;
            if preview.policy_revision != expected_policy_revision
                || preview.eligible_rows != expected_eligible_rows
            {
                return Err(TechnicalLogError::Conflict);
            }
            let deleted = Self::delete_eligible(&mut transaction, lab_id, preview.cutoff).await?;
            if deleted > 0 {
                Self::write_cleanup_audit(&mut transaction, lab_id, deleted, Some(audit)).await?;
            }
            transaction
                .commit()
                .await
                .map_err(|_| TechnicalLogError::Unavailable)?;
            self.preview_cleanup(lab_id).await
        }
    }

    fn actor_type_name(value: ActorType) -> &'static str {
        match value {
            ActorType::Human => "human",
            ActorType::Ai => "ai",
            ActorType::System => "system",
            ActorType::Migration => "migration",
        }
    }

    fn source_name(value: WriteSource) -> &'static str {
        match value {
            WriteSource::Desktop => "desktop",
            WriteSource::Web => "web",
            WriteSource::Api => "api",
            WriteSource::Ai => "ai",
            WriteSource::Migration => "migration",
            WriteSource::Mcp => "mcp",
        }
    }

    pub use PostgresTechnicalLogService as Service;
}

#[cfg(feature = "postgres")]
pub use postgres::Service as PostgresTechnicalLogService;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_defaults_and_bounds_are_explicit() {
        assert_eq!(TechnicalLogPolicyView::default().max_rows, 20_000);
        assert_eq!(TechnicalLogPolicyView::default().min_retention_days, 30);
        assert!(
            validate_policy_input(&SaveTechnicalLogPolicyInput {
                max_rows: 1_000,
                min_retention_days: 1,
                expected_revision: 0,
            })
            .is_ok()
        );
        assert!(
            validate_policy_input(&SaveTechnicalLogPolicyInput {
                max_rows: 999,
                min_retention_days: 30,
                expected_revision: 0,
            })
            .is_err()
        );
        assert!(
            validate_policy_input(&SaveTechnicalLogPolicyInput {
                max_rows: 20_000,
                min_retention_days: 0,
                expected_revision: 0,
            })
            .is_err()
        );
    }
}
