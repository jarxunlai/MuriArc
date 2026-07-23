use async_trait::async_trait;
use muriarc_core::{
    ActorType, AiModelCredentialState, AiModelProfile, AiModelProfileFilter,
    AiModelProfileSecretRef, AiModelProfileSecretRefStore, AiModelProfileStore,
    AiModelProfileVersion, AiUserModelDefaults, AuditAction, AuditContext, EntityType, StoreError,
    StoreResult, WriteSource,
};
use sqlx::{Row, sqlite::SqliteRow};
use uuid::Uuid;

use super::{SqliteStore, map_sqlx, meta, optional_uuid, snapshot, uuid, write_audit};

const PROFILE_COLUMNS: &str = "id, lab_id, user_id, name, current_version, archived_at, created_at, updated_at, deleted_at, revision";

fn profile(row: &SqliteRow) -> StoreResult<AiModelProfile> {
    Ok(AiModelProfile {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        name: row.try_get("name").map_err(map_sqlx)?,
        current_version: row.try_get("current_version").map_err(map_sqlx)?,
        archived_at: row.try_get("archived_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn defaults(row: &SqliteRow) -> StoreResult<AiUserModelDefaults> {
    Ok(AiUserModelDefaults {
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        default_conversation_profile_id: optional_uuid(
            row.try_get("default_conversation_profile_id")
                .map_err(map_sqlx)?,
        )?,
        default_vision_profile_id: optional_uuid(
            row.try_get("default_vision_profile_id").map_err(map_sqlx)?,
        )?,
        meta: meta(row)?,
    })
}

fn version(row: &SqliteRow) -> StoreResult<AiModelProfileVersion> {
    Ok(AiModelProfileVersion {
        profile_id: uuid(row.try_get("profile_id").map_err(map_sqlx)?)?,
        version: row.try_get("version").map_err(map_sqlx)?,
        protocol: super::decode(row.try_get("protocol").map_err(map_sqlx)?)?,
        transport: super::decode(row.try_get("transport").map_err(map_sqlx)?)?,
        base_url: row.try_get("base_url").map_err(map_sqlx)?,
        normalized_base_url: row.try_get("normalized_base_url").map_err(map_sqlx)?,
        model_id: row.try_get("model_id").map_err(map_sqlx)?,
        supports_vision: row.try_get::<i64, _>("supports_vision").map_err(map_sqlx)? != 0,
        context_window_tokens: row
            .try_get::<i64, _>("context_window_tokens")
            .map_err(map_sqlx)? as u32,
        max_input_tokens: row
            .try_get::<i64, _>("max_input_tokens")
            .map_err(map_sqlx)? as u32,
        max_output_tokens: row
            .try_get::<i64, _>("max_output_tokens")
            .map_err(map_sqlx)? as u32,
        history_token_budget: row
            .try_get::<i64, _>("history_token_budget")
            .map_err(map_sqlx)? as u32,
        history_turns: row.try_get::<i64, _>("history_turns").map_err(map_sqlx)? as u32,
        temperature: row.try_get("temperature").map_err(map_sqlx)?,
        timeout_ms: row.try_get::<i64, _>("timeout_ms").map_err(map_sqlx)? as u64,
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
    })
}

fn secret_ref(row: &SqliteRow) -> StoreResult<AiModelProfileSecretRef> {
    Ok(AiModelProfileSecretRef {
        profile_id: uuid(row.try_get("profile_id").map_err(map_sqlx)?)?,
        profile_version: row.try_get("profile_version").map_err(map_sqlx)?,
        keyring_account: row.try_get("keyring_account").map_err(map_sqlx)?,
        credential_state: super::decode(row.try_get("credential_state").map_err(map_sqlx)?)?,
        created_at: row.try_get("created_at").map_err(map_sqlx)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx)?,
        revision: row.try_get("revision").map_err(map_sqlx)?,
    })
}

fn validate(profile: &AiModelProfile, version: &AiModelProfileVersion) -> StoreResult<()> {
    if profile.id.is_nil()
        || profile.lab_id.is_nil()
        || profile.user_id.is_nil()
        || profile.name.trim().is_empty()
        || profile.name.chars().count() > 120
        || profile.meta.deleted_at.is_some()
        || profile.meta.revision <= 0
        || profile.current_version != version.version
        || version.profile_id != profile.id
        || version.version <= 0
        || version.base_url.trim().is_empty()
        || version.normalized_base_url.trim().is_empty()
        || version.normalized_base_url != version.base_url.trim().trim_end_matches('/')
        || version.model_id.trim().is_empty()
        || version.model_id.chars().count() > 256
        || !(4_096..=2_000_000).contains(&version.context_window_tokens)
        || !(1_024..=1_900_000).contains(&version.max_input_tokens)
        || !(1..=131_072).contains(&version.max_output_tokens)
        || version.max_input_tokens + version.max_output_tokens > version.context_window_tokens
        || version.history_token_budget > 1_000_000
        || version.history_token_budget > version.max_input_tokens
        || version.history_turns > 100
        || !version.temperature.is_finite()
        || !(0.0..=2.0).contains(&version.temperature)
        || !(100..=600_000).contains(&version.timeout_ms)
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

async fn active_user_lab(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: Uuid,
) -> StoreResult<Uuid> {
    let lab_id: Option<String> =
        sqlx::query_scalar("SELECT lab_id FROM users WHERE id = ? AND deleted_at IS NULL")
            .bind(user_id.to_string())
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx)?;
    lab_id
        .as_deref()
        .map(uuid)
        .transpose()?
        .ok_or(StoreError::NotFound {
            entity: "user",
            id: user_id,
        })
}

async fn insert_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    value: &AiModelProfileVersion,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO ai_model_profile_versions (profile_id, version, protocol, transport, base_url, normalized_base_url, model_id, supports_vision, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms, created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(value.profile_id.to_string()).bind(value.version).bind(super::encode(&value.protocol)?)
        .bind(super::encode(&value.transport)?)
        .bind(&value.base_url).bind(&value.normalized_base_url).bind(&value.model_id)
        .bind(value.supports_vision).bind(value.context_window_tokens).bind(value.max_input_tokens)
        .bind(value.max_output_tokens).bind(value.history_token_budget).bind(value.history_turns)
        .bind(value.temperature).bind(value.timeout_ms as i64).bind(value.created_at)
        .execute(&mut **tx).await.map_err(map_sqlx)?;
    Ok(())
}

#[async_trait]
impl AiModelProfileStore for SqliteStore {
    async fn create_ai_model_profile(
        &self,
        value: &AiModelProfile,
        initial: &AiModelProfileVersion,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate(value, initial)?;
        validate_owner(value.user_id, audit)?;
        if initial.version != 1
            || value.current_version != 1
            || value.meta.revision != 1
            || value.meta.deleted_at.is_some()
            || value.archived_at.is_some()
        {
            return Err(StoreError::Validation(
                "initial AI model profile version must be 1".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let owner_lab = active_user_lab(&mut tx, value.user_id).await?;
        if owner_lab != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile owner belongs to another lab".to_owned(),
            ));
        }
        sqlx::query("INSERT INTO ai_model_profiles (id, lab_id, user_id, name, current_version, created_at, updated_at, archived_at, deleted_at, revision) VALUES (?,?,?,?,?,?,?,?,?,?)")
            .bind(value.id.to_string()).bind(value.lab_id.to_string()).bind(value.user_id.to_string())
            .bind(&value.name).bind(value.current_version).bind(value.meta.created_at).bind(value.meta.updated_at)
            .bind(value.archived_at).bind(value.meta.deleted_at).bind(value.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
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
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id,
        })?;
        profile(&row)
    }

    async fn list_ai_model_profiles(
        &self,
        filter: &AiModelProfileFilter,
    ) -> StoreResult<Vec<AiModelProfile>> {
        let rows = sqlx::query(&format!("SELECT {PROFILE_COLUMNS} FROM ai_model_profiles WHERE lab_id = ? AND user_id = ? AND deleted_at IS NULL AND (? OR archived_at IS NULL) ORDER BY updated_at DESC, id"))
            .bind(filter.lab_id.to_string()).bind(filter.user_id.to_string()).bind(filter.include_archived)
            .fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(profile).collect()
    }

    async fn get_ai_model_profile_version(
        &self,
        profile_id: Uuid,
        number: i64,
    ) -> StoreResult<AiModelProfileVersion> {
        let row = sqlx::query("SELECT v.* FROM ai_model_profile_versions v JOIN ai_model_profiles p ON p.id = v.profile_id WHERE v.profile_id = ? AND v.version = ? AND p.deleted_at IS NULL")
            .bind(profile_id.to_string()).bind(number).fetch_optional(&self.pool).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "ai_model_profile", id: profile_id })?;
        version(&row)
    }

    async fn append_ai_model_profile_version(
        &self,
        value: &AiModelProfile,
        next: &AiModelProfileVersion,
        expected_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate(value, next)?;
        validate_owner(value.user_id, audit)?;
        if expected_revision <= 0
            || value.meta.revision != expected_revision + 1
            || next.version <= 1
        {
            return Err(StoreError::Validation(
                "AI model profile revision must advance exactly once".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let owner_lab = active_user_lab(&mut tx, value.user_id).await?;
        if owner_lab != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile must remain in its user's lab".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(value.id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: value.id,
        })?;
        let before = profile(&before_row)?;
        if before.meta.revision != expected_revision || next.version != before.current_version + 1 {
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
        if !next.supports_vision {
            let is_vision_default: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ai_user_model_defaults
                 WHERE default_vision_profile_id = ? AND deleted_at IS NULL",
            )
            .bind(value.id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if is_vision_default != 0 {
                return Err(StoreError::Validation(
                    "clear the default vision model before disabling vision support".to_owned(),
                ));
            }
        }
        insert_version(&mut tx, next).await?;
        let updated = sqlx::query("UPDATE ai_model_profiles SET name=?, current_version=?, updated_at=?, revision=? WHERE id=? AND revision=? AND archived_at IS NULL")
            .bind(&value.name).bind(value.current_version).bind(value.meta.updated_at)
            .bind(value.meta.revision).bind(value.id.to_string()).bind(expected_revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
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
        if value.archived_at.is_none()
            || value.meta.deleted_at.is_some()
            || expected_revision <= 0
            || value.meta.revision != expected_revision + 1
        {
            return Err(StoreError::Validation(
                "invalid archived AI model profile revision".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let owner_lab = active_user_lab(&mut tx, value.user_id).await?;
        if owner_lab != value.lab_id {
            return Err(StoreError::Validation(
                "AI model profile must remain in its user's lab".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM ai_model_profiles WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(value.id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: value.id,
        })?;
        let before = profile(&before_row)?;
        if before.archived_at.is_some() {
            return Err(StoreError::Conflict(
                "AI model profile is already archived".to_owned(),
            ));
        }
        if before.meta.revision != expected_revision
            || before.user_id != value.user_id
            || before.lab_id != value.lab_id
            || before.name != value.name
            || before.current_version != value.current_version
            || before.meta.created_at != value.meta.created_at
            || before.meta.deleted_at != value.meta.deleted_at
        {
            return Err(StoreError::Conflict(
                "AI model profile changed concurrently".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE ai_model_profiles SET archived_at=?, updated_at=?, revision=? WHERE id=? AND revision=?",
        )
        .bind(value.archived_at)
        .bind(value.meta.updated_at)
        .bind(value.meta.revision)
        .bind(value.id.to_string())
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI model profile changed concurrently".to_owned(),
            ));
        }
        let defaults_row = sqlx::query(
            "SELECT user_id, default_conversation_profile_id, default_vision_profile_id,
                created_at, updated_at, deleted_at, revision
             FROM ai_user_model_defaults
             WHERE user_id = ? AND deleted_at IS NULL",
        )
        .bind(value.user_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if let Some(defaults_row) = defaults_row {
            let before_defaults = defaults(&defaults_row)?;
            if before_defaults.default_conversation_profile_id == Some(value.id)
                || before_defaults.default_vision_profile_id == Some(value.id)
            {
                let mut after_defaults = before_defaults.clone();
                if after_defaults.default_conversation_profile_id == Some(value.id) {
                    after_defaults.default_conversation_profile_id = None;
                }
                if after_defaults.default_vision_profile_id == Some(value.id) {
                    after_defaults.default_vision_profile_id = None;
                }
                after_defaults.meta.updated_at =
                    after_defaults.meta.updated_at.max(value.meta.updated_at);
                after_defaults.meta.revision =
                    after_defaults.meta.revision.checked_add(1).ok_or_else(|| {
                        StoreError::Validation(
                            "AI user model defaults revision overflow".to_owned(),
                        )
                    })?;
                let defaults_updated = sqlx::query(
                    "UPDATE ai_user_model_defaults
                     SET default_conversation_profile_id = ?,
                         default_vision_profile_id = ?,
                         updated_at = ?, revision = ?
                     WHERE user_id = ? AND revision = ? AND deleted_at IS NULL",
                )
                .bind(
                    after_defaults
                        .default_conversation_profile_id
                        .map(|id| id.to_string()),
                )
                .bind(
                    after_defaults
                        .default_vision_profile_id
                        .map(|id| id.to_string()),
                )
                .bind(after_defaults.meta.updated_at)
                .bind(after_defaults.meta.revision)
                .bind(value.user_id.to_string())
                .bind(before_defaults.meta.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
                if defaults_updated.rows_affected() != 1 {
                    return Err(StoreError::Conflict(
                        "AI user model defaults changed while archiving the profile".to_owned(),
                    ));
                }
                write_audit(
                    &mut tx,
                    value.lab_id,
                    None,
                    EntityType::AiUserModelDefaults,
                    value.user_id,
                    AuditAction::Update,
                    audit,
                    Some(snapshot(&before_defaults)?),
                    Some(snapshot(&after_defaults)?),
                )
                .await?;
            }
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
        let before_row = sqlx::query("SELECT user_id, default_conversation_profile_id, default_vision_profile_id, created_at, updated_at, deleted_at, revision FROM ai_user_model_defaults WHERE user_id=? AND deleted_at IS NULL")
            .bind(value.user_id.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx)?;
        let before = before_row.as_ref().map(defaults).transpose()?;
        if before.as_ref().map(|current| current.meta.revision) != expected_revision {
            return Err(StoreError::Conflict(
                "AI user model defaults changed concurrently".to_owned(),
            ));
        }
        if before
            .as_ref()
            .is_some_and(|current| current.meta.created_at != value.meta.created_at)
        {
            return Err(StoreError::Validation(
                "AI user model defaults creation time is immutable".to_owned(),
            ));
        }
        for (profile_id, requires_vision) in [
            (value.default_conversation_profile_id, false),
            (value.default_vision_profile_id, true),
        ] {
            let Some(profile_id) = profile_id else {
                continue;
            };
            let valid: i64 = sqlx::query_scalar(
                "SELECT count(*)
                 FROM ai_model_profiles p
                 JOIN ai_model_profile_versions v
                   ON v.profile_id = p.id AND v.version = p.current_version
                 WHERE p.id = ? AND p.user_id = ? AND p.lab_id = ?
                   AND p.deleted_at IS NULL
                   AND p.archived_at IS NULL AND (? = 0 OR v.supports_vision = 1)",
            )
            .bind(profile_id.to_string())
            .bind(value.user_id.to_string())
            .bind(lab_id.to_string())
            .bind(requires_vision)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if valid != 1 {
                return Err(StoreError::Validation(
                    "AI default model profile is unavailable to this user".to_owned(),
                ));
            }
        }
        sqlx::query("INSERT INTO ai_user_model_defaults (user_id, default_conversation_profile_id, default_vision_profile_id, created_at, updated_at, deleted_at, revision) VALUES (?,?,?,?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET default_conversation_profile_id=excluded.default_conversation_profile_id, default_vision_profile_id=excluded.default_vision_profile_id, updated_at=excluded.updated_at, deleted_at=excluded.deleted_at, revision=excluded.revision")
            .bind(value.user_id.to_string()).bind(value.default_conversation_profile_id.map(|id| id.to_string()))
            .bind(value.default_vision_profile_id.map(|id| id.to_string())).bind(value.meta.created_at)
            .bind(value.meta.updated_at).bind(value.meta.deleted_at).bind(value.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            lab_id,
            None,
            EntityType::AiUserModelDefaults,
            value.user_id,
            if expected_revision.is_some() {
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
        let row = sqlx::query("SELECT user_id, default_conversation_profile_id, default_vision_profile_id, created_at, updated_at, deleted_at, revision FROM ai_user_model_defaults WHERE user_id=? AND deleted_at IS NULL")
            .bind(user_id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?;
        row.as_ref().map(defaults).transpose()
    }
}

#[async_trait]
impl AiModelProfileSecretRefStore for SqliteStore {
    async fn get_ai_model_profile_secret_ref(
        &self,
        profile_id: Uuid,
        profile_version: i64,
    ) -> StoreResult<Option<AiModelProfileSecretRef>> {
        let row = sqlx::query(
            "SELECT profile_id, profile_version, keyring_account, credential_state,
                created_at, updated_at, revision
             FROM ai_model_profile_secret_refs
             WHERE profile_id = ? AND profile_version = ?",
        )
        .bind(profile_id.to_string())
        .bind(profile_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(secret_ref).transpose()
    }

    async fn list_ai_model_profile_secret_refs(
        &self,
        profile_id: Uuid,
    ) -> StoreResult<Vec<AiModelProfileSecretRef>> {
        let rows = sqlx::query(
            "SELECT profile_id, profile_version, keyring_account, credential_state,
                created_at, updated_at, revision
             FROM ai_model_profile_secret_refs
             WHERE profile_id = ?
             ORDER BY profile_version",
        )
        .bind(profile_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(secret_ref).collect()
    }

    async fn save_ai_model_profile_secret_ref(
        &self,
        value: &AiModelProfileSecretRef,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let next_revision = expected_revision
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                StoreError::Validation(
                    "AI model profile secret reference revision overflow".to_owned(),
                )
            })?;
        if value.profile_id.is_nil()
            || value.profile_version <= 0
            || value.keyring_account.trim().is_empty()
            || value.keyring_account.chars().count() > 512
            || value.keyring_account.chars().any(char::is_control)
            || value.revision != next_revision
            || value.updated_at < value.created_at
        {
            return Err(StoreError::Validation(
                "invalid AI model profile secret reference".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let owner_row = sqlx::query(
            "SELECT p.lab_id, p.user_id
             FROM ai_model_profiles p
             JOIN ai_model_profile_versions v
               ON v.profile_id = p.id AND v.version = ?
             WHERE p.id = ? AND p.deleted_at IS NULL",
        )
        .bind(value.profile_version)
        .bind(value.profile_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: value.profile_id,
        })?;
        let lab_id = uuid(owner_row.try_get("lab_id").map_err(map_sqlx)?)?;
        let user_id = uuid(owner_row.try_get("user_id").map_err(map_sqlx)?)?;
        validate_owner(user_id, audit)?;
        if active_user_lab(&mut tx, user_id).await? != lab_id {
            return Err(StoreError::Validation(
                "AI model profile secret reference owner belongs to another lab".to_owned(),
            ));
        }

        let before_row = sqlx::query(
            "SELECT profile_id, profile_version, keyring_account, credential_state,
                created_at, updated_at, revision
             FROM ai_model_profile_secret_refs
             WHERE profile_id = ? AND profile_version = ?",
        )
        .bind(value.profile_id.to_string())
        .bind(value.profile_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let before = before_row.as_ref().map(secret_ref).transpose()?;

        match (&before, expected_revision) {
            (None, None) => {
                if value.revision != 1 {
                    return Err(StoreError::Validation(
                        "initial AI model profile secret reference revision must be 1".to_owned(),
                    ));
                }
                sqlx::query(
                    "INSERT INTO ai_model_profile_secret_refs (
                        profile_id, profile_version, keyring_account, credential_state,
                        created_at, updated_at, revision
                     ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(value.profile_id.to_string())
                .bind(value.profile_version)
                .bind(&value.keyring_account)
                .bind(super::encode(&value.credential_state)?)
                .bind(value.created_at)
                .bind(value.updated_at)
                .bind(value.revision)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            }
            (Some(current), Some(expected)) => {
                if current.revision != expected {
                    return Err(StoreError::Conflict(
                        "AI model profile secret reference changed concurrently".to_owned(),
                    ));
                }
                if current.profile_id != value.profile_id
                    || current.profile_version != value.profile_version
                    || current.keyring_account != value.keyring_account
                    || current.created_at != value.created_at
                {
                    return Err(StoreError::Validation(
                        "AI model profile secret reference identity is immutable".to_owned(),
                    ));
                }
                let updated = sqlx::query(
                    "UPDATE ai_model_profile_secret_refs
                     SET credential_state = ?, updated_at = ?, revision = ?
                     WHERE profile_id = ? AND profile_version = ? AND revision = ?",
                )
                .bind(super::encode(&value.credential_state)?)
                .bind(value.updated_at)
                .bind(value.revision)
                .bind(value.profile_id.to_string())
                .bind(value.profile_version)
                .bind(expected)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
                if updated.rows_affected() != 1 {
                    return Err(StoreError::Conflict(
                        "AI model profile secret reference changed before update".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(StoreError::Conflict(
                    "AI model profile secret reference revision does not match".to_owned(),
                ));
            }
        }

        write_audit(
            &mut tx,
            lab_id,
            None,
            EntityType::AiModelProfile,
            value.profile_id,
            match (&before, value.credential_state) {
                (None, _) => AuditAction::Create,
                (Some(_), muriarc_core::AiModelCredentialState::Revoked) => AuditAction::Revoke,
                (Some(_), muriarc_core::AiModelCredentialState::Present) => AuditAction::Update,
            },
            audit,
            before.as_ref().map(snapshot).transpose()?,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn revoke_ai_model_profile_secret_refs(
        &self,
        profile_id: Uuid,
        revoked_at: chrono::DateTime<chrono::Utc>,
        audit: &AuditContext,
    ) -> StoreResult<Vec<AiModelProfileSecretRef>> {
        if profile_id.is_nil() {
            return Err(StoreError::Validation(
                "invalid AI model profile secret reference profile".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let owner_row = sqlx::query(
            "SELECT lab_id, user_id
             FROM ai_model_profiles
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(profile_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_model_profile",
            id: profile_id,
        })?;
        let lab_id = uuid(owner_row.try_get("lab_id").map_err(map_sqlx)?)?;
        let user_id = uuid(owner_row.try_get("user_id").map_err(map_sqlx)?)?;
        validate_owner(user_id, audit)?;
        if active_user_lab(&mut tx, user_id).await? != lab_id {
            return Err(StoreError::Validation(
                "AI model profile secret reference owner belongs to another lab".to_owned(),
            ));
        }

        let rows = sqlx::query(
            "SELECT profile_id, profile_version, keyring_account, credential_state,
                created_at, updated_at, revision
             FROM ai_model_profile_secret_refs
             WHERE profile_id = ?
             ORDER BY profile_version",
        )
        .bind(profile_id.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let before_values = rows
            .iter()
            .map(secret_ref)
            .collect::<StoreResult<Vec<_>>>()?;
        let mut after_values = Vec::with_capacity(before_values.len());

        for before in before_values {
            if before.credential_state == AiModelCredentialState::Revoked {
                after_values.push(before);
                continue;
            }
            let mut after = before.clone();
            after.credential_state = AiModelCredentialState::Revoked;
            after.updated_at = after.updated_at.max(revoked_at);
            after.revision = after.revision.checked_add(1).ok_or_else(|| {
                StoreError::Validation(
                    "AI model profile secret reference revision overflow".to_owned(),
                )
            })?;
            let updated = sqlx::query(
                "UPDATE ai_model_profile_secret_refs
                 SET credential_state = ?, updated_at = ?, revision = ?
                 WHERE profile_id = ? AND profile_version = ? AND revision = ?
                   AND credential_state = ?",
            )
            .bind(super::encode(&after.credential_state)?)
            .bind(after.updated_at)
            .bind(after.revision)
            .bind(profile_id.to_string())
            .bind(after.profile_version)
            .bind(before.revision)
            .bind(super::encode(&AiModelCredentialState::Present)?)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "AI model profile secret references changed before revocation".to_owned(),
                ));
            }
            write_audit(
                &mut tx,
                lab_id,
                None,
                EntityType::AiModelProfile,
                profile_id,
                AuditAction::Revoke,
                audit,
                Some(snapshot(&before)?),
                Some(snapshot(&after)?),
            )
            .await?;
            after_values.push(after);
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(after_values)
    }
}
