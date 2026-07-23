use async_trait::async_trait;
use muriarc_core::{
    ActorType, AiModelProfile, AiModelProfileFilter, AiModelProfileStore, AiModelProfileVersion,
    AiUserModelDefaults, AuditAction, AuditContext, EntityType, StoreError, StoreResult,
    WriteSource,
};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use super::{PgTransaction, PostgresStore, encode, map_sqlx, meta, snapshot, write_audit};

const PROFILE_COLUMNS: &str = "id, lab_id, user_id, name, current_version, archived_at, created_at, updated_at, deleted_at, revision";
const DEFAULT_COLUMNS: &str = "user_id, default_conversation_profile_id, default_vision_profile_id, created_at, updated_at, deleted_at, revision";

fn profile_from_row(row: &PgRow) -> StoreResult<AiModelProfile> {
    Ok(AiModelProfile {
        id: row.try_get("id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        current_version: row.try_get("current_version").map_err(map_sqlx)?,
        archived_at: row.try_get("archived_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn defaults_from_row(row: &PgRow) -> StoreResult<AiUserModelDefaults> {
    Ok(AiUserModelDefaults {
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        default_conversation_profile_id: row
            .try_get("default_conversation_profile_id")
            .map_err(map_sqlx)?,
        default_vision_profile_id: row.try_get("default_vision_profile_id").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn checked_u32(value: i64, field: &'static str) -> StoreResult<u32> {
    u32::try_from(value)
        .map_err(|_| StoreError::Database(format!("invalid {field} in AI model profile version")))
}

fn checked_u64(value: i64, field: &'static str) -> StoreResult<u64> {
    u64::try_from(value)
        .map_err(|_| StoreError::Database(format!("invalid {field} in AI model profile version")))
}

fn version_from_row(row: &PgRow) -> StoreResult<AiModelProfileVersion> {
    let history_turns = row.try_get::<i32, _>("history_turns").map_err(map_sqlx)?;
    let temperature = row.try_get::<f64, _>("temperature").map_err(map_sqlx)?;
    Ok(AiModelProfileVersion {
        profile_id: row.try_get("profile_id").map_err(map_sqlx)?,
        version: row.try_get("version").map_err(map_sqlx)?,
        protocol: super::decode(row.try_get("protocol").map_err(map_sqlx)?)?,
        transport: super::decode(row.try_get("transport").map_err(map_sqlx)?)?,
        base_url: row.try_get("base_url").map_err(map_sqlx)?,
        normalized_base_url: row.try_get("normalized_base_url").map_err(map_sqlx)?,
        model_id: row.try_get("model_id").map_err(map_sqlx)?,
        supports_vision: row.try_get("supports_vision").map_err(map_sqlx)?,
        context_window_tokens: checked_u32(
            row.try_get("context_window_tokens").map_err(map_sqlx)?,
            "context_window_tokens",
        )?,
        max_input_tokens: checked_u32(
            row.try_get("max_input_tokens").map_err(map_sqlx)?,
            "max_input_tokens",
        )?,
        max_output_tokens: checked_u32(
            row.try_get("max_output_tokens").map_err(map_sqlx)?,
            "max_output_tokens",
        )?,
        history_token_budget: checked_u32(
            row.try_get("history_token_budget").map_err(map_sqlx)?,
            "history_token_budget",
        )?,
        history_turns: u32::try_from(history_turns).map_err(|_| {
            StoreError::Database("invalid history_turns in AI model profile version".to_owned())
        })?,
        temperature: temperature as f32,
        timeout_ms: checked_u64(row.try_get("timeout_ms").map_err(map_sqlx)?, "timeout_ms")?,
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
    })
}

fn validate_profile(profile: &AiModelProfile, version: &AiModelProfileVersion) -> StoreResult<()> {
    let name = profile.name.trim();
    let base_url = version.base_url.trim();
    let normalized_base_url = version.normalized_base_url.trim();
    let model_id = version.model_id.trim();
    let token_total = u64::from(version.max_input_tokens) + u64::from(version.max_output_tokens);
    if profile.id.is_nil()
        || profile.lab_id.is_nil()
        || profile.user_id.is_nil()
        || name.is_empty()
        || name.chars().count() > 120
        || profile.meta.deleted_at.is_some()
        || profile.meta.revision <= 0
        || profile.current_version != version.version
        || version.profile_id != profile.id
        || version.version <= 0
        || base_url.is_empty()
        || base_url.chars().count() > 2048
        || normalized_base_url.is_empty()
        || normalized_base_url.chars().count() > 2048
        || normalized_base_url != base_url.trim_end_matches('/')
        || model_id.is_empty()
        || model_id.chars().count() > 256
        || !(4_096..=2_000_000).contains(&version.context_window_tokens)
        || !(1_024..=1_900_000).contains(&version.max_input_tokens)
        || !(1..=131_072).contains(&version.max_output_tokens)
        || version.history_token_budget > 1_000_000
        || version.history_token_budget > version.max_input_tokens
        || version.history_turns > 100
        || !version.temperature.is_finite()
        || !(0.0..=2.0).contains(&version.temperature)
        || !(100..=600_000).contains(&version.timeout_ms)
        || token_total > u64::from(version.context_window_tokens)
    {
        return Err(StoreError::Validation(
            "invalid AI model profile".to_owned(),
        ));
    }
    Ok(())
}

fn validate_owner(user_id: Uuid, audit: &AuditContext) -> StoreResult<()> {
    let is_migration = audit.source == WriteSource::Migration
        && matches!(
            audit.actor.actor_type,
            ActorType::System | ActorType::Migration
        );
    if !is_migration && audit.actor.user_id != Some(user_id) {
        return Err(StoreError::Validation(
            "AI model profile actor must match its owner".to_owned(),
        ));
    }
    Ok(())
}

async fn active_user_lab(tx: &mut PgTransaction<'_>, user_id: Uuid) -> StoreResult<Uuid> {
    sqlx::query_scalar("SELECT lab_id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "user",
            id: user_id,
        })
}

async fn ensure_profile_can_be_default(
    tx: &mut PgTransaction<'_>,
    profile_id: Uuid,
    user_id: Uuid,
    lab_id: Uuid,
    requires_vision: bool,
) -> StoreResult<()> {
    let row = sqlx::query(
        "SELECT p.lab_id, p.user_id, p.archived_at, v.supports_vision
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = p.current_version
         WHERE p.id = $1 AND p.deleted_at IS NULL
         FOR SHARE OF p",
    )
    .bind(profile_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_model_profile",
        id: profile_id,
    })?;
    let actual_lab_id: Uuid = row.try_get("lab_id").map_err(map_sqlx)?;
    let actual_user_id: Uuid = row.try_get("user_id").map_err(map_sqlx)?;
    let archived_at: Option<chrono::DateTime<chrono::Utc>> =
        row.try_get("archived_at").map_err(map_sqlx)?;
    let supports_vision: bool = row.try_get("supports_vision").map_err(map_sqlx)?;
    if actual_lab_id != lab_id || actual_user_id != user_id {
        return Err(StoreError::Validation(
            "default AI model profile must belong to the same user and lab".to_owned(),
        ));
    }
    if archived_at.is_some() {
        return Err(StoreError::Validation(
            "archived AI model profile cannot be selected as a default".to_owned(),
        ));
    }
    if requires_vision && !supports_vision {
        return Err(StoreError::Validation(
            "default vision profile must support vision".to_owned(),
        ));
    }
    Ok(())
}

async fn insert_version(
    tx: &mut PgTransaction<'_>,
    value: &AiModelProfileVersion,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO ai_model_profile_versions (
            profile_id, version, protocol, transport, base_url, normalized_base_url, model_id,
            supports_vision, context_window_tokens, max_input_tokens, max_output_tokens,
            history_token_budget, history_turns, temperature, timeout_ms, created_at
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
    )
    .bind(value.profile_id)
    .bind(value.version)
    .bind(encode(&value.protocol)?)
    .bind(encode(&value.transport)?)
    .bind(&value.base_url)
    .bind(&value.normalized_base_url)
    .bind(&value.model_id)
    .bind(value.supports_vision)
    .bind(i64::from(value.context_window_tokens))
    .bind(i64::from(value.max_input_tokens))
    .bind(i64::from(value.max_output_tokens))
    .bind(i64::from(value.history_token_budget))
    .bind(i32::try_from(value.history_turns).map_err(|_| {
        StoreError::Validation("AI model profile history_turns is too large".to_owned())
    })?)
    .bind(f64::from(value.temperature))
    .bind(i64::try_from(value.timeout_ms).map_err(|_| {
        StoreError::Validation("AI model profile timeout_ms is too large".to_owned())
    })?)
    .bind(value.created_at)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

#[async_trait]
impl AiModelProfileStore for PostgresStore {
    async fn create_ai_model_profile(
        &self,
        value: &AiModelProfile,
        initial: &AiModelProfileVersion,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_profile(value, initial)?;
        validate_owner(value.user_id, audit)?;
        if initial.version != 1
            || value.current_version != 1
            || value.meta.revision != 1
            || value.archived_at.is_some()
        {
            return Err(StoreError::Validation(
                "initial AI model profile must be active at version and revision 1".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let user_lab_id = active_user_lab(&mut tx, value.user_id).await?;
        if user_lab_id != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile must belong to its user's lab".to_owned(),
            ));
        }
        sqlx::query(
            "INSERT INTO ai_model_profiles (
                id, lab_id, user_id, name, current_version, created_at, updated_at,
                archived_at, deleted_at, revision
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(value.id)
        .bind(value.lab_id)
        .bind(value.user_id)
        .bind(&value.name)
        .bind(value.current_version)
        .bind(value.meta.created_at)
        .bind(value.meta.updated_at)
        .bind(value.archived_at)
        .bind(value.meta.deleted_at)
        .bind(value.meta.revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        insert_version(&mut tx, initial).await?;
        write_audit(
            &mut tx,
            value.lab_id,
            None,
            EntityType::AiModelProfile,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_ai_model_profile(&self, id: Uuid) -> StoreResult<AiModelProfile> {
        let row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles
             WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id,
        })?;
        profile_from_row(&row)
    }

    async fn list_ai_model_profiles(
        &self,
        filter: &AiModelProfileFilter,
    ) -> StoreResult<Vec<AiModelProfile>> {
        let rows = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles
             WHERE lab_id = $1 AND user_id = $2 AND deleted_at IS NULL
               AND ($3 OR archived_at IS NULL)
             ORDER BY updated_at DESC, id"
        ))
        .bind(filter.lab_id)
        .bind(filter.user_id)
        .bind(filter.include_archived)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(profile_from_row).collect()
    }

    async fn get_ai_model_profile_version(
        &self,
        profile_id: Uuid,
        number: i64,
    ) -> StoreResult<AiModelProfileVersion> {
        let row = sqlx::query(
            "SELECT v.* FROM ai_model_profile_versions v
             JOIN ai_model_profiles p ON p.id = v.profile_id
             WHERE v.profile_id = $1 AND v.version = $2 AND p.deleted_at IS NULL",
        )
        .bind(profile_id)
        .bind(number)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: profile_id,
        })?;
        version_from_row(&row)
    }

    async fn append_ai_model_profile_version(
        &self,
        value: &AiModelProfile,
        next: &AiModelProfileVersion,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_profile(value, next)?;
        validate_owner(value.user_id, audit)?;
        if expected_revision <= 0 || value.meta.revision != expected_revision + 1 {
            return Err(StoreError::Validation(
                "AI model profile revision must advance exactly once".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let user_lab_id = active_user_lab(&mut tx, value.user_id).await?;
        if user_lab_id != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile must remain in its user's lab".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles
             WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(value.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: value.id,
        })?;
        let before = profile_from_row(&before_row)?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "AI model profile changed concurrently".to_owned(),
            ));
        }
        if before.lab_id != value.lab_id
            || before.user_id != value.user_id
            || before.meta.created_at != value.meta.created_at
            || before.meta.deleted_at != value.meta.deleted_at
            || before.archived_at != value.archived_at
        {
            return Err(StoreError::Validation(
                "AI model profile ownership, identity, and archive state are immutable while appending a version"
                    .to_owned(),
            ));
        }
        if before.archived_at.is_some() {
            return Err(StoreError::Conflict(
                "archived AI model profile cannot receive new versions".to_owned(),
            ));
        }
        if next.version != before.current_version + 1 {
            return Err(StoreError::Conflict(
                "AI model profile version changed concurrently".to_owned(),
            ));
        }

        insert_version(&mut tx, next).await?;
        let updated = sqlx::query(
            "UPDATE ai_model_profiles
             SET name = $1, current_version = $2, updated_at = $3, revision = $4
             WHERE id = $5 AND revision = $6 AND archived_at IS NULL
               AND deleted_at IS NULL",
        )
        .bind(&value.name)
        .bind(value.current_version)
        .bind(value.meta.updated_at)
        .bind(value.meta.revision)
        .bind(value.id)
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI model profile changed before the update was applied".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            value.lab_id,
            None,
            EntityType::AiModelProfile,
            value.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn archive_ai_model_profile(
        &self,
        value: &AiModelProfile,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_owner(value.user_id, audit)?;
        if value.id.is_nil()
            || value.lab_id.is_nil()
            || value.user_id.is_nil()
            || value.archived_at.is_none()
            || value.meta.deleted_at.is_some()
            || expected_revision <= 0
            || value.meta.revision != expected_revision + 1
        {
            return Err(StoreError::Validation(
                "invalid archived AI model profile revision".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let user_lab_id = active_user_lab(&mut tx, value.user_id).await?;
        if user_lab_id != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile must remain in its user's lab".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles
             WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(value.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: value.id,
        })?;
        let before = profile_from_row(&before_row)?;
        if before.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "AI model profile changed concurrently".to_owned(),
            ));
        }
        if before.archived_at.is_some() {
            return Err(StoreError::Conflict(
                "AI model profile is already archived".to_owned(),
            ));
        }
        if before.lab_id != value.lab_id
            || before.user_id != value.user_id
            || before.name != value.name
            || before.current_version != value.current_version
            || before.meta.created_at != value.meta.created_at
            || before.meta.deleted_at != value.meta.deleted_at
        {
            return Err(StoreError::Validation(
                "archiving cannot change AI model profile identity or configuration".to_owned(),
            ));
        }

        let updated = sqlx::query(
            "UPDATE ai_model_profiles
             SET archived_at = $1, updated_at = $2, revision = $3
             WHERE id = $4 AND revision = $5 AND archived_at IS NULL
               AND deleted_at IS NULL",
        )
        .bind(value.archived_at)
        .bind(value.meta.updated_at)
        .bind(value.meta.revision)
        .bind(value.id)
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI model profile changed before it was archived".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            value.lab_id,
            None,
            EntityType::AiModelProfile,
            value.id,
            AuditAction::Archive,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn save_ai_user_model_defaults(
        &self,
        value: &AiUserModelDefaults,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_owner(value.user_id, audit)?;
        if value.user_id.is_nil()
            || value.meta.deleted_at.is_some()
            || value.meta.revision != expected_revision.unwrap_or(0) + 1
        {
            return Err(StoreError::Validation(
                "invalid AI user model defaults revision".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let lab_id = active_user_lab(&mut tx, value.user_id).await?;
        if let Some(profile_id) = value.default_conversation_profile_id {
            ensure_profile_can_be_default(&mut tx, profile_id, value.user_id, lab_id, false)
                .await?;
        }
        if let Some(profile_id) = value.default_vision_profile_id {
            ensure_profile_can_be_default(&mut tx, profile_id, value.user_id, lab_id, true).await?;
        }

        let before_row = sqlx::query(&format!(
            "SELECT {DEFAULT_COLUMNS} FROM ai_user_model_defaults
             WHERE user_id = $1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(value.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let before = before_row.as_ref().map(defaults_from_row).transpose()?;

        match (&before, expected_revision) {
            (None, None) => {
                if value.meta.revision != 1 {
                    return Err(StoreError::Validation(
                        "initial AI user model defaults revision must be 1".to_owned(),
                    ));
                }
                sqlx::query(
                    "INSERT INTO ai_user_model_defaults (
                        user_id, default_conversation_profile_id, default_vision_profile_id,
                        created_at, updated_at, deleted_at, revision
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
                )
                .bind(value.user_id)
                .bind(value.default_conversation_profile_id)
                .bind(value.default_vision_profile_id)
                .bind(value.meta.created_at)
                .bind(value.meta.updated_at)
                .bind(value.meta.deleted_at)
                .bind(value.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            }
            (Some(current), Some(expected)) => {
                if current.meta.revision != expected {
                    return Err(StoreError::Conflict(
                        "AI user model defaults changed concurrently".to_owned(),
                    ));
                }
                if current.meta.created_at != value.meta.created_at {
                    return Err(StoreError::Validation(
                        "AI user model defaults creation time is immutable".to_owned(),
                    ));
                }
                let updated = sqlx::query(
                    "UPDATE ai_user_model_defaults
                     SET default_conversation_profile_id = $1,
                         default_vision_profile_id = $2, updated_at = $3,
                         revision = $4
                     WHERE user_id = $5 AND revision = $6 AND deleted_at IS NULL",
                )
                .bind(value.default_conversation_profile_id)
                .bind(value.default_vision_profile_id)
                .bind(value.meta.updated_at)
                .bind(value.meta.revision)
                .bind(value.user_id)
                .bind(expected)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
                if updated.rows_affected() != 1 {
                    return Err(StoreError::Conflict(
                        "AI user model defaults changed before the update was applied".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(StoreError::Conflict(
                    "AI user model defaults changed concurrently".to_owned(),
                ));
            }
        }

        write_audit(
            &mut tx,
            lab_id,
            None,
            EntityType::AiUserModelDefaults,
            value.user_id,
            if before.is_some() {
                AuditAction::Update
            } else {
                AuditAction::Create
            },
            audit,
            before.as_ref().map(snapshot).transpose()?,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_ai_user_model_defaults(
        &self,
        user_id: Uuid,
    ) -> StoreResult<Option<AiUserModelDefaults>> {
        let row = sqlx::query(&format!(
            "SELECT {DEFAULT_COLUMNS} FROM ai_user_model_defaults
             WHERE user_id = $1 AND deleted_at IS NULL"
        ))
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(defaults_from_row).transpose()
    }
}
