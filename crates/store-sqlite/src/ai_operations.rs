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
use serde_json::Value;
use sqlx::{Row, Sqlite, sqlite::SqliteRow};
use uuid::Uuid;

use super::{
    GENOTYPING_RECORD_COLUMNS, SqliteStore, append_derived_animal_event_tx, encode,
    insert_measurement_tx, insert_provenance_tx, map_sqlx, meta, optional_uuid, snapshot, uuid,
    write_audit,
};

const CONVERSATION_COLUMNS: &str = "id, lab_id, project_id, user_id, title, model_profile_id, model_profile_version, legacy_read_only, pinned_at, archived_at, created_at, updated_at, deleted_at, revision";
const MESSAGE_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, source_refs_json, created_at, updated_at, deleted_at, revision";
const TOOL_RUN_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision";
const APPROVAL_COLUMNS: &str = "id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision";
const AUTONOMY_GRANT_COLUMNS: &str = "id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision";
const MAX_AI_TURN_TOOL_RUNS: usize = 128;

fn parse_json(value: &str) -> StoreResult<Value> {
    serde_json::from_str(value).map_err(|error| StoreError::Serialization(error.to_string()))
}

fn conversation_from_row(row: &SqliteRow) -> StoreResult<AiConversation> {
    Ok(AiConversation {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        title: row.try_get("title").map_err(map_sqlx)?,
        model_profile: match (
            optional_uuid(row.try_get("model_profile_id").map_err(map_sqlx)?)?,
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
        legacy_read_only: row
            .try_get::<i64, _>("legacy_read_only")
            .map_err(map_sqlx)?
            != 0,
        pinned_at: row.try_get("pinned_at").map_err(map_sqlx)?,
        archived_at: row.try_get("archived_at").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn message_from_row(row: &SqliteRow) -> StoreResult<AiConversationMessage> {
    let message = AiConversationMessage {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        conversation_id: uuid(row.try_get("conversation_id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        sequence: row.try_get("sequence").map_err(map_sqlx)?,
        role: super::decode(row.try_get("role").map_err(map_sqlx)?)?,
        content: row.try_get("content").map_err(map_sqlx)?,
        response: row
            .try_get::<Option<String>, _>("response_json")
            .map_err(map_sqlx)?
            .as_deref()
            .map(parse_json)
            .transpose()?,
        source_refs: serde_json::from_str(
            row.try_get::<String, _>("source_refs_json")
                .map_err(map_sqlx)?
                .as_str(),
        )
        .map_err(|error| StoreError::Serialization(error.to_string()))?,
        meta: meta(row)?,
    };
    validate_message(&message)?;
    Ok(message)
}

fn tool_run_from_row(row: &SqliteRow) -> StoreResult<ToolRun> {
    Ok(ToolRun {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        conversation_id: optional_uuid(row.try_get("conversation_id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        tool_name: row.try_get("tool_name").map_err(map_sqlx)?,
        input: parse_json(row.try_get("input_json").map_err(map_sqlx)?)?,
        output: row
            .try_get::<Option<String>, _>("output_json")
            .map_err(map_sqlx)?
            .as_deref()
            .map(parse_json)
            .transpose()?,
        status: super::decode(row.try_get("status").map_err(map_sqlx)?)?,
        source: super::decode(row.try_get("source").map_err(map_sqlx)?)?,
        started_at: row.try_get("started_at").map_err(map_sqlx)?,
        completed_at: row.try_get("completed_at").map_err(map_sqlx)?,
        error: row.try_get("error").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn approval_from_row(row: &SqliteRow) -> StoreResult<Approval> {
    Ok(Approval {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        tool_run_id: uuid(row.try_get("tool_run_id").map_err(map_sqlx)?)?,
        requested_diff: parse_json(row.try_get("requested_diff_json").map_err(map_sqlx)?)?,
        decision: super::decode(row.try_get("decision").map_err(map_sqlx)?)?,
        decided_by: optional_uuid(row.try_get("decided_by").map_err(map_sqlx)?)?,
        decided_at: row.try_get("decided_at").map_err(map_sqlx)?,
        reason: row.try_get("reason").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

fn autonomy_grant_from_row(row: &SqliteRow) -> StoreResult<AiAutonomyGrant> {
    Ok(AiAutonomyGrant {
        id: uuid(row.try_get("id").map_err(map_sqlx)?)?,
        conversation_id: uuid(row.try_get("conversation_id").map_err(map_sqlx)?)?,
        lab_id: uuid(row.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(row.try_get("project_id").map_err(map_sqlx)?)?,
        user_id: uuid(row.try_get("user_id").map_err(map_sqlx)?)?,
        session_id: optional_uuid(row.try_get("session_id").map_err(map_sqlx)?)?,
        mode: super::decode(row.try_get("mode").map_err(map_sqlx)?)?,
        allowed_categories: serde_json::from_str(
            row.try_get::<String, _>("allowed_categories_json")
                .map_err(map_sqlx)?
                .as_str(),
        )
        .map_err(|error| StoreError::Serialization(error.to_string()))?,
        batch_limit: row.try_get::<i64, _>("batch_limit").map_err(map_sqlx)? as u32,
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
        || value.title.chars().any(char::is_control)
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

async fn validate_conversation_references_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    if let Some(project_id) = conversation.project_id {
        let project_lab: String =
            sqlx::query_scalar("SELECT lab_id FROM projects WHERE id = ? AND deleted_at IS NULL")
                .bind(project_id.to_string())
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_sqlx)?
                .ok_or(StoreError::NotFound {
                    entity: "project",
                    id: project_id,
                })?;
        if uuid(&project_lab)? != conversation.lab_id {
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
    let owner: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT p.lab_id, p.user_id, p.archived_at
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = ?
         WHERE p.id = ? AND p.deleted_at IS NULL",
    )
    .bind(binding.profile_version)
    .bind(binding.profile_id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    let Some((lab_id, user_id, archived_at)) = owner else {
        return Err(StoreError::Validation(
            "AI conversation model profile version does not exist".to_owned(),
        ));
    };
    if uuid(&lab_id)? != conversation.lab_id
        || uuid(&user_id)? != conversation.user_id
        || archived_at.is_some()
    {
        return Err(StoreError::Validation(
            "AI conversation model profile is unavailable to this user".to_owned(),
        ));
    }
    Ok(())
}

async fn insert_conversation_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO ai_conversations (id, lab_id, project_id, user_id, title, model_profile_id, model_profile_version, legacy_read_only, pinned_at, archived_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(conversation.id.to_string())
        .bind(conversation.lab_id.to_string())
        .bind(conversation.project_id.map(|id| id.to_string()))
        .bind(conversation.user_id.to_string())
        .bind(&conversation.title)
        .bind(conversation.model_profile.map(|binding| binding.profile_id.to_string()))
        .bind(conversation.model_profile.map(|binding| binding.profile_version))
        .bind(conversation.legacy_read_only)
        .bind(conversation.pinned_at)
        .bind(conversation.archived_at)
        .bind(conversation.meta.created_at)
        .bind(conversation.meta.updated_at)
        .bind(conversation.meta.deleted_at)
        .bind(conversation.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

async fn insert_autonomy_grant_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    grant: &AiAutonomyGrant,
) -> StoreResult<()> {
    let categories = serde_json::to_string(&grant.allowed_categories)
        .map_err(|error| StoreError::Serialization(error.to_string()))?;
    sqlx::query("INSERT INTO ai_autonomy_grants (id, conversation_id, lab_id, project_id, user_id, session_id, mode, allowed_categories_json, batch_limit, step_up_verified_at, last_used_at, expires_at, revoked_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(grant.id.to_string())
        .bind(grant.conversation_id.to_string())
        .bind(grant.lab_id.to_string())
        .bind(grant.project_id.map(|id| id.to_string()))
        .bind(grant.user_id.to_string())
        .bind(grant.session_id.map(|id| id.to_string()))
        .bind(encode(&grant.mode)?)
        .bind(categories)
        .bind(i64::from(grant.batch_limit))
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

async fn insert_message_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    value: &AiConversationMessage,
    audit: &AuditContext,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO ai_conversation_messages (id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, source_refs_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(value.id.to_string())
        .bind(value.conversation_id.to_string())
        .bind(value.lab_id.to_string())
        .bind(value.project_id.map(|id| id.to_string()))
        .bind(value.user_id.to_string())
        .bind(value.sequence)
        .bind(encode(&value.role)?)
        .bind(&value.content)
        .bind(value.response.as_ref().map(Value::to_string))
        .bind(encode(&value.source_refs)?)
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
    let decision_state_valid = match value.decision {
        ApprovalDecision::Pending => value.decided_by.is_none() && value.decided_at.is_none(),
        ApprovalDecision::Approved | ApprovalDecision::Rejected => {
            value.decided_by.is_some() && value.decided_at.is_some()
        }
    };
    if !decision_state_valid
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

async fn insert_tool_run_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    tool_run: &ToolRun,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_tool_run(tool_run)?;
    sqlx::query("INSERT INTO ai_tool_runs (id, conversation_id, lab_id, project_id, user_id, tool_name, input_json, output_json, status, source, started_at, completed_at, error, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(tool_run.id.to_string())
        .bind(tool_run.conversation_id.map(|id| id.to_string()))
        .bind(tool_run.lab_id.to_string())
        .bind(tool_run.project_id.map(|id| id.to_string()))
        .bind(tool_run.user_id.to_string())
        .bind(&tool_run.tool_name)
        .bind(tool_run.input.to_string())
        .bind(tool_run.output.as_ref().map(Value::to_string))
        .bind(encode(&tool_run.status)?)
        .bind(encode(&tool_run.source)?)
        .bind(tool_run.started_at)
        .bind(tool_run.completed_at)
        .bind(&tool_run.error)
        .bind(tool_run.meta.created_at)
        .bind(tool_run.meta.updated_at)
        .bind(tool_run.meta.deleted_at)
        .bind(tool_run.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        tool_run.lab_id,
        tool_run.project_id,
        EntityType::ToolRun,
        tool_run.id,
        AuditAction::Create,
        audit,
        None,
        Some(
            ai_source_ref_safe_audit_snapshot(tool_run)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        ),
    )
    .await
}

async fn insert_approval_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    approval: &Approval,
    tool_run: &ToolRun,
    audit: &AuditContext,
) -> StoreResult<()> {
    validate_approval(approval)?;
    if approval.tool_run_id != tool_run.id
        || tool_run.status != ToolRunStatus::AwaitingApproval
        || approval.decision != ApprovalDecision::Pending
    {
        return Err(StoreError::Validation(
            "only an awaiting AI tool run can request approval".to_owned(),
        ));
    }
    sqlx::query("INSERT INTO ai_approvals (id, tool_run_id, requested_diff_json, decision, decided_by, decided_at, reason, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(approval.id.to_string())
        .bind(approval.tool_run_id.to_string())
        .bind(approval.requested_diff.to_string())
        .bind(encode(&approval.decision)?)
        .bind(approval.decided_by.map(|id| id.to_string()))
        .bind(approval.decided_at)
        .bind(&approval.reason)
        .bind(approval.meta.created_at)
        .bind(approval.meta.updated_at)
        .bind(approval.meta.deleted_at)
        .bind(approval.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    write_audit(
        tx,
        tool_run.lab_id,
        tool_run.project_id,
        EntityType::Approval,
        approval.id,
        AuditAction::Create,
        audit,
        None,
        Some(snapshot(approval)?),
    )
    .await
}

async fn conversation_in_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    id: Uuid,
) -> StoreResult<AiConversation> {
    let row = sqlx::query(&format!(
        "SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id.to_string())
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
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
    let profile_id: Option<String> = sqlx::query_scalar(
        "SELECT p.id
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = ?
         WHERE p.id = ?
           AND p.lab_id = ?
           AND p.user_id = ?
           AND p.archived_at IS NULL
           AND p.deleted_at IS NULL",
    )
    .bind(binding.profile_version)
    .bind(binding.profile_id.to_string())
    .bind(conversation.lab_id.to_string())
    .bind(conversation.user_id.to_string())
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

async fn ensure_conversation_writable(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    conversation: &AiConversation,
) -> StoreResult<()> {
    if conversation.archived_at.is_some() {
        return Err(StoreError::Conflict(
            "archived AI conversation is read-only".to_owned(),
        ));
    }
    ensure_conversation_model_available(tx, conversation).await
}

async fn ensure_tool_conversation(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    tool_run: &ToolRun,
    require_writable: bool,
) -> StoreResult<()> {
    let Some(conversation_id) = tool_run.conversation_id else {
        return Ok(());
    };
    let conversation = conversation_in_tx(tx, conversation_id).await?;
    if conversation.lab_id != tool_run.lab_id
        || conversation.user_id != tool_run.user_id
        || conversation.project_id != tool_run.project_id
    {
        return Err(StoreError::Validation(
            "AI tool run scope differs from its conversation".to_owned(),
        ));
    }
    if require_writable {
        ensure_conversation_writable(tx, &conversation).await?;
    }
    Ok(())
}

async fn tool_run_in_tx(tx: &mut sqlx::Transaction<'_, Sqlite>, id: Uuid) -> StoreResult<ToolRun> {
    let row = sqlx::query(&format!(
        "SELECT {TOOL_RUN_COLUMNS} FROM ai_tool_runs WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_tool_run",
        id,
    })?;
    tool_run_from_row(&row)
}

async fn approval_in_tx(tx: &mut sqlx::Transaction<'_, Sqlite>, id: Uuid) -> StoreResult<Approval> {
    let row = sqlx::query(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM ai_approvals WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_approval",
        id,
    })?;
    approval_from_row(&row)
}

pub(crate) async fn update_resolution_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
    ensure_tool_conversation(
        tx,
        &before_tool,
        approval.decision != ApprovalDecision::Rejected,
    )
    .await?;
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
    let output_json = tool_run.output.as_ref().map(Value::to_string);
    let updated_tool = sqlx::query("UPDATE ai_tool_runs SET output_json = ?, status = ?, completed_at = ?, error = ?, updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
        .bind(output_json)
        .bind(encode(&tool_run.status)?)
        .bind(tool_run.completed_at)
        .bind(&tool_run.error)
        .bind(tool_run.meta.updated_at)
        .bind(tool_run.meta.deleted_at)
        .bind(tool_run.meta.revision)
        .bind(tool_run.id.to_string())
        .bind(expected_tool_revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
    let updated_approval = sqlx::query("UPDATE ai_approvals SET requested_diff_json = ?, decision = ?, decided_by = ?, decided_at = ?, reason = ?, updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
        .bind(approval.requested_diff.to_string())
        .bind(encode(&approval.decision)?)
        .bind(approval.decided_by.map(|id| id.to_string()))
        .bind(approval.decided_at)
        .bind(&approval.reason)
        .bind(approval.meta.updated_at)
        .bind(approval.meta.deleted_at)
        .bind(approval.meta.revision)
        .bind(approval.id.to_string())
        .bind(expected_approval_revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
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
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
impl AiOperationStore for SqliteStore {
    async fn create_ai_conversation(
        &self,
        conversation: &AiConversation,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        validate_conversation(conversation)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        validate_conversation_references_tx(&mut tx, conversation).await?;
        insert_conversation_tx(&mut tx, conversation).await?;
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
        validate_conversation_references_tx(&mut tx, conversation).await?;
        insert_conversation_tx(&mut tx, conversation).await?;
        insert_autonomy_grant_tx(&mut tx, grant).await?;
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
        let row = sqlx::query(&format!("SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE id = ? AND deleted_at IS NULL"))
            .bind(id.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?
            .ok_or(StoreError::NotFound { entity: "ai_conversation", id })?;
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
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(format!(
            "SELECT {CONVERSATION_COLUMNS} FROM ai_conversations WHERE lab_id = "
        ));
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND user_id = ")
            .push_bind(filter.user_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND project_id = ")
                .push_bind(project_id.to_string());
        }
        if let Some(title_query) = title_query {
            query
                .push(" AND instr(lower(title), lower(")
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
            query.push("(pinned_at IS NULL) ASC, pinned_at DESC, ");
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
            "UPDATE ai_conversations SET title = ?, pinned_at = ?, archived_at = ?, updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(&updated.title)
        .bind(updated.pinned_at)
        .bind(updated.archived_at)
        .bind(updated.meta.updated_at)
        .bind(updated.meta.revision)
        .bind(updated.id.to_string())
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
        ensure_conversation_writable(&mut tx, &before).await?;
        if before.lab_id != user_message.lab_id
            || before.project_id != user_message.project_id
            || before.user_id != user_message.user_id
        {
            return Err(StoreError::Validation(
                "AI message scope differs from its conversation".to_owned(),
            ));
        }
        let actual_last: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(sequence), 0) FROM ai_conversation_messages WHERE conversation_id = ? AND deleted_at IS NULL",
        )
        .bind(before.id.to_string())
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
            insert_tool_run_tx(&mut tx, tool_run, audit).await?;
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
            insert_approval_tx(&mut tx, approval, tool_run, audit).await?;
        }
        insert_message_tx(&mut tx, user_message, audit).await?;
        insert_message_tx(&mut tx, assistant_message, audit).await?;
        let mut updated = before.clone();
        updated.meta.touch(assistant_message.meta.created_at);
        let result = sqlx::query("UPDATE ai_conversations SET updated_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
            .bind(updated.meta.updated_at)
            .bind(updated.meta.revision)
            .bind(updated.id.to_string())
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
            "SELECT {MESSAGE_COLUMNS} FROM ai_conversation_messages WHERE conversation_id = ? AND deleted_at IS NULL ORDER BY sequence DESC LIMIT ?"
        ))
        .bind(conversation_id.to_string())
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
            "SELECT {AUTONOMY_GRANT_COLUMNS} FROM ai_autonomy_grants WHERE conversation_id = ? AND deleted_at IS NULL"
        ))
        .bind(conversation_id.to_string())
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
        ensure_conversation_writable(&mut tx, &conversation).await?;
        if conversation.lab_id != grant.lab_id
            || conversation.project_id != grant.project_id
            || conversation.user_id != grant.user_id
        {
            return Err(StoreError::Validation(
                "AI autonomy grant scope differs from its conversation".to_owned(),
            ));
        }
        let before_row = sqlx::query(&format!(
            "SELECT {AUTONOMY_GRANT_COLUMNS} FROM ai_autonomy_grants WHERE conversation_id = ? AND deleted_at IS NULL"
        ))
        .bind(grant.conversation_id.to_string())
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
        let categories = serde_json::to_string(&grant.allowed_categories)
            .map_err(|error| StoreError::Serialization(error.to_string()))?;
        if before.is_some() {
            let result = sqlx::query("UPDATE ai_autonomy_grants SET session_id = ?, mode = ?, allowed_categories_json = ?, batch_limit = ?, step_up_verified_at = ?, last_used_at = ?, expires_at = ?, revoked_at = ?, updated_at = ?, deleted_at = ?, revision = ? WHERE id = ? AND revision = ? AND deleted_at IS NULL")
                .bind(grant.session_id.map(|id| id.to_string())).bind(encode(&grant.mode)?).bind(categories)
                .bind(i64::from(grant.batch_limit)).bind(grant.step_up_verified_at).bind(grant.last_used_at)
                .bind(grant.expires_at).bind(grant.revoked_at).bind(grant.meta.updated_at).bind(grant.meta.deleted_at)
                .bind(grant.meta.revision).bind(grant.id.to_string()).bind(expected_revision)
                .execute(&mut *tx).await.map_err(map_sqlx)?;
            if result.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "AI autonomy grant revision changed".to_owned(),
                ));
            }
        } else {
            insert_autonomy_grant_tx(&mut tx, grant).await?;
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

    async fn create_tool_run(&self, tool_run: &ToolRun, audit: &AuditContext) -> StoreResult<()> {
        validate_tool_run(tool_run)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        ensure_tool_conversation(&mut tx, tool_run, true).await?;
        insert_tool_run_tx(&mut tx, tool_run, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_tool_run(&self, id: Uuid) -> StoreResult<ToolRun> {
        let row = sqlx::query(&format!(
            "SELECT {TOOL_RUN_COLUMNS} FROM ai_tool_runs WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_tool_run",
            id,
        })?;
        tool_run_from_row(&row)
    }

    async fn create_approval(&self, approval: &Approval, audit: &AuditContext) -> StoreResult<()> {
        validate_approval(approval)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let tool_run = tool_run_in_tx(&mut tx, approval.tool_run_id).await?;
        ensure_tool_conversation(&mut tx, &tool_run, true).await?;
        if tool_run.status != ToolRunStatus::AwaitingApproval
            || approval.decision != ApprovalDecision::Pending
        {
            return Err(StoreError::Validation(
                "only an awaiting AI tool run can request approval".to_owned(),
            ));
        }
        insert_approval_tx(&mut tx, approval, &tool_run, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn get_approval(&self, id: Uuid) -> StoreResult<Approval> {
        let row = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM ai_approvals WHERE id = ? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
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
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "SELECT a.id, a.tool_run_id, a.requested_diff_json, a.decision, a.decided_by, a.decided_at, a.reason, a.created_at, a.updated_at, a.deleted_at, a.revision FROM ai_approvals a JOIN ai_tool_runs t ON t.id = a.tool_run_id WHERE t.lab_id = ",
        );
        query
            .push_bind(filter.lab_id.to_string())
            .push(" AND t.user_id = ")
            .push_bind(filter.user_id.to_string())
            .push(" AND a.deleted_at IS NULL AND t.deleted_at IS NULL");
        if let Some(project_id) = filter.project_id {
            query
                .push(" AND t.project_id = ")
                .push_bind(project_id.to_string());
        }
        if let Some(decision) = filter.decision {
            query
                .push(" AND a.decision = ")
                .push_bind(encode(&decision)?);
        }
        query.push(" ORDER BY a.created_at DESC, a.id");
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.iter().map(approval_from_row).collect()
    }

    async fn finalize_ai_draft(
        &self,
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let valid_resolution = matches!(
            (approval.decision, tool_run.status),
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
            tool_run,
            expected_tool_run_revision,
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
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        if approval.decision != ApprovalDecision::Approved
            || tool_run.status != ToolRunStatus::Completed
            || measurement.status != RecordStatus::Draft
            || measurement.signed_by.is_some()
            || measurement.signed_at.is_some()
            || tool_run.lab_id != measurement.lab_id
            || tool_run.project_id != Some(measurement.project_id)
        {
            return Err(StoreError::Validation(
                "invalid approved AI measurement draft".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let actual_animal_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM animals WHERE id = ? AND lab_id = ? AND deleted_at IS NULL",
        )
        .bind(measurement.animal_id.to_string())
        .bind(measurement.lab_id.to_string())
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
            tool_run,
            expected_tool_run_revision,
            approval,
            expected_approval_revision,
            audit,
        )
        .await?;
        insert_measurement_tx(&mut tx, measurement, audit, AuditAction::Create).await?;
        let mut provenance = Provenance::from_audit(
            measurement.lab_id,
            Some(measurement.project_id),
            EntityType::Measurement,
            measurement.id,
            audit,
            measurement.meta.created_at,
        );
        provenance.source = ProvenanceSource::Ai;
        provenance.tool_run_id = Some(tool_run.id);
        provenance.provider = tool_run.output.as_ref().and_then(|value| {
            value
                .pointer("/provider_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
        provenance.model = tool_run.output.as_ref().and_then(|value| {
            value
                .pointer("/model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
        provenance.confidence = tool_run.output.as_ref().and_then(|value| {
            value
                .pointer("/confidence")
                .and_then(serde_json::Value::as_f64)
        });
        insert_provenance_tx(&mut tx, &provenance).await?;
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
        append_derived_animal_event_tx(&mut tx, &event, audit).await?;
        tx.commit().await.map_err(map_sqlx)
    }

    async fn apply_ai_experiment_grouping_draft(
        &self,
        application: &AiExperimentGroupingApplication,
        tool_run: &ToolRun,
        expected_tool_run_revision: i64,
        approval: &Approval,
        expected_approval_revision: i64,
        audit: &AuditContext,
    ) -> StoreResult<Vec<Participation>> {
        validate_grouping_application(application, tool_run, approval)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let project_locked = sqlx::query(
            "UPDATE projects SET updated_at = updated_at WHERE id = ? AND lab_id = ? AND status = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(application.project_id.to_string())
        .bind(application.lab_id.to_string())
        .bind(encode(&ProjectStatus::Active)?)
        .bind(application.expected_project_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if project_locked.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "project changed after experiment grouping draft creation".to_owned(),
            ));
        }
        let experiment_locked = sqlx::query(
            "UPDATE experiments SET updated_at = updated_at WHERE id = ? AND lab_id = ? AND project_id = ? AND status IN (?, ?) AND revision = ? AND deleted_at IS NULL",
        )
        .bind(application.experiment_id.to_string())
        .bind(application.lab_id.to_string())
        .bind(application.project_id.to_string())
        .bind(encode(&ExperimentStatus::Draft)?)
        .bind(encode(&ExperimentStatus::Active)?)
        .bind(application.expected_experiment_revision)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if experiment_locked.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "experiment changed after grouping draft creation".to_owned(),
            ));
        }
        let grouped_animal_ids = application
            .participations
            .iter()
            .map(|participation| participation.animal_id)
            .collect::<BTreeSet<_>>();
        for expected in &application.expected_animal_revisions {
            let animal_locked = sqlx::query(
                "UPDATE animals SET updated_at = updated_at WHERE id = ? AND lab_id = ? AND revision = ? AND deleted_at IS NULL",
            )
            .bind(expected.animal_id.to_string())
            .bind(application.lab_id.to_string())
            .bind(expected.expected_revision)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            if animal_locked.rows_affected() != 1 {
                return Err(StoreError::Conflict(
                    "animal revision changed after experiment grouping draft creation".to_owned(),
                ));
            }
            let assigned: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM project_animal_assignments WHERE project_id = ? AND animal_id = ? AND deleted_at IS NULL",
            )
            .bind(application.project_id.to_string())
            .bind(expected.animal_id.to_string())
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
                    "SELECT COUNT(*) FROM experiment_participations WHERE experiment_id = ? AND animal_id = ? AND deleted_at IS NULL",
                )
                .bind(application.experiment_id.to_string())
                .bind(expected.animal_id.to_string())
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
            let mut query = sqlx::QueryBuilder::<Sqlite>::new(
                "SELECT animal_id, measurement_id, revision FROM (SELECT m.animal_id, m.id AS measurement_id, m.revision, ROW_NUMBER() OVER (PARTITION BY m.animal_id ORDER BY m.measured_at DESC, m.id DESC) AS row_number FROM measurements m WHERE m.deleted_at IS NULL AND m.value_number IS NOT NULL AND lower(m.measurement_key) IN ('weight', 'body_weight') AND m.project_id = ",
            );
            query
                .push_bind(application.project_id.to_string())
                .push(" AND m.animal_id IN (");
            {
                let mut separated = query.separated(", ");
                for expected in &application.expected_latest_weights {
                    separated.push_bind(expected.animal_id.to_string());
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
                        uuid(row.try_get("animal_id").map_err(map_sqlx)?)?,
                        (
                            uuid(row.try_get("measurement_id").map_err(map_sqlx)?)?,
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
            tool_run,
            expected_tool_run_revision,
            approval,
            expected_approval_revision,
            audit,
        )
        .await?;

        for cohort in &application.cohorts {
            sqlx::query(
                "INSERT INTO cohorts (id, experiment_id, name, description, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(cohort.id.to_string())
            .bind(cohort.experiment_id.to_string())
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
            insert_provenance_tx(
                &mut tx,
                &ai_grouping_provenance(
                    application,
                    EntityType::Cohort,
                    cohort.id,
                    tool_run,
                    audit,
                    cohort.meta.created_at,
                ),
            )
            .await?;
        }

        let mut applied = Vec::with_capacity(application.participations.len());
        for proposed in &application.participations {
            let rows = sqlx::query(&format!(
                "SELECT {GENOTYPING_RECORD_COLUMNS} FROM genotyping_records WHERE animal_id = ? AND deleted_at IS NULL AND voided_at IS NULL ORDER BY created_at, id"
            ))
            .bind(proposed.animal_id.to_string())
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
            sqlx::query(
                "INSERT INTO experiment_participations (id, experiment_id, animal_id, cohort_id, status, enrolled_at, exited_at, genotype_snapshot_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(participation.id.to_string())
            .bind(participation.experiment_id.to_string())
            .bind(participation.animal_id.to_string())
            .bind(participation.cohort_id.map(|id| id.to_string()))
            .bind(encode(&participation.status)?)
            .bind(participation.enrolled_at)
            .bind(participation.exited_at)
            .bind(serde_json::to_string(&participation.genotype_snapshot).map_err(|error| StoreError::Serialization(error.to_string()))?)
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
            insert_provenance_tx(
                &mut tx,
                &ai_grouping_provenance(
                    application,
                    EntityType::Participation,
                    participation.id,
                    tool_run,
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
            append_derived_animal_event_tx(&mut tx, &event, audit).await?;
            applied.push(participation);
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(applied)
    }
}
