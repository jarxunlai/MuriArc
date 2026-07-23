use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use muriarc_core::{
    ActorType, AiAutonomyGrant, AiConversation, AiConversationArchiveFilter, AiConversationFilter,
    AiConversationMessage, AiConversationMessageRole, AiConversationUpdate,
    AiExperimentGroupingApplication, AiOperationStore, AnimalEvent, AnimalEventKind, Approval,
    ApprovalDecision, AuditAction, AuditContext, EntityType, ExperimentStatus,
    GenotypeSnapshotEntry, GenotypingRecord, Measurement, Participation, ParticipationStatus,
    ProjectStatus, Provenance, ProvenanceSource, RecordMeta, RecordStatus, StoreError, StoreResult,
    ToolRun, ToolRunStatus, WriteSource, ai_source_ref_safe_audit_snapshot,
};
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};
use uuid::Uuid;

use super::{
    GENOTYPING_RECORD_COLUMNS, PgTransaction, PostgresStore, append_derived_animal_event, encode,
    insert_measurement, insert_provenance, lock_and_reject_duplicate_measurement, map_sqlx, meta,
    snapshot, validate_measurement_relationships, write_audit,
};

const CONVERSATION_COLUMNS: &str = "id, lab_id, project_id, user_id, title, pinned_at, archived_at, created_at, updated_at, deleted_at, revision";
const MESSAGE_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, source_refs_json, created_at, updated_at, deleted_at, revision";
const TOOL_RUN_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision";
const APPROVAL_COLUMNS: &str = "id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision";
const AUTONOMY_GRANT_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision";
const MAX_AI_TURN_TOOL_RUNS: usize = 128;

