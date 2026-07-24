use async_trait::async_trait;
use muriarc_core::{
    AiAutonomyGrant, AiConversation, AiConversationFilter, AiConversationMessage,
    AiConversationMessageRole, AiOperationStore, AnimalEvent, AnimalEventKind, Approval,
    ApprovalDecision, AuditAction, AuditContext, EntityType, Measurement, Provenance,
    ProvenanceSource, RecordStatus, StoreError, StoreResult, ToolRun, ToolRunStatus,
};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use super::{
    PgTransaction, PostgresStore, append_derived_animal_event, encode, insert_measurement,
    insert_provenance, lock_and_reject_duplicate_measurement, map_sqlx, meta, snapshot,
    validate_measurement_relationships, write_audit,
};

const CONVERSATION_COLUMNS: &str = "id, lab_id, project_id, user_id, title, model_profile_id, model_profile_version, legacy_read_only, created_at, updated_at, deleted_at, revision";
const MESSAGE_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, created_at, updated_at, deleted_at, revision";
const TOOL_RUN_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision";
const APPROVAL_COLUMNS: &str = "id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision";
const AUTONOMY_GRANT_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision";

fn conversation_from_row(row: &PgRow) -> StoreResult<AiConversation> {
    Ok(AiConversation {
        id: row.try_get("id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        title: row.try_get("title").map_err(map_sqlx)?,
        model_profile: match (
            row.try_get::<Option<Uuid>, _>("model_profile_id")
                .map_err(map_sqlx)?,
            row.try_get::<Option<i64>, _>("model_profile_version")
                .map_err(map_sqlx)?,
        ) {
            (Some(profile_id), Some(profile_version)) => {
                Some(muriarc_core::AiModelProfileBinding {
                    profile_id,
                    profile_version,
                })
            }
            (None, None) => None,
            _ => {
                return Err(StoreError::Serialization(
                    "incomplete AI conversation model binding".to_owned(),
                ));
            }
        },
        legacy_read_only: row.try_get("legacy_read_only").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn message_from_row(row: &PgRow) -> StoreResult<AiConversationMessage> {
    Ok(AiConversationMessage {
        id: row.try_get("id").map_err(map_sqlx)?,
        conversation_id: row.try_get("conversation_id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        sequence: row.try_get("sequence").map_err(map_sqlx)?,
        role: super::decode(row.try_get("role").map_err(map_sqlx)?)?,
        content: row.try_get("content").map_err(map_sqlx)?,
        response: row.try_get("response_json").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn tool_run_from_row(row: &PgRow) -> StoreResult<ToolRun> {
    Ok(ToolRun {
        id: row.try_get("id").map_err(map_sqlx)?,
        conversation_id: row.try_get("conversation_id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        tool_name: row.try_get("tool_name").map_err(map_sqlx)?,
        input: row.try_get("input_json").map_err(map_sqlx)?,
        output: row.try_get("output_json").map_err(map_sqlx)?,
        status: super::decode(row.try_get("status").map_err(map_sqlx)?)?,
        source: super::decode(row.try_get("source").map_err(map_sqlx)?)?,
        started_at: row.try_get("started_at").map_err(map_sqlx)?,
        completed_at: row.try_get("completed_at").map_err(map_sqlx)?,
        error: row.try_get("error").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn approval_from_row(row: &PgRow) -> StoreResult<Approval> {
    Ok(Approval {
        id: row.try_get("id").map_err(map_sqlx)?,
        tool_run_id: row.try_get("tool_run_id").map_err(map_sqlx)?,
        requested_diff: row.try_get("requested_diff_json").map_err(map_sqlx)?,
        decision: super::decode(row.try_get("decision").map_err(map_sqlx)?)?,
        decided_by: row.try_get("decided_by").map_err(map_sqlx)?,
        decided_at: row.try_get("decided_at").map_err(map_sqlx)?,
        reason: row.try_get("reason").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn autonomy_grant_from_row(row: &PgRow) -> StoreResult<AiAutonomyGrant> {
    Ok(AiAutonomyGrant {
        id: row.try_get("id").map_err(map_sqlx)?,
        conversation_id: row.try_get("conversation_id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        session_id: row.try_get("session_id").map_err(map_sqlx)?,
        mode: super::decode(row.try_get("mode").map_err(map_sqlx)?)?,
        allowed_categories: serde_json::from_value(
            row.try_get("allowed_categories_json").map_err(map_sqlx)?,
        )
        .map_err(|error| StoreError::Serialization(error.to_string()))?,
        batch_limit: row.try_get::<i32, _>("batch_limit").map_err(map_sqlx)? as u32,
        step_up_verified_at: row.try_get("step_up_verified_at").map_err(map_sqlx)?,
        last_used_at: row.try_get("last_used_at").map_err(map_sqlx)?,
        expires_at: row.try_get("expires_at").map_err(map_sqlx)?,
        revoked_at: row.try_get("revoked_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn validate_autonomy_grant(value: &AiAutonomyGrant) -> StoreResult<()> {
    let mode_metadata_is_valid = match value.mode {
        muriarc_core::AiAutonomyMode::Full => {
            value.session_id.is_some_and(|id| !id.is_nil())
                && value.step_up_verified_at.is_some()
                && value.expires_at.is_some()
        }
        muriarc_core::AiAutonomyMode::Ask | muriarc_core::AiAutonomyMode::Auto => {
            value.session_id.is_none()
                && value.step_up_verified_at.is_none()
                && value.expires_at.is_none()
        }
    };
    if value.id.is_nil()
        || value.conversation_id.is_nil()
        || value.lab_id.is_nil()
        || value.user_id.is_nil()
        || value.allowed_categories.is_empty()
        || value.batch_limit != value.mode.batch_limit()
        || value.batch_limit > 100
        || !mode_metadata_is_valid
    {
        return Err(StoreError::Validation(
            "invalid AI autonomy grant".to_owned(),
        ));
    }
    Ok(())
}

fn validate_conversation(value: &AiConversation) -> StoreResult<()> {
    if value.id.is_nil()
        || value.lab_id.is_nil()
        || value.user_id.is_nil()
        || value.project_id.is_some_and(|id| id.is_nil())
        || value.title.trim().is_empty()
        || value.title.chars().count() > 256
        || value
            .model_profile
            .is_none_or(|binding| binding.profile_id.is_nil() || binding.profile_version <= 0)
        || value.legacy_read_only
    {
        return Err(StoreError::Validation(
            "new AI conversations require a valid immutable model profile binding and a 1-256 character title"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_initial_conversation_autonomy(
    conversation: &AiConversation,
    grant: &AiAutonomyGrant,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_conversation(conversation)?;
    validate_autonomy_grant(grant)?;
    if conversation.meta.revision != 1
        || conversation.meta.deleted_at.is_some()
        || grant.meta.revision != 1
        || grant.meta.deleted_at.is_some()
        || grant.revoked_at.is_some()
        || grant.conversation_id != conversation.id
        || grant.lab_id != conversation.lab_id
        || grant.project_id != conversation.project_id
        || grant.user_id != conversation.user_id
    {
        return Err(StoreError::Validation(
            "initial AI autonomy grant must match its new conversation scope and owner".to_owned(),
        ));
    }
    if audit.actor.user_id != Some(conversation.user_id) {
        return Err(StoreError::Validation(
            "AI conversation audit actor must match its owner".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_conversation_references(
    tx: &mut PgTransaction<'_>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    if let Some(project_id) = conversation.project_id {
        let project_lab: Uuid = sqlx::query_scalar(
            "SELECT lab_id FROM projects WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "project",
            id: project_id,
        })?;
        if project_lab != conversation.lab_id {
            return Err(StoreError::Validation(
                "AI conversation project belongs to another lab".to_owned(),
            ));
        }
    }
    let binding = conversation.model_profile.ok_or_else(|| {
        StoreError::Validation(
            "new AI conversations require an immutable model profile binding".to_owned(),
        )
    })?;
    let owner: Option<(Uuid, Uuid, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT p.lab_id, p.user_id, p.archived_at
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = $1
         WHERE p.id = $2 AND p.deleted_at IS NULL
         FOR SHARE OF p",
    )
    .bind(binding.profile_version)
    .bind(binding.profile_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    let Some((lab_id, user_id, archived_at)) = owner else {
        return Err(StoreError::Validation(
            "AI conversation model profile version does not exist".to_owned(),
        ));
    };
    if lab_id != conversation.lab_id || user_id != conversation.user_id || archived_at.is_some() {
        return Err(StoreError::Validation(
            "AI conversation model profile is unavailable to this user".to_owned(),
        ));
    }
    Ok(())
}

async fn insert_conversation(
    tx: &mut PgTransaction<'_>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO ai_conversations (id, lab_id, project_id, user_id, title, model_profile_id, model_profile_version, legacy_read_only, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(conversation.id)
        .bind(conversation.lab_id)
        .bind(conversation.project_id)
        .bind(conversation.user_id)
        .bind(&conversation.title)
        .bind(conversation.model_profile.map(|binding| binding.profile_id))
        .bind(conversation.model_profile.map(|binding| binding.profile_version))
        .bind(conversation.legacy_read_only)
        .bind(conversation.meta.created_at)
        .bind(conversation.meta.updated_at)
        .bind(conversation.meta.deleted_at)
        .bind(conversation.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

async fn insert_autonomy_grant(
    tx: &mut PgTransaction<'_>,
    grant: &AiAutonomyGrant,
) -> StoreResult<()> {
    let categories = serde_json::to_value(&grant.allowed_categories)
        .map_err(|error| StoreError::Serialization(error.to_string()))?;
    sqlx::query("INSERT INTO ai_autonomy_grants (id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
        .bind(grant.id)
        .bind(grant.conversation_id)
        .bind(grant.lab_id)
        .bind(grant.project_id)
        .bind(grant.user_id)
        .bind(grant.session_id)
        .bind(encode(&grant.mode)?)
        .bind(categories)
        .bind(grant.batch_limit as i32)
        .bind(grant.step_up_verified_at)
        .bind(grant.last_used_at)
        .bind(grant.expires_at)
        .bind(grant.revoked_at)
        .bind(grant.meta.created_at)
        .bind(grant.meta.updated_at)
        .bind(grant.meta.deleted_at)
        .bind(grant.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

fn validate_message(value: &AiConversationMessage) -> StoreResult<()> {
    value
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))
}

async fn insert_message(
    tx: &mut PgTransaction<'_>,
    value: &AiConversationMessage,
    audit: &AuditContext,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO ai_conversation_messages (id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
        .bind(value.id)
        .bind(value.conversation_id)
        .bind(value.lab_id)
        .bind(value.project_id)
        .bind(value.user_id)
        .bind(value.sequence)
        .bind(encode(&value.role)?)
        .bind(&value.content)
        .bind(&value.response)
        .bind(value.meta.created_at)
        .bind(value.meta.updated_at)
        .bind(value.meta.deleted_at)
        .bind(value.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        value.lab_id,
        value.project_id,
        EntityType::AiConversationMessage,
        value.id,
        AuditAction::Create,
        audit,
        None,
        Some(snapshot(value)?),
    )
    .await
}

fn validate_tool_run(value: &ToolRun) -> StoreResult<()> {
    if value.tool_name.trim().is_empty()
        || value.tool_name.len() > 64
        || value.error.as_ref().is_some_and(|error| error.len() > 1024)
    {
        return Err(StoreError::Validation("invalid AI tool run".to_owned()));
    }
    Ok(())
}

fn validate_approval(value: &Approval) -> StoreResult<()> {
    let valid = match value.decision {
        ApprovalDecision::Pending => value.decided_by.is_none() && value.decided_at.is_none(),
        ApprovalDecision::Approved | ApprovalDecision::Rejected => {
            value.decided_by.is_some() && value.decided_at.is_some()
        }
    };
    if !valid
        || value
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 1024)
    {
        return Err(StoreError::Validation(
            "invalid AI approval state".to_owned(),
        ));
    }
    Ok(())
}

async fn conversation_in_tx(tx: &mut PgTransaction<'_>, id: Uuid) -> StoreResult<AiConversation> {
    let row = sqlx::query(&format!(
        "SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_conversation",
        id,
    })?;
    conversation_from_row(&row)
}

async fn ensure_conversation_model_available(
    tx: &mut PgTransaction<'_>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    if conversation.legacy_read_only {
        return Err(StoreError::Conflict(
            "legacy AI conversation is read-only".to_owned(),
        ));
    }
    let Some(binding) = conversation.model_profile else {
        return Err(StoreError::Conflict(
            "AI conversation model profile is unavailable".to_owned(),
        ));
    };
    let profile_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT p.id
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = $1
         WHERE p.id = $2
           AND p.lab_id = $3
           AND p.user_id = $4
           AND p.archived_at IS NULL
           AND p.deleted_at IS NULL
         FOR SHARE OF p",
    )
    .bind(binding.profile_version)
    .bind(binding.profile_id)
    .bind(conversation.lab_id)
    .bind(conversation.user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if profile_id.is_none() {
        return Err(StoreError::Conflict(
            "AI conversation model profile is unavailable".to_owned(),
        ));
    }
    Ok(())
}

async fn ensure_writable_tool_conversation(
    tx: &mut PgTransaction<'_>,
    tool_run: &ToolRun,
) -> StoreResult<()> {
    let Some(conversation_id) = tool_run.conversation_id else {
        return Ok(());
    };
    let conversation = conversation_in_tx(tx, conversation_id).await?;
    ensure_conversation_model_available(tx, &conversation).await?;
    if conversation.lab_id != tool_run.lab_id
        || conversation.user_id != tool_run.user_id
        || conversation.project_id != tool_run.project_id
    {
        return Err(StoreError::Validation(
            "AI tool run scope differs from its conversation".to_owned(),
        ));
    }
    Ok(())
}

async fn tool_run_in_tx(tx: &mut PgTransaction<'_>, id: Uuid) -> StoreResult<ToolRun> {
    let row = sqlx::query(&format!("SELECT {TOOL_RUN_COLUMNS} FROM ai_tool_runs WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"))
        .bind(id).fetch_optional(&mut **tx).await.map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "ai_tool_run", id })?;
    tool_run_from_row(&row)
}

async fn approval_in_tx(tx: &mut PgTransaction<'_>, id: Uuid) -> StoreResult<Approval> {
    let row = sqlx::query(&format!("SELECT {APPROVAL_COLUMNS} FROM ai_approvals WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"))
        .bind(id).fetch_optional(&mut **tx).await.map_err(map_sqlx)?
        .ok_or(StoreError::NotFound { entity: "ai_approval", id })?;
    approval_from_row(&row)
}

async fn update_resolution_tx(
    tx: &mut PgTransaction<'_>,
    tool_run: &ToolRun,
    expected_tool_revision: i64,
    approval: &Approval,
    expected_approval_revision: i64,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_tool_run(tool_run)?;
    validate_approval(approval)?;
    if tool_run.meta.revision != expected_tool_revision + 1
        || approval.meta.revision != expected_approval_revision + 1
    {
        return Err(StoreError::Validation(
            "AI operation revision must advance exactly once".to_owned(),
        ));
    }
    let before_tool = tool_run_in_tx(tx, tool_run.id).await?;
    let before_approval = approval_in_tx(tx, approval.id).await?;
    ensure_writable_tool_conversation(tx, &before_tool).await?;
    if before_tool.meta.revision != expected_tool_revision
        || before_approval.meta.revision != expected_approval_revision
        || before_approval.tool_run_id != before_tool.id
        || approval.tool_run_id != tool_run.id
        || before_tool.conversation_id != tool_run.conversation_id
        || before_tool.lab_id != tool_run.lab_id
        || before_tool.project_id != tool_run.project_id
        || before_tool.user_id != tool_run.user_id
        || before_tool.tool_name != tool_run.tool_name
        || before_tool.status != ToolRunStatus::AwaitingApproval
        || before_approval.decision != ApprovalDecision::Pending
    {
        return Err(StoreError::Conflict(
            "AI draft changed before the decision was applied".to_owned(),
        ));
    }
    if audit.actor.user_id != approval.decided_by {
        return Err(StoreError::Validation(
            "AI approval must be attributed to the deciding human".to_owned(),
        ));
    }
    let updated_tool = sqlx::query("UPDATE ai_tool_runs SET output_json = $1, status = $2, completed_at = $3, error = $4, updated_at = $5, deleted_at = $6, revision = $7 WHERE id = $8 AND revision = $9 AND deleted_at IS NULL")
        .bind(&tool_run.output).bind(encode(&tool_run.status)?).bind(tool_run.completed_at).bind(&tool_run.error)
        .bind(tool_run.meta.updated_at).bind(tool_run.meta.deleted_at).bind(tool_run.meta.revision)
        .bind(tool_run.id).bind(expected_tool_revision).execute(&mut **tx).await.map_err(map_sqlx)?;
    let updated_approval = sqlx::query("UPDATE ai_approvals SET requested_diff_json = $1, decision = $2, decided_by = $3, decided_at = $4, reason = $5, updated_at = $6, deleted_at = $7, revision = $8 WHERE id = $9 AND revision = $10 AND deleted_at IS NULL")
        .bind(&approval.requested_diff).bind(encode(&approval.decision)?).bind(approval.decided_by).bind(approval.decided_at)
        .bind(&approval.reason).bind(approval.meta.updated_at).bind(approval.meta.deleted_at).bind(approval.meta.revision)
        .bind(approval.id).bind(expected_approval_revision).execute(&mut **tx).await.map_err(map_sqlx)?;
    if updated_tool.rows_affected() != 1 || updated_approval.rows_affected() != 1 {
        return Err(StoreError::Conflict(
            "AI draft revision changed during decision".to_owned(),
        ));
    }
    write_audit(
        tx,
        tool_run.lab_id,
        tool_run.project_id,
        EntityType::ToolRun,
        tool_run.id,
        AuditAction::Update,
        audit,
        Some(snapshot(&before_tool)?),
        Some(snapshot(tool_run)?),
    )
    .await?;
    write_audit(
        tx,
        tool_run.lab_id,
        tool_run.project_id,
        EntityType::Approval,
        approval.id,
        AuditAction::Update,
        audit,
        Some(snapshot(&before_approval)?),
        Some(snapshot(approval)?),
    )
    .await
}

#[async_trait]
impl AiOperationStore for PostgresStore {
    async fn create_ai_conversation(
        &self,
        value: &AiConversation,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_conversation(value)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_conversation_references(&mut tx, value).await?;
        insert_conversation(&mut tx, value).await?;
        write_audit(
            &mut tx,
            value.lab_id,
            value.project_id,
            EntityType::AiConversation,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn create_ai_conversation_with_autonomy(
        &self,
        conversation: &AiConversation,
        grant: &AiAutonomyGrant,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_initial_conversation_autonomy(conversation, grant, audit)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_conversation_references(&mut tx, conversation).await?;
        insert_conversation(&mut tx, conversation).await?;
        insert_autonomy_grant(&mut tx, grant).await?;
        write_audit(
            &mut tx,
            conversation.lab_id,
            conversation.project_id,
            EntityType::AiConversation,
            conversation.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(conversation)?),
        )
        .await?;
        write_audit(
            &mut tx,
            grant.lab_id,
            grant.project_id,
            EntityType::AiAutonomyGrant,
            grant.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(grant)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_ai_conversation(&self, id: Uuid) -> StoreResult<AiConversation> {
        let row = sqlx::query(&format!("SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE id = $1 AND deleted_at IS NULL"))
            .bind(id).fetch_optional(&self.pool).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound { entity: "ai_conversation", id })?;
        conversation_from_row(&row)
    }

    async fn list_ai_conversations(
        &self,
        filter: &AiConversationFilter,
        offset: u32,
        limit: u32,
    ) -> StoreResult<Vec<AiConversation>> {
        if limit == 0 || limit > 100 || offset > 100_000 {
            return Err(StoreError::Validation(
                "AI conversation page is outside the allowed range".to_owned(),
            ));
        }
        let rows = sqlx::query(&format!(
            "SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE lab_id = $1 AND user_id = $2 AND ($3::uuid IS NULL OR project_id = $3) AND deleted_at IS NULL ORDER BY updated_at DESC, id DESC LIMIT $4 OFFSET $5"
        ))
        .bind(filter.lab_id)
        .bind(filter.user_id)
        .bind(filter.project_id)
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(conversation_from_row).collect()
    }

    async fn append_ai_turn_messages(
        &self,
        user_message: &AiConversationMessage,
        assistant_message: &AiConversationMessage,
        expected_last_sequence: i64,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation> {
        validate_message(user_message)?;
        validate_message(assistant_message)?;
        if expected_last_sequence < 0
            || user_message.role != AiConversationMessageRole::User
            || assistant_message.role != AiConversationMessageRole::Assistant
            || assistant_message.sequence != user_message.sequence + 1
            || user_message.conversation_id != assistant_message.conversation_id
            || user_message.lab_id != assistant_message.lab_id
            || user_message.project_id != assistant_message.project_id
            || user_message.user_id != assistant_message.user_id
        {
            return Err(StoreError::Validation(
                "AI turn messages must be one contiguous user/assistant pair".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let before = conversation_in_tx(&mut tx, user_message.conversation_id).await?;
        ensure_conversation_model_available(&mut tx, &before).await?;
        if before.lab_id != user_message.lab_id
            || before.project_id != user_message.project_id
            || before.user_id != user_message.user_id
        {
            return Err(StoreError::Validation(
                "AI message scope differs from its conversation".to_owned(),
            ));
        }
        let actual_last: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(sequence), 0)::bigint FROM ai_conversation_messages WHERE conversation_id = $1 AND deleted_at IS NULL",
        )
        .bind(before.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if actual_last != expected_last_sequence {
            return Err(StoreError::Conflict(
                "AI conversation changed before the turn was saved".to_owned(),
            ));
        }
        if user_message.sequence != actual_last + 1 {
            return Err(StoreError::Validation(
                "AI turn messages must immediately follow the saved conversation".to_owned(),
            ));
        }

        insert_message(&mut tx, user_message, audit).await?;
        insert_message(&mut tx, assistant_message, audit).await?;
        let mut updated = before.clone();
        updated.meta.touch(assistant_message.meta.created_at);
        let result = sqlx::query("UPDATE ai_conversations SET updated_at = $1, revision = $2 WHERE id = $3 AND revision = $4 AND deleted_at IS NULL")
            .bind(updated.meta.updated_at)
            .bind(updated.meta.revision)
            .bind(updated.id)
            .bind(before.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI conversation revision changed during turn persistence".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            updated.lab_id,
            updated.project_id,
            EntityType::AiConversation,
            updated.id,
            AuditAction::Update,
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&updated)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(updated)
    }

    async fn list_ai_conversation_messages(
        &self,
        conversation_id: Uuid,
        limit: u32,
    ) -> StoreResult<Vec<AiConversationMessage>> {
        if limit == 0 || limit > 200 {
            return Err(StoreError::Validation(
                "AI conversation message limit must be between 1 and 200".to_owned(),
            ));
        }
        let mut rows = sqlx::query(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM ai_conversation_messages WHERE conversation_id = $1 AND deleted_at IS NULL ORDER BY sequence DESC LIMIT $2"
        ))
        .bind(conversation_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.reverse();
        rows.iter().map(message_from_row).collect()
    }

    async fn get_ai_autonomy_grant(
        &self,
        conversation_id: Uuid,
    ) -> StoreResult<Option<AiAutonomyGrant>> {
        let row = sqlx::query(&format!(
            "SELECT {AUTONOMY_GRANT_COLUMNS} FROM ai_autonomy_grants WHERE conversation_id = $1 AND deleted_at IS NULL"
        ))
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(autonomy_grant_from_row).transpose()
    }

    async fn save_ai_autonomy_grant(
        &self,
        grant: &AiAutonomyGrant,
        expected_revision: Option<i64>,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_autonomy_grant(grant)?;
        if audit.actor.user_id != Some(grant.user_id) {
            return Err(StoreError::Validation(
                "AI autonomy grant actor must match its owner".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let conversation = conversation_in_tx(&mut tx, grant.conversation_id).await?;
        ensure_conversation_model_available(&mut tx, &conversation).await?;
        if conversation.lab_id != grant.lab_id
            || conversation.project_id != grant.project_id
            || conversation.user_id != grant.user_id
        {
            return Err(StoreError::Validation(
                "AI autonomy grant scope differs from its conversation".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {AUTONOMY_GRANT_COLUMNS} FROM ai_autonomy_grants WHERE conversation_id = $1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(grant.conversation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let before = before_row
            .as_ref()
            .map(autonomy_grant_from_row)
            .transpose()?;
        if before.as_ref().map(|value| value.meta.revision) != expected_revision
            || expected_revision.is_some_and(|revision| grant.meta.revision != revision + 1)
            || (expected_revision.is_none() && grant.meta.revision != 1)
            || before.as_ref().is_some_and(|value| value.id != grant.id)
        {
            return Err(StoreError::Conflict(
                "AI autonomy grant revision changed".to_owned(),
            ));
        }
        let categories = serde_json::to_value(&grant.allowed_categories)
            .map_err(|error| StoreError::Serialization(error.to_string()))?;
        if before.is_some() {
            let result = sqlx::query("UPDATE ai_autonomy_grants SET session_id = $1, mode = $2, allowed_categories_json = $3, batch_limit = $4, step_up_verified_at = $5, last_used_at = $6, expires_at = $7, revoked_at = $8, updated_at = $9, deleted_at = $10, revision = $11 WHERE id = $12 AND revision = $13 AND deleted_at IS NULL")
                .bind(grant.session_id).bind(encode(&grant.mode)?).bind(categories).bind(grant.batch_limit as i32)
                .bind(grant.step_up_verified_at).bind(grant.last_used_at).bind(grant.expires_at).bind(grant.revoked_at)
                .bind(grant.meta.updated_at).bind(grant.meta.deleted_at).bind(grant.meta.revision).bind(grant.id).bind(expected_revision)
                .execute(&mut *tx).await.map_err(map_sqlx)?;
            if result.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "AI autonomy grant revision changed".to_owned(),
                ));
            }
        } else {
            insert_autonomy_grant(&mut tx, grant).await?;
        }
        write_audit(
            &mut tx,
            grant.lab_id,
            grant.project_id,
            EntityType::AiAutonomyGrant,
            grant.id,
            if before.is_some() {
                AuditAction::Update
            } else {
                AuditAction::Create
            },
            audit,
            before.as_ref().map(snapshot).transpose()?,
            Some(snapshot(grant)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn create_tool_run(&self, value: &ToolRun, audit: &AuditContext) -> StoreResult<()> {
        validate_tool_run(value)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        ensure_writable_tool_conversation(&mut tx, value).await?;
        sqlx::query("INSERT INTO ai_tool_runs (id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
            .bind(value.id).bind(value.conversation_id).bind(value.lab_id).bind(value.project_id).bind(value.user_id).bind(&value.tool_name)
            .bind(&value.input).bind(&value.output).bind(encode(&value.status)?).bind(encode(&value.source)?).bind(value.started_at).bind(value.completed_at)
            .bind(&value.error).bind(value.meta.created_at).bind(value.meta.updated_at).bind(value.meta.deleted_at).bind(value.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            value.lab_id,
            value.project_id,
            EntityType::ToolRun,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_tool_run(&self, id: Uuid) -> StoreResult<ToolRun> {
        let row = sqlx::query(&format!(
            "SELECT {TOOL_RUN_COLUMNS} FROM ai_tool_runs WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_tool_run",
            id,
        })?;
        tool_run_from_row(&row)
    }

    async fn create_approval(&self, value: &Approval, audit: &AuditContext) -> StoreResult<()> {
        validate_approval(value)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let tool = tool_run_in_tx(&mut tx, value.tool_run_id).await?;
        ensure_writable_tool_conversation(&mut tx, &tool).await?;
        if tool.status != ToolRunStatus::AwaitingApproval
            || value.decision != ApprovalDecision::Pending
        {
            return Err(StoreError::Validation(
                "only an awaiting AI tool run can request approval".to_owned(),
            ));
        }
        sqlx::query("INSERT INTO ai_approvals (id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(value.id).bind(value.tool_run_id).bind(&value.requested_diff).bind(encode(&value.decision)?).bind(value.decided_by).bind(value.decided_at)
            .bind(&value.reason).bind(value.meta.created_at).bind(value.meta.updated_at).bind(value.meta.deleted_at).bind(value.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            tool.lab_id,
            tool.project_id,
            EntityType::Approval,
            value.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(value)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_approval(&self, id: Uuid) -> StoreResult<Approval> {
        let row = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM ai_approvals WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_approval",
            id,
        })?;
        approval_from_row(&row)
    }

    async fn list_approvals(
        &self,
        filter: &muriarc_core::AiApprovalFilter,
    ) -> StoreResult<Vec<Approval>> {
        let decision = filter.decision.map(|value| encode(&value)).transpose()?;
        let rows = sqlx::query(
            "SELECT a.id, a.tool_run_id, a.requested_diff_json, a.decision, a.decided_by, a.decided_at, a.reason, a.created_at, a.updated_at, a.deleted_at, a.revision FROM ai_approvals a JOIN ai_tool_runs t ON t.id = a.tool_run_id WHERE t.lab_id = $1 AND t.user_id = $2 AND ($3::uuid IS NULL OR t.project_id = $3) AND ($4::text IS NULL OR a.decision = $4) AND a.deleted_at IS NULL AND t.deleted_at IS NULL ORDER BY a.created_at DESC, a.id",
        )
        .bind(filter.lab_id)
        .bind(filter.user_id)
        .bind(filter.project_id)
        .bind(decision)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.iter().map(approval_from_row).collect()
    }

    async fn finalize_ai_draft(
        &self,
        tool: &ToolRun,
        expected_tool_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let valid_resolution = matches!(
            (approval.decision, tool.status),
            (ApprovalDecision::Rejected, ToolRunStatus::Cancelled)
                | (ApprovalDecision::Approved, ToolRunStatus::Completed)
                | (ApprovalDecision::Approved, ToolRunStatus::Failed)
        );
        if !valid_resolution {
            return Err(StoreError::Validation(
                "invalid non-measurement AI draft resolution".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        update_resolution_tx(
            &mut tx,
            tool,
            expected_tool_revision,
            approval,
            expected_approval_revision,
            audit,
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn apply_ai_measurement_draft(
        &self,
        measurement: &Measurement,
        expected_animal_revision: i64,
        tool: &ToolRun,
        expected_tool_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        if approval.decision != ApprovalDecision::Approved
            || tool.status != ToolRunStatus::Completed
            || measurement.status != RecordStatus::Draft
            || measurement.signed_by.is_some()
            || measurement.signed_at.is_some()
            || tool.lab_id != measurement.lab_id
            || tool.project_id != Some(measurement.project_id)
        {
            return Err(StoreError::Validation(
                "invalid approved AI measurement draft".to_owned(),
            ));
        }
        measurement
            .validate_record()
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let actual_animal_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM animals WHERE id = $1 AND lab_id = $2 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(measurement.animal_id)
        .bind(measurement.lab_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "animal",
            id: measurement.animal_id,
        })?;
        if actual_animal_revision != expected_animal_revision {
            return Err(StoreError::Conflict(
                "animal revision changed after AI draft creation".to_owned(),
            ));
        }
        update_resolution_tx(
            &mut tx,
            tool,
            expected_tool_revision,
            approval,
            expected_approval_revision,
            audit,
        )
        .await?;
        validate_measurement_relationships(&mut tx, measurement).await?;
        lock_and_reject_duplicate_measurement(&mut tx, measurement).await?;
        insert_measurement(&mut tx, measurement).await?;
        write_audit(
            &mut tx,
            measurement.lab_id,
            Some(measurement.project_id),
            EntityType::Measurement,
            measurement.id,
            AuditAction::Create,
            audit,
            None,
            Some(snapshot(measurement)?),
        )
        .await?;
        let mut provenance = Provenance::from_audit(
            measurement.lab_id,
            Some(measurement.project_id),
            EntityType::Measurement,
            measurement.id,
            audit,
            measurement.meta.created_at,
        );
        provenance.source = ProvenanceSource::Ai;
        provenance.tool_run_id = Some(tool.id);
        provenance.provider = tool.output.as_ref().and_then(|value| {
            value
                .pointer("/provider_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
        provenance.model = tool.output.as_ref().and_then(|value| {
            value
                .pointer("/model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
        provenance.confidence = tool.output.as_ref().and_then(|value| {
            value
                .pointer("/confidence")
                .and_then(serde_json::Value::as_f64)
        });
        insert_provenance(&mut tx, &provenance).await?;
        let mut event = AnimalEvent::new(
            measurement.lab_id,
            measurement.animal_id,
            AnimalEventKind::MeasurementRecorded {
                measurement_id: measurement.id,
            },
            measurement.measured_at,
            measurement.meta.created_at,
        );
        event.project_id = Some(measurement.project_id);
        event.recorded_by = audit.actor.user_id;
        append_derived_animal_event(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }
}