fn conversation_from_row(row: &PgRow) -> StoreResult<AiConversation> {
    Ok(AiConversation {
        id: row.try_get("id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        title: row.try_get("title").map_err(map_sqlx)?,
        pinned_at: row.try_get("pinned_at").map_err(map_sqlx)?,
        archived_at: row.try_get("archived_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn message_from_row(row: &PgRow) -> StoreResult<AiConversationMessage> {
    let message = AiConversationMessage {
        id: row.try_get("id").map_err(map_sqlx)?,
        conversation_id: row.try_get("conversation_id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        user_id: row.try_get("user_id").map_err(map_sqlx)?,
        sequence: row.try_get("sequence").map_err(map_sqlx)?,
        role: super::decode(row.try_get("role").map_err(map_sqlx)?)?,
        content: row.try_get("content").map_err(map_sqlx)?,
        response: row.try_get("response_json").map_err(map_sqlx)?,
        source_refs: serde_json::from_value(row.try_get("source_refs_json").map_err(map_sqlx)?)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        meta: meta(row)?,
    };
    validate_message(&message)?;
    Ok(message)
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
    if value.id.is_nil()
        || value.conversation_id.is_nil()
        || value.lab_id.is_nil()
        || value.user_id.is_nil()
        || value.allowed_categories.is_empty()
        || value.batch_limit != value.mode.batch_limit()
        || value.batch_limit > 100
        || (value.mode == muriarc_core::AiAutonomyMode::Full && value.expires_at.is_none())
    {
        return Err(StoreError::Validation(
            "invalid AI autonomy grant".to_owned(),
        ));
    }
    Ok(())
}

fn validate_conversation(value: &AiConversation) -> StoreResult<()> {
    if value.title.trim().is_empty()
        || value.title.chars().count() > 256
        || value.title.chars().any(char::is_control)
    {
        return Err(StoreError::Validation(
            "AI conversation title must contain 1-256 characters".to_owned(),
        ));
    }
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
    sqlx::query("INSERT INTO ai_conversation_messages (id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, source_refs_json, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
        .bind(value.id)
        .bind(value.conversation_id)
        .bind(value.lab_id)
        .bind(value.project_id)
        .bind(value.user_id)
        .bind(value.sequence)
        .bind(encode(&value.role)?)
        .bind(&value.content)
        .bind(&value.response)
        .bind(
            serde_json::to_value(&value.source_refs)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        )
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
        Some(
            ai_source_ref_safe_audit_snapshot(value)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        ),
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

fn has_initial_meta(meta: &RecordMeta) -> bool {
    meta.revision == 1 && meta.deleted_at.is_none() && meta.created_at == meta.updated_at
}

fn validate_ai_turn_records(
    user_message: &AiConversationMessage,
    assistant_message: &AiConversationMessage,
    tool_runs: &[ToolRun],
    approvals: &[Approval],
    expected_last_sequence: i64,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_message(user_message)?;
    validate_message(assistant_message)?;
    if expected_last_sequence < 0
        || user_message.role != AiConversationMessageRole::User
        || assistant_message.role != AiConversationMessageRole::Assistant
        || assistant_message.sequence != user_message.sequence.checked_add(1).unwrap_or(i64::MIN)
        || user_message.id == assistant_message.id
        || user_message.conversation_id != assistant_message.conversation_id
        || user_message.lab_id != assistant_message.lab_id
        || user_message.project_id != assistant_message.project_id
        || user_message.user_id != assistant_message.user_id
        || !has_initial_meta(&user_message.meta)
        || !has_initial_meta(&assistant_message.meta)
        || audit.actor.actor_type != ActorType::Ai
        || audit.actor.user_id != Some(user_message.user_id)
        || audit.source != WriteSource::Ai
    {
        return Err(StoreError::Validation(
            "AI turn messages must be one contiguous owner-scoped user/assistant pair".to_owned(),
        ));
    }
    if tool_runs.len() > MAX_AI_TURN_TOOL_RUNS
        || approvals.len() > MAX_AI_TURN_TOOL_RUNS
        || approvals.len() > tool_runs.len()
    {
        return Err(StoreError::Validation(
            "AI turn contains too many tool or approval records".to_owned(),
        ));
    }

    let mut tool_ids = BTreeSet::new();
    let mut awaiting_tool_ids = BTreeSet::new();
    for tool_run in tool_runs {
        validate_tool_run(tool_run)?;
        let timestamps_valid = tool_run.started_at.is_some()
            && tool_run
                .started_at
                .zip(tool_run.completed_at)
                .is_none_or(|(started_at, completed_at)| completed_at >= started_at);
        let state_valid = tool_run.output.is_some()
            && tool_run.source == WriteSource::Ai
            && match tool_run.status {
                ToolRunStatus::AwaitingApproval => {
                    tool_run.completed_at.is_none() && tool_run.error.is_none()
                }
                ToolRunStatus::Completed => {
                    tool_run.completed_at.is_some() && tool_run.error.is_none()
                }
                ToolRunStatus::Pending
                | ToolRunStatus::Running
                | ToolRunStatus::Failed
                | ToolRunStatus::Cancelled => false,
            };
        if tool_run.id.is_nil()
            || tool_run.conversation_id != Some(user_message.conversation_id)
            || tool_run.lab_id != user_message.lab_id
            || tool_run.project_id != user_message.project_id
            || tool_run.user_id != user_message.user_id
            || !has_initial_meta(&tool_run.meta)
            || !timestamps_valid
            || !state_valid
            || !tool_ids.insert(tool_run.id)
        {
            return Err(StoreError::Validation(
                "invalid or duplicate AI turn tool run".to_owned(),
            ));
        }
        if tool_run.status == ToolRunStatus::AwaitingApproval {
            awaiting_tool_ids.insert(tool_run.id);
        }
    }

    let mut approval_ids = BTreeSet::new();
    let mut approval_tool_ids = BTreeSet::new();
    for approval in approvals {
        validate_approval(approval)?;
        if approval.id.is_nil()
            || approval.tool_run_id.is_nil()
            || approval.decision != ApprovalDecision::Pending
            || !has_initial_meta(&approval.meta)
            || !approval_ids.insert(approval.id)
            || !tool_ids.contains(&approval.tool_run_id)
            || !approval_tool_ids.insert(approval.tool_run_id)
        {
            return Err(StoreError::Validation(
                "invalid or duplicate AI turn approval".to_owned(),
            ));
        }
    }
    if approval_tool_ids != awaiting_tool_ids {
        return Err(StoreError::Validation(
            "every awaiting AI tool run must have exactly one approval".to_owned(),
        ));
    }
    Ok(())
}

async fn insert_tool_run(
    tx: &mut PgTransaction<'_>,
    value: &ToolRun,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_tool_run(value)?;
    sqlx::query("INSERT INTO ai_tool_runs (id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
        .bind(value.id)
        .bind(value.conversation_id)
        .bind(value.lab_id)
        .bind(value.project_id)
        .bind(value.user_id)
        .bind(&value.tool_name)
        .bind(&value.input)
        .bind(&value.output)
        .bind(encode(&value.status)?)
        .bind(encode(&value.source)?)
        .bind(value.started_at)
        .bind(value.completed_at)
        .bind(&value.error)
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
        EntityType::ToolRun,
        value.id,
        AuditAction::Create,
        audit,
        None,
        Some(
            ai_source_ref_safe_audit_snapshot(value)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        ),
    )
    .await
}

async fn insert_approval(
    tx: &mut PgTransaction<'_>,
    value: &Approval,
    tool_run: &ToolRun,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_approval(value)?;
    if value.tool_run_id != tool_run.id
        || tool_run.status != ToolRunStatus::AwaitingApproval
        || value.decision != ApprovalDecision::Pending
    {
        return Err(StoreError::Validation(
            "only an awaiting AI tool run can request approval".to_owned(),
        ));
    }
    sqlx::query("INSERT INTO ai_approvals (id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(value.id)
        .bind(value.tool_run_id)
        .bind(&value.requested_diff)
        .bind(encode(&value.decision)?)
        .bind(value.decided_by)
        .bind(value.decided_at)
        .bind(&value.reason)
        .bind(value.meta.created_at)
        .bind(value.meta.updated_at)
        .bind(value.meta.deleted_at)
        .bind(value.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        tool_run.lab_id,
        tool_run.project_id,
        EntityType::Approval,
        value.id,
        AuditAction::Create,
        audit,
        None,
        Some(snapshot(value)?),
    )
    .await
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

pub(crate) async fn update_resolution_tx(
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
    if before_tool.meta.revision != expected_tool_revision
        || before_approval.meta.revision != expected_approval_revision
        || before_approval.tool_run_id != before_tool.id
        || approval.tool_run_id != tool_run.id
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
        Some(
            ai_source_ref_safe_audit_snapshot(&before_tool)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        ),
        Some(
            ai_source_ref_safe_audit_snapshot(tool_run)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        ),
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

pub(crate) async fn validate_resolution_replay_tx(
    tx: &mut PgTransaction<'_>,
    tool_run: &ToolRun,
    approval: &Approval,
) -> StoreResult<()> {
    validate_tool_run(tool_run)?;
    validate_approval(approval)?;
    let stored_tool_run = tool_run_in_tx(tx, tool_run.id).await?;
    let stored_approval = approval_in_tx(tx, approval.id).await?;
    if stored_tool_run != *tool_run || stored_approval != *approval {
        return Err(StoreError::Conflict(
            "AI import resolution does not match the completed replay".to_owned(),
        ));
    }
    Ok(())
}

fn validate_grouping_application(
    application: &AiExperimentGroupingApplication,
    tool_run: &ToolRun,
    approval: &Approval,
) -> StoreResult<()> {
    let cohort_ids = application
        .cohorts
        .iter()
        .map(|cohort| cohort.id)
        .collect::<BTreeSet<_>>();
    let participation_ids = application
        .participations
        .iter()
        .map(|participation| participation.id)
        .collect::<BTreeSet<_>>();
    let animal_ids = application
        .participations
        .iter()
        .map(|participation| participation.animal_id)
        .collect::<BTreeSet<_>>();
    let revision_ids = application
        .expected_animal_revisions
        .iter()
        .map(|value| value.animal_id)
        .collect::<BTreeSet<_>>();
    let weight_ids = application
        .expected_latest_weights
        .iter()
        .map(|value| value.animal_id)
        .collect::<BTreeSet<_>>();
    let valid_hash = application.input_snapshot_sha256.len() == 64
        && application
            .input_snapshot_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if approval.decision != ApprovalDecision::Approved
        || tool_run.status != ToolRunStatus::Completed
        || tool_run.tool_name != "experiment_grouping_draft"
        || tool_run.lab_id != application.lab_id
        || tool_run.project_id != Some(application.project_id)
        || application.lab_id.is_nil()
        || application.project_id.is_nil()
        || application.experiment_id.is_nil()
        || application.expected_project_revision <= 0
        || application.expected_experiment_revision <= 0
        || !(2..=20).contains(&application.cohorts.len())
        || application.participations.is_empty()
        || application.participations.len() > 200
        || application.expected_animal_revisions.len() > 200
        || cohort_ids.len() != application.cohorts.len()
        || participation_ids.len() != application.participations.len()
        || animal_ids.len() != application.participations.len()
        || revision_ids.len() != application.expected_animal_revisions.len()
        || !animal_ids.is_subset(&revision_ids)
        || application.expected_latest_weights.len() > 200
        || weight_ids.len() != application.expected_latest_weights.len()
        || (!weight_ids.is_empty() && weight_ids != revision_ids)
        || !valid_hash
        || application.cohorts.iter().any(|cohort| {
            cohort.experiment_id != application.experiment_id
                || cohort.meta.revision != 1
                || cohort.meta.deleted_at.is_some()
                || cohort.name.trim().is_empty()
        })
        || application.participations.iter().any(|participation| {
            participation.experiment_id != application.experiment_id
                || participation.status != ParticipationStatus::Enrolled
                || participation
                    .cohort_id
                    .is_none_or(|id| !cohort_ids.contains(&id))
                || !participation.genotype_snapshot.is_empty()
                || participation.meta.revision != 1
                || participation.meta.deleted_at.is_some()
        })
        || application
            .expected_animal_revisions
            .iter()
            .any(|value| value.animal_id.is_nil() || value.expected_revision <= 0)
        || application.expected_latest_weights.iter().any(|value| {
            value.animal_id.is_nil()
                || value.measurement_id.is_some() != value.expected_revision.is_some()
                || value.measurement_id.is_some_and(|id| id.is_nil())
                || value
                    .expected_revision
                    .is_some_and(|revision| revision <= 0)
        })
    {
        return Err(StoreError::Validation(
            "invalid approved AI experiment grouping draft".to_owned(),
        ));
    }
    Ok(())
}

fn ai_grouping_provenance(
    application: &AiExperimentGroupingApplication,
    entity_type: EntityType,
    entity_id: Uuid,
    tool_run: &ToolRun,
    audit: &AuditContext,
    recorded_at: chrono::DateTime<chrono::Utc>,
) -> Provenance {
    let mut provenance = Provenance::from_audit(
        application.lab_id,
        Some(application.project_id),
        entity_type,
        entity_id,
        audit,
        recorded_at,
    );
    provenance.source = ProvenanceSource::Ai;
    provenance.tool_run_id = Some(tool_run.id);
    provenance
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
        if let Some(project_id) = value.project_id {
            let project_lab: Uuid = sqlx::query_scalar(
                "SELECT lab_id FROM projects WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "project",
                id: project_id,
            })?;
            if project_lab != value.lab_id {
                return Err(StoreError::Validation(
                    "AI conversation project belongs to another lab".to_owned(),
                ));
            }
        }
        sqlx::query("INSERT INTO ai_conversations (id, lab_id, project_id, user_id, title, pinned_at, archived_at, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(value.id).bind(value.lab_id).bind(value.project_id).bind(value.user_id).bind(&value.title)
            .bind(value.pinned_at).bind(value.archived_at).bind(value.meta.created_at).bind(value.meta.updated_at)
            .bind(value.meta.deleted_at).bind(value.meta.revision)
            .execute(&mut *tx).await.map_err(map_sqlx)?;
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
        let title_query = filter
            .title_query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty());
        if title_query
            .is_some_and(|query| query.chars().count() > 256 || query.chars().any(char::is_control))
        {
            return Err(StoreError::Validation(
                "AI conversation title query must contain at most 256 characters".to_owned(),
            ));
        }
        let mut query = QueryBuilder::<Postgres>::new(format!(
            "SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE lab_id = "
        ));
        query
            .push_bind(filter.lab_id)
            .push(" AND user_id = ")
            .push_bind(filter.user_id)
            .push(" AND deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query.push(" AND project_id = ").push_bind(project_id);
        }
        if let Some(title_query) = title_query {
            query
                .push(" AND strpos(lower(title), lower(")
                .push_bind(title_query)
                .push(")) > 0");
        }
        match filter.archive {
            AiConversationArchiveFilter::Active => {
                query.push(" AND archived_at IS NULL");
            }
            AiConversationArchiveFilter::Archived => {
                query.push(" AND archived_at IS NOT NULL");
            }
            AiConversationArchiveFilter::All => {}
        }
        query.push(" ORDER BY ");
        if filter.pinned_first {
            query.push("pinned_at DESC NULLS LAST, ");
        }
        query
            .push("updated_at DESC, id DESC LIMIT ")
            .push_bind(i64::from(limit))
            .push(" OFFSET ")
            .push_bind(i64::from(offset));
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(conversation_from_row).collect()
    }

    async fn update_ai_conversation(
        &self,
        update: &AiConversationUpdate,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation> {
        if update.id.is_nil() || update.expected_revision <= 0 {
            return Err(StoreError::Validation(
                "invalid AI conversation update".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let before = conversation_in_tx(&mut tx, update.id).await?;
        if audit.actor.user_id != Some(before.user_id) {
            return Err(StoreError::Validation(
                "AI conversation update actor must match its owner".to_owned(),
            ));
        }
        if before.meta.revision != update.expected_revision {
            return Err(StoreError::Conflict(
                "AI conversation changed before the update was applied".to_owned(),
            ));
        }
        let mut updated = before.clone();
        updated
            .apply_change(&update.change, update.updated_at)
            .map_err(|error| StoreError::Validation(error.to_string()))?;
        let result = sqlx::query(
            "UPDATE ai_conversations SET title = $1, pinned_at = $2, archived_at = $3, updated_at = $4, revision = $5 WHERE id = $6 AND revision = $7 AND deleted_at IS NULL",
        )
        .bind(&updated.title)
        .bind(updated.pinned_at)
        .bind(updated.archived_at)
        .bind(updated.meta.updated_at)
        .bind(updated.meta.revision)
        .bind(updated.id)
        .bind(update.expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI conversation revision changed during update".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            updated.lab_id,
            updated.project_id,
            EntityType::AiConversation,
            updated.id,
            if matches!(&update.change, muriarc_core::AiConversationChange::Archive) {
                AuditAction::Archive
            } else {
                AuditAction::Update
            },
            audit,
            Some(snapshot(&before)?),
            Some(snapshot(&updated)?),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(updated)
    }

    async fn append_ai_turn_records(
        &self,
        user_message: &AiConversationMessage,
        assistant_message: &AiConversationMessage,
        tool_runs: &[ToolRun],
        approvals: &[Approval],
        expected_last_sequence: i64,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation> {
        validate_ai_turn_records(
            user_message,
            assistant_message,
            tool_runs,
            approvals,
            expected_last_sequence,
            audit,
        )?;

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let before = conversation_in_tx(&mut tx, user_message.conversation_id).await?;
        if before.archived_at.is_some() {
            return Err(StoreError::Conflict(
                "archived AI conversations cannot accept new turns".to_owned(),
            ));
        }
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

        for tool_run in tool_runs {
            insert_tool_run(&mut tx, tool_run, audit).await?;
        }
        for approval in approvals {
            let tool_run = tool_runs
                .iter()
                .find(|tool_run| tool_run.id == approval.tool_run_id)
                .ok_or_else(|| {
                    StoreError::Validation(
                        "AI turn approval references an unknown tool run".to_owned(),
                    )
                })?;
            insert_approval(&mut tx, approval, tool_run, audit).await?;
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

    async fn append_ai_turn_messages(
        &self,
        user_message: &AiConversationMessage,
        assistant_message: &AiConversationMessage,
        expected_last_sequence: i64,
        audit: &AuditContext,
    ) -> StoreResult<AiConversation> {
        self.append_ai_turn_records(
            user_message,
            assistant_message,
            &[],
            &[],
            expected_last_sequence,
            audit,
        )
        .await
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
        if conversation.archived_at.is_some() {
            return Err(StoreError::Conflict(
                "archived AI conversations cannot change autonomy".to_owned(),
            ));
        }
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
            sqlx::query("INSERT INTO ai_autonomy_grants (id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
                .bind(grant.id).bind(grant.conversation_id).bind(grant.lab_id).bind(grant.project_id).bind(grant.user_id)
                .bind(grant.session_id).bind(encode(&grant.mode)?).bind(categories).bind(grant.batch_limit as i32)
                .bind(grant.step_up_verified_at).bind(grant.last_used_at).bind(grant.expires_at).bind(grant.revoked_at)
                .bind(grant.meta.created_at).bind(grant.meta.updated_at).bind(grant.meta.deleted_at).bind(grant.meta.revision)
                .execute(&mut *tx).await.map_err(map_sqlx)?;
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
        if let Some(conversation_id) = value.conversation_id {
            let conversation = conversation_in_tx(&mut tx, conversation_id).await?;
            if conversation.lab_id != value.lab_id
                || conversation.user_id != value.user_id
                || conversation.project_id != value.project_id
            {
                return Err(StoreError::Validation(
                    "AI tool run scope differs from its conversation".to_owned(),
                ));
            }
        }
        insert_tool_run(&mut tx, value, audit).await?;
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
        if tool.status != ToolRunStatus::AwaitingApproval
            || value.decision != ApprovalDecision::Pending
        {
            return Err(StoreError::Validation(
                "only an awaiting AI tool run can request approval".to_owned(),
            ));
        }
        insert_approval(&mut tx, value, &tool, audit).await?;
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

    async fn apply_ai_experiment_grouping_draft(
        &self,
        application: &AiExperimentGroupingApplication,
        tool: &ToolRun,
        expected_tool_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<Vec<Participation>> {
        validate_grouping_application(application, tool, approval)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let project_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM projects WHERE id = $1 AND lab_id = $2 AND status = $3 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(application.project_id)
        .bind(application.lab_id)
        .bind(encode(&ProjectStatus::Active)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "project",
            id: application.project_id,
        })?;
        if project_revision != application.expected_project_revision {
            return Err(StoreError::Conflict(
                "project changed after experiment grouping draft creation".to_owned(),
            ));
        }
        let experiment_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM experiments WHERE id = $1 AND lab_id = $2 AND project_id = $3 AND status IN ($4, $5) AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(application.experiment_id)
        .bind(application.lab_id)
        .bind(application.project_id)
        .bind(encode(&ExperimentStatus::Draft)?)
        .bind(encode(&ExperimentStatus::Active)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "experiment",
            id: application.experiment_id,
        })?;
        if experiment_revision != application.expected_experiment_revision {
            return Err(StoreError::Conflict(
                "experiment changed after grouping draft creation".to_owned(),
            ));
        }
        let mut expected_animals = application.expected_animal_revisions.clone();
        expected_animals.sort_by_key(|value| value.animal_id);
        let grouped_animal_ids = application
            .participations
            .iter()
            .map(|participation| participation.animal_id)
            .collect::<BTreeSet<_>>();
        for expected in &expected_animals {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("genotype-snapshot:{}", expected.animal_id))
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            let animal_revision: i64 = sqlx::query_scalar(
                "SELECT revision FROM animals WHERE id = $1 AND lab_id = $2 AND deleted_at IS NULL FOR UPDATE",
            )
            .bind(expected.animal_id)
            .bind(application.lab_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "animal",
                id: expected.animal_id,
            })?;
            if animal_revision != expected.expected_revision {
                return Err(StoreError::Conflict(
                    "animal revision changed after experiment grouping draft creation".to_owned(),
                ));
            }
            let assigned: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM project_animal_assignments WHERE project_id = $1 AND animal_id = $2 AND deleted_at IS NULL",
            )
            .bind(application.project_id)
            .bind(expected.animal_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if assigned != 1 {
                return Err(StoreError::Conflict(
                    "animal is no longer assigned to the grouping project".to_owned(),
                ));
            }
            if grouped_animal_ids.contains(&expected.animal_id) {
                let enrolled: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM experiment_participations WHERE experiment_id = $1 AND animal_id = $2 AND deleted_at IS NULL",
                )
                .bind(application.experiment_id)
                .bind(expected.animal_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx)?;
                if enrolled != 0 {
                    return Err(StoreError::Conflict(
                        "animal already participates in the grouping experiment".to_owned(),
                    ));
                }
            }
        }
        if !application.expected_latest_weights.is_empty() {
            let mut query = QueryBuilder::<Postgres>::new(
                "SELECT animal_id, measurement_id, revision FROM (SELECT m.animal_id, m.id AS measurement_id, m.revision, ROW_NUMBER() OVER (PARTITION BY m.animal_id ORDER BY m.measured_at DESC, m.id DESC) AS row_number FROM measurements m WHERE m.deleted_at IS NULL AND m.value_number IS NOT NULL AND lower(m.measurement_key) IN ('weight', 'body_weight') AND m.project_id = ",
            );
            query
                .push_bind(application.project_id)
                .push(" AND m.animal_id IN (");
            {
                let mut separated = query.separated(", ");
                for expected in &application.expected_latest_weights {
                    separated.push_bind(expected.animal_id);
                }
                separated.push_unseparated(") ) ranked WHERE row_number = 1");
            }
            let actual = query
                .build()
                .fetch_all(&mut *tx)
                .await
                .map_err(map_sqlx)?
                .into_iter()
                .map(|row| {
                    Ok((
                        row.try_get::<Uuid, _>("animal_id").map_err(map_sqlx)?,
                        (
                            row.try_get::<Uuid, _>("measurement_id").map_err(map_sqlx)?,
                            row.try_get::<i64, _>("revision").map_err(map_sqlx)?,
                        ),
                    ))
                })
                .collect::<StoreResult<BTreeMap<_, _>>>()?;
            let expected = application
                .expected_latest_weights
                .iter()
                .filter_map(|value| {
                    value
                        .measurement_id
                        .zip(value.expected_revision)
                        .map(|snapshot| (value.animal_id, snapshot))
                })
                .collect::<BTreeMap<_, _>>();
            if actual != expected {
                return Err(StoreError::Conflict(
                    "latest weight changed after experiment grouping draft creation".to_owned(),
                ));
            }
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

        for cohort in &application.cohorts {
            sqlx::query(
                "INSERT INTO cohorts (id, experiment_id, name, description, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(cohort.id)
            .bind(cohort.experiment_id)
            .bind(&cohort.name)
            .bind(&cohort.description)
            .bind(cohort.meta.created_at)
            .bind(cohort.meta.updated_at)
            .bind(cohort.meta.deleted_at)
            .bind(cohort.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                application.lab_id,
                Some(application.project_id),
                EntityType::Cohort,
                cohort.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(cohort)?),
            )
            .await?;
            insert_provenance(
                &mut tx,
                &ai_grouping_provenance(
                    application,
                    EntityType::Cohort,
                    cohort.id,
                    tool,
                    audit,
                    cohort.meta.created_at,
                ),
            )
            .await?;
        }

        let mut applied = Vec::with_capacity(application.participations.len());
        for proposed in &application.participations {
            let rows = sqlx::query(&format!(
                "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE animal_id = $1 AND deleted_at IS NULL AND voided_at IS NULL ORDER BY created_at, id"
            ))
            .bind(proposed.animal_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            let mut latest = BTreeMap::<Uuid, GenotypingRecord>::new();
            for row in &rows {
                let record = super::genotyping_record_from_row(row)?;
                latest.insert(record.genotype_definition_id, record);
            }
            let mut participation = proposed.clone();
            participation.genotype_snapshot = latest
                .into_values()
                .map(|record| GenotypeSnapshotEntry {
                    genotyping_record_id: record.id,
                    genotype_definition_id: record.genotype_definition_id,
                    state: record.state,
                    assessed_at: record.assessed_at,
                })
                .collect();
            let genotype_snapshot = serde_json::to_value(&participation.genotype_snapshot)
                .map_err(|error| StoreError::Serialization(error.to_string()))?;
            sqlx::query(
                "INSERT INTO experiment_participations (id, experiment_id, animal_id, cohort_id, status, enrolled_at, exited_at, genotype_snapshot_json, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(participation.id)
            .bind(participation.experiment_id)
            .bind(participation.animal_id)
            .bind(participation.cohort_id)
            .bind(encode(&participation.status)?)
            .bind(participation.enrolled_at)
            .bind(participation.exited_at)
            .bind(genotype_snapshot)
            .bind(participation.meta.created_at)
            .bind(participation.meta.updated_at)
            .bind(participation.meta.deleted_at)
            .bind(participation.meta.revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                application.lab_id,
                Some(application.project_id),
                EntityType::Participation,
                participation.id,
                AuditAction::Create,
                audit,
                None,
                Some(snapshot(&participation)?),
            )
            .await?;
            insert_provenance(
                &mut tx,
                &ai_grouping_provenance(
                    application,
                    EntityType::Participation,
                    participation.id,
                    tool,
                    audit,
                    participation.meta.created_at,
                ),
            )
            .await?;
            let mut event = AnimalEvent::new(
                application.lab_id,
                participation.animal_id,
                AnimalEventKind::ExperimentEnrolled {
                    participation_id: participation.id,
                },
                participation.enrolled_at,
                participation.meta.created_at,
            );
            event.project_id = Some(application.project_id);
            event.recorded_by = audit.actor.user_id;
            append_derived_animal_event(&mut tx, &event, audit).await?;
            applied.push(participation);
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(applied)
    }
}
