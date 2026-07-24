use super::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_core::*;
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};
use uuid::Uuid;
const LC: &str = "id,lab_id,project_id,attachment_id,target_type,target_id,created_by,created_at,updated_at,deleted_at,revision";
const DC: &str = "id,lab_id,project_id,attachment_id,kind,media_type,relative_path,size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision";
const IC: &str = "id,lab_id,user_id,conversation_id,attachment_id,project_id,status,last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision";
const SC: &str = "id,lab_id,user_id,conversation_id,project_id,attachment_id,kind,status,last_activity_at,expires_at,archived_at,error_code,created_at,updated_at,deleted_at,revision";
const XC: &str = "id,lab_id,user_id,project_id,experiment_id,experiment_event_id,private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,data_cell_definition_id,data_cell_subject_type,data_cell_subject_id,model_profile_id,model_profile_version,model_purpose,usage_input_tokens,usage_output_tokens,usage_total_tokens,provider_request_id,trace_json,status,items_json,error_code,created_at,updated_at,deleted_at,revision";
const EC: &str = "draft_id,display_order,private_image_id,private_attachment_id,promoted_attachment_id,original_sha256,sanitized_sha256,created_at,updated_at,revision";
type WritableAiConversationRow = (
    Uuid,
    Uuid,
    Option<Uuid>,
    Option<DateTime<Utc>>,
    bool,
    Option<Uuid>,
    Option<i64>,
);
fn safe_source_snapshot(source: &AiConversationSource) -> StoreResult<serde_json::Value> {
    ai_conversation_source_audit_snapshot(source)
        .map_err(|error| StoreError::Serialization(error.to_string()))
}
fn safe_source_attachment_snapshot(attachment: &Attachment) -> StoreResult<serde_json::Value> {
    ai_source_attachment_audit_snapshot(attachment)
        .map_err(|error| StoreError::Serialization(error.to_string()))
}
async fn writable_ai_conversation_scope(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    conversation_id: Uuid,
    expected_lab_id: Uuid,
    expected_user_id: Uuid,
) -> StoreResult<Option<Uuid>> {
    let row: Option<WritableAiConversationRow> = sqlx::query_as(
        "SELECT lab_id,user_id,project_id,archived_at,legacy_read_only,
                model_profile_id,model_profile_version
         FROM ai_conversations
         WHERE id=$1 AND deleted_at IS NULL
         FOR SHARE",
    )
    .bind(conversation_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    let Some((
        lab_id,
        user_id,
        project_id,
        archived_at,
        legacy_read_only,
        profile_id,
        profile_version,
    )) = row
    else {
        return Err(StoreError::NotFound {
            entity: "ai_conversation",
            id: conversation_id,
        });
    };
    if (lab_id, user_id) != (expected_lab_id, expected_user_id) {
        return Err(StoreError::Validation(
            "AI conversation scope does not match".to_owned(),
        ));
    }
    let (Some(profile_id), Some(profile_version)) = (profile_id, profile_version) else {
        return Err(StoreError::Conflict(
            "AI conversation model profile is unavailable".to_owned(),
        ));
    };
    if archived_at.is_some() || legacy_read_only {
        return Err(StoreError::Conflict(
            "AI conversation is read-only".to_owned(),
        ));
    }
    let available: Option<Uuid> = sqlx::query_scalar(
        "SELECT profile.id
         FROM ai_model_profiles profile
         JOIN ai_model_profile_versions profile_version
           ON profile_version.profile_id=profile.id AND profile_version.version=$1
         WHERE profile.id=$2
           AND profile.lab_id=$3
           AND profile.user_id=$4
           AND profile.archived_at IS NULL
           AND profile.deleted_at IS NULL
         FOR SHARE OF profile",
    )
    .bind(profile_version)
    .bind(profile_id)
    .bind(lab_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if available.is_none() {
        return Err(StoreError::Conflict(
            "AI conversation model profile is unavailable".to_owned(),
        ));
    }
    Ok(project_id)
}
fn rm(r: &PgRow) -> StoreResult<RecordMeta> {
    Ok(RecordMeta {
        created_at: r.try_get("created_at").map_err(map_sqlx)?,
        updated_at: r.try_get("updated_at").map_err(map_sqlx)?,
        deleted_at: r.try_get("deleted_at").map_err(map_sqlx)?,
        revision: r.try_get("revision").map_err(map_sqlx)?,
    })
}
fn lr(r: &PgRow) -> StoreResult<AttachmentLink> {
    Ok(AttachmentLink {
        id: r.try_get("id").map_err(map_sqlx)?,
        lab_id: r.try_get("lab_id").map_err(map_sqlx)?,
        project_id: r.try_get("project_id").map_err(map_sqlx)?,
        attachment_id: r.try_get("attachment_id").map_err(map_sqlx)?,
        target_type: decode(r.try_get("target_type").map_err(map_sqlx)?)?,
        target_id: r.try_get("target_id").map_err(map_sqlx)?,
        created_by: r.try_get("created_by").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
}
fn dr(r: &PgRow) -> StoreResult<AttachmentDerivative> {
    Ok(AttachmentDerivative {
        id: r.try_get("id").map_err(map_sqlx)?,
        lab_id: r.try_get("lab_id").map_err(map_sqlx)?,
        project_id: r.try_get("project_id").map_err(map_sqlx)?,
        attachment_id: r.try_get("attachment_id").map_err(map_sqlx)?,
        kind: decode(r.try_get("kind").map_err(map_sqlx)?)?,
        media_type: r.try_get("media_type").map_err(map_sqlx)?,
        relative_path: r.try_get("relative_path").map_err(map_sqlx)?,
        size_bytes: r.try_get("size_bytes").map_err(map_sqlx)?,
        sha256: r.try_get("sha256").map_err(map_sqlx)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
}
fn ir(r: &PgRow) -> StoreResult<PrivateAiImage> {
    Ok(PrivateAiImage {
        id: r.try_get("id").map_err(map_sqlx)?,
        lab_id: r.try_get("lab_id").map_err(map_sqlx)?,
        user_id: r.try_get("user_id").map_err(map_sqlx)?,
        conversation_id: r.try_get("conversation_id").map_err(map_sqlx)?,
        attachment_id: r.try_get("attachment_id").map_err(map_sqlx)?,
        project_id: r.try_get("project_id").map_err(map_sqlx)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        last_activity_at: r.try_get("last_activity_at").map_err(map_sqlx)?,
        expires_at: r.try_get("expires_at").map_err(map_sqlx)?,
        archived_at: r.try_get("archived_at").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
}
fn sr(r: &PgRow) -> StoreResult<AiConversationSource> {
    Ok(AiConversationSource {
        id: r.try_get("id").map_err(map_sqlx)?,
        lab_id: r.try_get("lab_id").map_err(map_sqlx)?,
        user_id: r.try_get("user_id").map_err(map_sqlx)?,
        conversation_id: r.try_get("conversation_id").map_err(map_sqlx)?,
        project_id: r.try_get("project_id").map_err(map_sqlx)?,
        attachment_id: r.try_get("attachment_id").map_err(map_sqlx)?,
        kind: decode(r.try_get("kind").map_err(map_sqlx)?)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        last_activity_at: r.try_get("last_activity_at").map_err(map_sqlx)?,
        expires_at: r.try_get("expires_at").map_err(map_sqlx)?,
        archived_at: r.try_get("archived_at").map_err(map_sqlx)?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
}
pub(crate) fn ai_conversation_source_from_row(row: &PgRow) -> StoreResult<AiConversationSource> {
    sr(row)
}
fn xr(r: &PgRow) -> StoreResult<AiExtractionDraft> {
    let data_cell_definition_id: Option<Uuid> =
        r.try_get("data_cell_definition_id").map_err(map_sqlx)?;
    let data_cell = match data_cell_definition_id {
        Some(definition_id) => Some(AiObservationDataCell {
            definition_id,
            subject_type: decode(
                &r.try_get::<Option<String>, _>("data_cell_subject_type")
                    .map_err(map_sqlx)?
                    .ok_or_else(|| {
                        StoreError::Serialization(
                            "extraction data cell subject type is missing".to_owned(),
                        )
                    })?,
            )?,
            subject_id: r
                .try_get::<Option<Uuid>, _>("data_cell_subject_id")
                .map_err(map_sqlx)?
                .ok_or_else(|| {
                    StoreError::Serialization(
                        "extraction data cell subject id is missing".to_owned(),
                    )
                })?,
        }),
        None => None,
    };
    let model_profile_id: Option<Uuid> = r.try_get("model_profile_id").map_err(map_sqlx)?;
    let model_trace = match model_profile_id {
        Some(profile_id) => Some(AiExtractionModelTrace {
            profile_id,
            profile_version: r
                .try_get::<Option<i64>, _>("model_profile_version")
                .map_err(map_sqlx)?
                .ok_or_else(|| {
                    StoreError::Serialization("extraction model version is missing".to_owned())
                })?,
            purpose: decode(
                &r.try_get::<Option<String>, _>("model_purpose")
                    .map_err(map_sqlx)?
                    .ok_or_else(|| {
                        StoreError::Serialization("extraction model purpose is missing".to_owned())
                    })?,
            )?,
            input_tokens: nonnegative_u64(
                r.try_get::<Option<i64>, _>("usage_input_tokens")
                    .map_err(map_sqlx)?
                    .ok_or_else(|| {
                        StoreError::Serialization("extraction input usage is missing".to_owned())
                    })?,
                "extraction input usage",
            )?,
            output_tokens: nonnegative_u64(
                r.try_get::<Option<i64>, _>("usage_output_tokens")
                    .map_err(map_sqlx)?
                    .ok_or_else(|| {
                        StoreError::Serialization("extraction output usage is missing".to_owned())
                    })?,
                "extraction output usage",
            )?,
            total_tokens: nonnegative_u64(
                r.try_get::<Option<i64>, _>("usage_total_tokens")
                    .map_err(map_sqlx)?
                    .ok_or_else(|| {
                        StoreError::Serialization("extraction total usage is missing".to_owned())
                    })?,
                "extraction total usage",
            )?,
            provider_request_id: r
                .try_get::<Option<String>, _>("provider_request_id")
                .map_err(map_sqlx)?,
            trace: r
                .try_get::<Option<Value>, _>("trace_json")
                .map_err(map_sqlx)?
                .ok_or_else(|| {
                    StoreError::Serialization("extraction trace is missing".to_owned())
                })?,
        }),
        None => None,
    };
    Ok(AiExtractionDraft {
        id: r.try_get("id").map_err(map_sqlx)?,
        lab_id: r.try_get("lab_id").map_err(map_sqlx)?,
        user_id: r.try_get("user_id").map_err(map_sqlx)?,
        project_id: r.try_get("project_id").map_err(map_sqlx)?,
        experiment_id: r.try_get("experiment_id").map_err(map_sqlx)?,
        experiment_event_id: r.try_get("experiment_event_id").map_err(map_sqlx)?,
        private_image_id: r.try_get("private_image_id").map_err(map_sqlx)?,
        attachment_id: r.try_get("attachment_id").map_err(map_sqlx)?,
        image_sha256: r.try_get("image_sha256").map_err(map_sqlx)?,
        provider: r.try_get("provider").map_err(map_sqlx)?,
        model: r.try_get("model").map_err(map_sqlx)?,
        tool_run_id: r.try_get("tool_run_id").map_err(map_sqlx)?,
        data_cell,
        evidence: Vec::new(),
        model_trace,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        items: serde_json::from_value(r.try_get("items_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
}

fn er(r: &PgRow) -> StoreResult<AiExtractionEvidence> {
    Ok(AiExtractionEvidence {
        display_order: r.try_get("display_order").map_err(map_sqlx)?,
        private_image_id: r.try_get("private_image_id").map_err(map_sqlx)?,
        private_attachment_id: r.try_get("private_attachment_id").map_err(map_sqlx)?,
        promoted_attachment_id: r.try_get("promoted_attachment_id").map_err(map_sqlx)?,
        original_sha256: r.try_get("original_sha256").map_err(map_sqlx)?,
        sanitized_sha256: r.try_get("sanitized_sha256").map_err(map_sqlx)?,
        meta: RecordMeta {
            created_at: r.try_get("created_at").map_err(map_sqlx)?,
            updated_at: r.try_get("updated_at").map_err(map_sqlx)?,
            deleted_at: None,
            revision: r.try_get("revision").map_err(map_sqlx)?,
        },
    })
}

fn nonnegative_u64(value: i64, field: &str) -> StoreResult<u64> {
    u64::try_from(value)
        .map_err(|_| StoreError::Serialization(format!("{field} must not be negative")))
}

fn postgres_i64(value: u64, field: &str) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::Validation(format!("{field} is too large")))
}

async fn load_evidence_postgres(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    draft_id: Uuid,
) -> StoreResult<Vec<AiExtractionEvidence>> {
    let rows = sqlx::query(&format!(
        "SELECT {EC} FROM ai_extraction_evidence WHERE draft_id=$1 ORDER BY display_order"
    ))
    .bind(draft_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(er).collect()
}

async fn load_evidence_postgres_pool(
    pool: &sqlx::PgPool,
    draft_id: Uuid,
) -> StoreResult<Vec<AiExtractionEvidence>> {
    let rows = sqlx::query(&format!(
        "SELECT {EC} FROM ai_extraction_evidence WHERE draft_id=$1 ORDER BY display_order"
    ))
    .bind(draft_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(er).collect()
}
async fn lab(tx: &mut sqlx::Transaction<'_, Postgres>, t: &str, id: Uuid) -> StoreResult<Uuid> {
    sqlx::query_scalar(&format!(
        "SELECT lab_id FROM {t} WHERE id=$1 AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "workspace_scope",
        id,
    })
}
async fn ia(tx: &mut sqlx::Transaction<'_, Postgres>, a: &Attachment) -> StoreResult<()> {
    sqlx::query("INSERT INTO attachments(id,lab_id,project_id,entity_type,entity_id,file_name,media_type,relative_path,size_bytes,sha256,version,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)").bind(a.id).bind(a.lab_id).bind(a.project_id).bind(&a.entity_type).bind(a.entity_id).bind(&a.file_name).bind(&a.media_type).bind(&a.relative_path).bind(a.size_bytes).bind(&a.sha256).bind(a.version).bind(a.meta.created_at).bind(a.meta.updated_at).bind(a.meta.deleted_at).bind(a.meta.revision).execute(&mut**tx).await.map_err(map_sqlx)?;
    Ok(())
}
async fn io(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    d: &AiExtractionDraft,
    it: &AiExtractionItem,
    a: &AuditContext,
) -> StoreResult<()> {
    let o = &it.observation;
    let v = &it.value;
    let experiment_status: String =
        sqlx::query_scalar("SELECT status FROM experiments WHERE id=$1 AND deleted_at IS NULL")
            .bind(o.experiment_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "experiment",
                id: o.experiment_id,
            })?;
    if matches!(experiment_status.as_str(), "completed" | "cancelled") {
        return Err(StoreError::Conflict(
            "AI extraction cannot write to a completed or cancelled experiment".to_owned(),
        ));
    }
    validate_observation_recorder(v, a)?;
    it.validate()
        .map_err(|e| StoreError::Validation(e.to_owned()))?;
    validate_observation_scope_postgres(tx, o.lab_id, o.project_id, o.experiment_id).await?;
    let n:i64=sqlx::query_scalar("SELECT count(*) FROM observations WHERE experiment_event_id=$1 AND definition_id=$2 AND subject_type=$3 AND subject_id=$4 AND deleted_at IS NULL").bind(o.experiment_event_id).bind(o.definition_id).bind(encode(&o.subject_type)?).bind(o.subject_id).fetch_one(&mut**tx).await.map_err(map_sqlx)?;
    if n != 0 {
        return Err(StoreError::Conflict(
            "AI extraction cannot overwrite an existing observation cell".to_owned(),
        ));
    }
    let es:Option<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT lab_id,project_id,experiment_id FROM experiment_events WHERE id=$1 AND deleted_at IS NULL").bind(o.experiment_event_id).fetch_optional(&mut**tx).await.map_err(map_sqlx)?;
    if es != Some((d.lab_id, d.project_id, d.experiment_id)) {
        return Err(StoreError::Validation(
            "extraction event scope is invalid".to_owned(),
        ));
    }
    let rr=sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE id=$1 AND deleted_at IS NULL")).bind(o.definition_id).fetch_optional(&mut**tx).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound{entity:"observation_definition",id:o.definition_id})?;
    let def = observation_definition_from_row(&rr)?;
    if (def.lab_id, def.project_id, def.experiment_id) != (d.lab_id, d.project_id, d.experiment_id)
    {
        return Err(StoreError::Validation(
            "extraction definition scope is invalid".to_owned(),
        ));
    }
    def.validate_value(&v.value)
        .map_err(|e| StoreError::Validation(e.to_string()))?;
    validate_observation_subject_postgres(tx, o).await?;
    sqlx::query("INSERT INTO observations(id,lab_id,project_id,experiment_id,experiment_event_id,definition_id,subject_type,subject_id,context_json,current_value_version,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)").bind(o.id).bind(o.lab_id).bind(o.project_id).bind(o.experiment_id).bind(o.experiment_event_id).bind(o.definition_id).bind(encode(&o.subject_type)?).bind(o.subject_id).bind(&o.context).bind(o.current_value_version).bind(o.meta.created_at).bind(o.meta.updated_at).bind(o.meta.deleted_at).bind(o.meta.revision).execute(&mut**tx).await.map_err(map_sqlx)?;
    insert_observation_value_postgres(tx, v).await?;
    for (et, id, af) in [
        (EntityType::Observation, o.id, snapshot(o)?),
        (EntityType::ObservationValue, v.id, snapshot(v)?),
    ] {
        write_audit(
            tx,
            d.lab_id,
            Some(d.project_id),
            et,
            id,
            AuditAction::Approve,
            a,
            None,
            Some(af),
        )
        .await?;
        insert_provenance(
            tx,
            &Provenance {
                id: Uuid::new_v4(),
                lab_id: d.lab_id,
                project_id: Some(d.project_id),
                entity_type: et,
                entity_id: id,
                source: ProvenanceSource::Ai,
                actor_user_id: a.actor.user_id,
                import_job_id: None,
                import_commit_id: None,
                tool_run_id: d.tool_run_id,
                provider: Some(d.provider.clone()),
                model: Some(d.model.clone()),
                confidence: Some(it.confidence),
                request_id: a.request_id.clone(),
                recorded_at: Utc::now(),
            },
        )
        .await?
    }
    Ok(())
}

async fn validate_versioned_extraction_postgres(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    draft: &AiExtractionDraft,
) -> StoreResult<()> {
    let Some(cell) = draft.data_cell.as_ref() else {
        return Ok(());
    };
    let trace = draft
        .model_trace
        .as_ref()
        .ok_or_else(|| StoreError::Validation("extraction model trace is required".to_owned()))?;
    let profile = sqlx::query(
        "SELECT p.lab_id,p.user_id,p.archived_at,p.deleted_at,v.model_id,v.supports_vision
             FROM ai_model_profiles p
             JOIN ai_model_profile_versions v ON v.profile_id=p.id
             WHERE p.id=$1 AND v.version=$2",
    )
    .bind(trace.profile_id)
    .bind(trace.profile_version)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    let Some(profile) = profile else {
        return Err(StoreError::NotFound {
            entity: "ai_model_profile_version",
            id: trace.profile_id,
        });
    };
    let profile_lab: Uuid = profile.try_get("lab_id").map_err(map_sqlx)?;
    let profile_user: Uuid = profile.try_get("user_id").map_err(map_sqlx)?;
    let archived_at: Option<DateTime<Utc>> = profile.try_get("archived_at").map_err(map_sqlx)?;
    let deleted_at: Option<DateTime<Utc>> = profile.try_get("deleted_at").map_err(map_sqlx)?;
    let model_id: String = profile.try_get("model_id").map_err(map_sqlx)?;
    let supports_vision: bool = profile.try_get("supports_vision").map_err(map_sqlx)?;
    if (profile_lab, profile_user) != (draft.lab_id, draft.user_id)
        || archived_at.is_some()
        || deleted_at.is_some()
        || !supports_vision
        || model_id != draft.model
    {
        return Err(StoreError::Validation(
            "extraction vision model binding is unavailable or out of scope".to_owned(),
        ));
    }
    let event: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT lab_id,project_id,experiment_id
         FROM experiment_events WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(draft.experiment_event_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if event.is_none() {
        return Err(StoreError::NotFound {
            entity: "experiment_event",
            id: draft.experiment_event_id,
        });
    }
    if event != Some((draft.lab_id, draft.project_id, draft.experiment_id)) {
        return Err(StoreError::Validation(
            "extraction event scope is invalid".to_owned(),
        ));
    }
    let definition: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT lab_id,project_id,experiment_id
         FROM observation_definitions WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(cell.definition_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if definition.is_none() {
        return Err(StoreError::NotFound {
            entity: "observation_definition",
            id: cell.definition_id,
        });
    }
    if definition != Some((draft.lab_id, draft.project_id, draft.experiment_id)) {
        return Err(StoreError::Validation(
            "extraction definition scope is invalid".to_owned(),
        ));
    }
    validate_observation_subject_postgres(tx, &draft.items[0].observation).await?;
    let existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM observations
         WHERE experiment_event_id=$1 AND definition_id=$2 AND subject_type=$3
           AND subject_id=$4 AND deleted_at IS NULL",
    )
    .bind(draft.experiment_event_id)
    .bind(cell.definition_id)
    .bind(encode(&cell.subject_type)?)
    .bind(cell.subject_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if existing != 0 {
        return Err(StoreError::Conflict(
            "AI extraction cannot target an existing observation cell".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_extraction_evidence_postgres(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    draft: &AiExtractionDraft,
    evidence: &AiExtractionEvidence,
) -> StoreResult<PrivateAiImage> {
    let row = sqlx::query(&format!(
        "SELECT {IC} FROM ai_private_images
         WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(evidence.private_image_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_private_image",
        id: evidence.private_image_id,
    })?;
    let image = ir(&row)?;
    let sha256: String =
        sqlx::query_scalar("SELECT sha256 FROM attachments WHERE id=$1 AND deleted_at IS NULL")
            .bind(evidence.private_attachment_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "attachment",
                id: evidence.private_attachment_id,
            })?;
    if (image.lab_id, image.user_id, image.attachment_id)
        != (draft.lab_id, draft.user_id, evidence.private_attachment_id)
        || image.project_id.is_some()
        || !matches!(
            image.status,
            PrivateImageStatus::Active | PrivateImageStatus::Processing
        )
        || sha256 != evidence.original_sha256
        || evidence.promoted_attachment_id.is_some()
    {
        return Err(StoreError::Validation(
            "extraction evidence owner, state, attachment, or original SHA is invalid".to_owned(),
        ));
    }
    Ok(image)
}

async fn promote_extraction_evidence_postgres(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    draft: &AiExtractionDraft,
    evidence: &mut AiExtractionEvidence,
    observations: &[Observation],
    actor_user_id: Uuid,
    now: DateTime<Utc>,
    audit: &AuditContext,
) -> StoreResult<(Attachment, Vec<AttachmentLink>)> {
    let image_row = sqlx::query(&format!(
        "SELECT {IC} FROM ai_private_images
         WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(evidence.private_image_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "ai_private_image",
        id: evidence.private_image_id,
    })?;
    let mut image = ir(&image_row)?;
    let attachment_row = sqlx::query(&format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM attachments
         WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(evidence.private_attachment_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "attachment",
        id: evidence.private_attachment_id,
    })?;
    let mut attachment = attachment_from_row(&attachment_row)?;
    if image.attachment_id != attachment.id
        || image.project_id.is_some()
        || image.status != PrivateImageStatus::PendingApproval
        || attachment.project_id.is_some()
        || attachment.entity_type != "ai_private_image"
        || attachment.sha256 != evidence.original_sha256
        || evidence.promoted_attachment_id.is_some()
    {
        return Err(StoreError::Conflict(
            "AI extraction evidence changed before approval".to_owned(),
        ));
    }
    let before_attachment = snapshot(&attachment)?;
    let before_image = snapshot(&image)?;
    let target_observation = observations.first().ok_or_else(|| {
        StoreError::Validation("AI extraction evidence requires an approved observation".to_owned())
    })?;
    attachment.project_id = Some(draft.project_id);
    attachment.entity_type = "observation".to_owned();
    attachment.entity_id = target_observation.id;
    attachment.meta.touch(now);
    sqlx::query(
        "UPDATE attachments
         SET project_id=$1,entity_type=$2,entity_id=$3,updated_at=$4,revision=$5
         WHERE id=$6",
    )
    .bind(draft.project_id)
    .bind(&attachment.entity_type)
    .bind(attachment.entity_id)
    .bind(attachment.meta.updated_at)
    .bind(attachment.meta.revision)
    .bind(attachment.id)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;

    image.project_id = Some(draft.project_id);
    image.status = PrivateImageStatus::Archived;
    image.last_activity_at = now;
    image.archived_at = Some(now);
    image.meta.touch(now);
    sqlx::query(
        "UPDATE ai_private_images
         SET project_id=$1,status='archived',last_activity_at=$2,archived_at=$3,
             updated_at=$4,revision=$5
         WHERE id=$6",
    )
    .bind(draft.project_id)
    .bind(now)
    .bind(now)
    .bind(image.meta.updated_at)
    .bind(image.meta.revision)
    .bind(image.id)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;

    evidence.promoted_attachment_id = Some(attachment.id);
    evidence.meta.touch(now);
    sqlx::query(
        "UPDATE ai_extraction_evidence
         SET promoted_attachment_id=$1,updated_at=$2,revision=$3
         WHERE draft_id=$4 AND display_order=$5",
    )
    .bind(attachment.id)
    .bind(evidence.meta.updated_at)
    .bind(evidence.meta.revision)
    .bind(draft.id)
    .bind(evidence.display_order)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;

    write_audit(
        tx,
        draft.lab_id,
        Some(draft.project_id),
        EntityType::Attachment,
        attachment.id,
        AuditAction::Approve,
        audit,
        Some(before_attachment),
        Some(snapshot(&attachment)?),
    )
    .await?;
    insert_provenance(
        tx,
        &Provenance::from_audit(
            draft.lab_id,
            Some(draft.project_id),
            EntityType::Attachment,
            attachment.id,
            audit,
            now,
        ),
    )
    .await?;
    write_audit(
        tx,
        draft.lab_id,
        Some(draft.project_id),
        EntityType::AiPrivateImage,
        image.id,
        AuditAction::Archive,
        audit,
        Some(before_image),
        Some(snapshot(&image)?),
    )
    .await?;
    insert_provenance(
        tx,
        &Provenance::from_audit(
            draft.lab_id,
            Some(draft.project_id),
            EntityType::AiPrivateImage,
            image.id,
            audit,
            now,
        ),
    )
    .await?;

    let mut links = Vec::with_capacity(observations.len());
    for observation in observations {
        let link = AttachmentLink {
            id: Uuid::new_v4(),
            lab_id: draft.lab_id,
            project_id: draft.project_id,
            attachment_id: attachment.id,
            target_type: AttachmentLinkTarget::DataCell,
            target_id: observation.id,
            created_by: actor_user_id,
            meta: RecordMeta::new(now),
        };
        sqlx::query(
            "INSERT INTO attachment_links(
                id,lab_id,project_id,attachment_id,target_type,target_id,created_by,
                created_at,updated_at,deleted_at,revision
             )VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(link.id)
        .bind(link.lab_id)
        .bind(link.project_id)
        .bind(link.attachment_id)
        .bind(encode(&link.target_type)?)
        .bind(link.target_id)
        .bind(link.created_by)
        .bind(link.meta.created_at)
        .bind(link.meta.updated_at)
        .bind(link.meta.deleted_at)
        .bind(link.meta.revision)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        write_audit(
            tx,
            draft.lab_id,
            Some(draft.project_id),
            EntityType::AttachmentLink,
            link.id,
            AuditAction::Link,
            audit,
            None,
            Some(snapshot(&link)?),
        )
        .await?;
        insert_provenance(
            tx,
            &Provenance::from_audit(
                draft.lab_id,
                Some(draft.project_id),
                EntityType::AttachmentLink,
                link.id,
                audit,
                now,
            ),
        )
        .await?;
        links.push(link);
    }
    Ok((attachment, links))
}
#[async_trait]
impl WorkspaceStore for PostgresStore {
    async fn create_attachment_link(
        &self,
        l: &AttachmentLink,
        a: &AuditContext,
    ) -> StoreResult<()> {
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        for (x, id) in [
            ("projects", l.project_id),
            ("attachments", l.attachment_id),
            ("users", l.created_by),
        ] {
            if lab(&mut t, x, id).await? != l.lab_id {
                return Err(StoreError::Validation(
                    "attachment link crosses labs".into(),
                ));
            }
        }
        sqlx::query("INSERT INTO attachment_links(id,lab_id,project_id,attachment_id,target_type,target_id,created_by,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(l.id).bind(l.lab_id).bind(l.project_id).bind(l.attachment_id).bind(encode(&l.target_type)?).bind(l.target_id).bind(l.created_by).bind(l.meta.created_at).bind(l.meta.updated_at).bind(l.meta.deleted_at).bind(l.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
        write_audit(
            &mut t,
            l.lab_id,
            Some(l.project_id),
            EntityType::AttachmentLink,
            l.id,
            AuditAction::Link,
            a,
            None,
            Some(snapshot(l)?),
        )
        .await?;
        t.commit().await.map_err(map_sqlx)
    }
    async fn list_attachment_links(&self, id: Uuid) -> StoreResult<Vec<AttachmentLink>> {
        let r=sqlx::query(&format!("SELECT {LC} FROM attachment_links WHERE attachment_id=$1 AND deleted_at IS NULL ORDER BY created_at,id")).bind(id).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter().map(lr).collect()
    }
    async fn create_attachment_derivative(
        &self,
        d: &AttachmentDerivative,
        a: &AuditContext,
    ) -> StoreResult<()> {
        if d.size_bytes.is_some_and(|v| v < 0)
            || d.sha256.as_ref().is_some_and(|v| v.len() != 64)
            || (d.status == DerivativeStatus::Ready
                && (d.relative_path.is_none() || d.sha256.is_none()))
        {
            return Err(StoreError::Validation("invalid derivative".into()));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        if lab(&mut t, "attachments", d.attachment_id).await? != d.lab_id {
            return Err(StoreError::Validation("derivative crosses labs".into()));
        }
        sqlx::query("INSERT INTO attachment_derivatives(id,lab_id,project_id,attachment_id,kind,media_type,relative_path,size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)").bind(d.id).bind(d.lab_id).bind(d.project_id).bind(d.attachment_id).bind(encode(&d.kind)?).bind(&d.media_type).bind(&d.relative_path).bind(d.size_bytes).bind(&d.sha256).bind(encode(&d.status)?).bind(&d.error_code).bind(d.meta.created_at).bind(d.meta.updated_at).bind(d.meta.deleted_at).bind(d.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
        write_audit(
            &mut t,
            d.lab_id,
            d.project_id,
            EntityType::AttachmentDerivative,
            d.id,
            AuditAction::Process,
            a,
            None,
            Some(snapshot(d)?),
        )
        .await?;
        t.commit().await.map_err(map_sqlx)
    }
    async fn list_attachment_derivatives(
        &self,
        id: Uuid,
    ) -> StoreResult<Vec<AttachmentDerivative>> {
        let r=sqlx::query(&format!("SELECT {DC} FROM attachment_derivatives WHERE attachment_id=$1 AND deleted_at IS NULL ORDER BY kind,id")).bind(id).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter().map(dr).collect()
    }
    async fn create_private_ai_image(
        &self,
        at: &Attachment,
        i: &PrivateAiImage,
        a: &AuditContext,
    ) -> StoreResult<()> {
        if at.id != i.attachment_id
            || at.lab_id != i.lab_id
            || at.entity_type != "ai_private_image"
            || at.entity_id != i.id
            || at.project_id.is_some()
            || i.project_id.is_some()
            || i.expires_at <= i.last_activity_at
            || at.sha256.len() != 64
            || at.size_bytes < 0
        {
            return Err(StoreError::Validation("invalid private image".into()));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        if lab(&mut t, "users", i.user_id).await? != i.lab_id {
            return Err(StoreError::Validation("private owner crosses labs".into()));
        }
        if let Some(c) = i.conversation_id {
            let conversation_project_id =
                writable_ai_conversation_scope(&mut t, c, i.lab_id, i.user_id).await?;
            if i.project_id.is_some() && i.project_id != conversation_project_id {
                return Err(StoreError::Validation(
                    "private image project does not match its conversation".into(),
                ));
            }
        }
        ia(&mut t, at).await?;
        sqlx::query("INSERT INTO ai_private_images(id,lab_id,user_id,conversation_id,attachment_id,project_id,status,last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)").bind(i.id).bind(i.lab_id).bind(i.user_id).bind(i.conversation_id).bind(i.attachment_id).bind(i.project_id).bind(encode(&i.status)?).bind(i.last_activity_at).bind(i.expires_at).bind(i.archived_at).bind(i.meta.created_at).bind(i.meta.updated_at).bind(i.meta.deleted_at).bind(i.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
        for (et, id, af) in [
            (EntityType::Attachment, at.id, snapshot(at)?),
            (EntityType::AiPrivateImage, i.id, snapshot(i)?),
        ] {
            write_audit(
                &mut t,
                i.lab_id,
                None,
                et,
                id,
                AuditAction::Create,
                a,
                None,
                Some(af),
            )
            .await?;
            insert_provenance(
                &mut t,
                &Provenance::from_audit(i.lab_id, None, et, id, a, i.meta.created_at),
            )
            .await?
        }
        t.commit().await.map_err(map_sqlx)
    }
    async fn get_private_ai_image(&self, id: Uuid) -> StoreResult<PrivateAiImage> {
        let r = sqlx::query(&format!(
            "SELECT {IC} FROM ai_private_images WHERE id=$1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_private_image",
            id,
        })?;
        ir(&r)
    }
    async fn list_private_ai_images(
        &self,
        f: &PrivateImageFilter,
    ) -> StoreResult<Vec<PrivateAiImage>> {
        let mut q = QueryBuilder::<Postgres>::new(format!(
            "SELECT {IC} FROM ai_private_images WHERE lab_id="
        ));
        q.push_bind(f.lab_id).push(" AND deleted_at IS NULL");
        if let Some(v) = f.user_id {
            q.push(" AND user_id=").push_bind(v);
        }
        if let Some(v) = f.conversation_id {
            q.push(" AND conversation_id=").push_bind(v);
        }
        if let Some(v) = f.project_id {
            q.push(" AND project_id=").push_bind(v);
        }
        if let Some(v) = f.status {
            q.push(" AND status=").push_bind(encode(&v)?);
        }
        q.push(" ORDER BY created_at DESC,id");
        let r = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter().map(ir).collect()
    }
    async fn archive_private_ai_image(
        &self,
        id: Uuid,
        p: Uuid,
        rev: i64,
        now: DateTime<Utc>,
        a: &AuditContext,
    ) -> StoreResult<PrivateAiImage> {
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let r = sqlx::query(&format!(
            "SELECT {IC} FROM ai_private_images WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_private_image",
            id,
        })?;
        let mut i = ir(&r)?;
        if i.meta.revision != rev {
            return Err(StoreError::Conflict(
                "private image revision changed".into(),
            ));
        }
        if matches!(
            i.status,
            PrivateImageStatus::Processing
                | PrivateImageStatus::PendingApproval
                | PrivateImageStatus::Expired
        ) {
            return Err(StoreError::Conflict("private image busy or expired".into()));
        }
        if let Some(conversation_id) = i.conversation_id {
            writable_ai_conversation_scope(&mut t, conversation_id, i.lab_id, i.user_id).await?;
        }
        if lab(&mut t, "projects", p).await? != i.lab_id {
            return Err(StoreError::Validation(
                "archive project crosses labs".into(),
            ));
        }
        let b = snapshot(&i)?;
        i.project_id = Some(p);
        i.status = PrivateImageStatus::Archived;
        i.archived_at = Some(now);
        i.last_activity_at = now;
        i.meta.touch(now);
        sqlx::query("UPDATE ai_private_images SET project_id=$1,status=$2,last_activity_at=$3,archived_at=$4,updated_at=$5,revision=$6 WHERE id=$7").bind(p).bind(encode(&i.status)?).bind(i.last_activity_at).bind(i.archived_at).bind(i.meta.updated_at).bind(i.meta.revision).bind(id).execute(&mut*t).await.map_err(map_sqlx)?;
        write_audit(
            &mut t,
            i.lab_id,
            None,
            EntityType::AiPrivateImage,
            id,
            AuditAction::Archive,
            a,
            Some(b),
            Some(snapshot(&i)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance::from_audit(i.lab_id, None, EntityType::AiPrivateImage, i.id, a, now),
        )
        .await?;
        t.commit().await.map_err(map_sqlx)?;
        Ok(i)
    }
    async fn private_ai_image_stats(
        &self,
        l: Uuid,
        n: DateTime<Utc>,
    ) -> StoreResult<Vec<PrivateImageStats>> {
        let r=sqlx::query("SELECT i.user_id,count(*) image_count,coalesce(sum(a.size_bytes),0)::bigint total_size_bytes,count(*)FILTER(WHERE i.status NOT IN('archived','expired')AND i.expires_at<=$2+interval '7 days')expiring_count,count(*)FILTER(WHERE i.status='failed')failed_count FROM ai_private_images i JOIN attachments a ON a.id=i.attachment_id WHERE i.lab_id=$1 AND i.deleted_at IS NULL GROUP BY i.user_id ORDER BY i.user_id").bind(l).bind(n).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter()
            .map(|x| {
                Ok(PrivateImageStats {
                    user_id: x.try_get("user_id").map_err(map_sqlx)?,
                    image_count: x.try_get("image_count").map_err(map_sqlx)?,
                    total_size_bytes: x.try_get("total_size_bytes").map_err(map_sqlx)?,
                    expiring_count: x.try_get("expiring_count").map_err(map_sqlx)?,
                    failed_count: x.try_get("failed_count").map_err(map_sqlx)?,
                })
            })
            .collect()
    }
    async fn create_ai_conversation_source(
        &self,
        at: &Attachment,
        s: &AiConversationSource,
        a: &AuditContext,
    ) -> StoreResult<()> {
        s.validate()
            .map_err(|error| StoreError::Validation(error.to_owned()))?;
        if at.id != s.attachment_id
            || at.lab_id != s.lab_id
            || at.entity_type != "ai_conversation_source"
            || at.entity_id != s.id
            || at.project_id.is_some()
            || at.sha256.len() != 64
            || at.size_bytes < 0
            || matches!(
                s.status,
                AiConversationSourceStatus::Archived | AiConversationSourceStatus::Expired
            )
        {
            return Err(StoreError::Validation(
                "invalid AI conversation source".to_owned(),
            ));
        }
        if at.size_bytes > MAX_ACTIVE_AI_CONVERSATION_SOURCE_BYTES_PER_OWNER {
            return Err(StoreError::Conflict(
                AI_CONVERSATION_SOURCE_QUOTA_EXCEEDED.to_owned(),
            ));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        // The owner row is the quota lock. Concurrent uploads for one owner
        // cannot both observe the same pre-insert count/byte total.
        let owner_lab: Option<Uuid> = sqlx::query_scalar(
            "SELECT lab_id FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(s.user_id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?;
        let owner_lab = owner_lab.ok_or(StoreError::NotFound {
            entity: "workspace_scope",
            id: s.user_id,
        })?;
        if owner_lab != s.lab_id {
            return Err(StoreError::Validation(
                "AI source owner crosses labs".to_owned(),
            ));
        }
        let quota: (i64, i64) = sqlx::query_as(
            "SELECT count(*)::bigint,coalesce(sum(attachment.size_bytes),0)::bigint
             FROM ai_conversation_sources source
             JOIN attachments attachment ON attachment.id=source.attachment_id
             WHERE source.lab_id=$1 AND source.user_id=$2
               AND source.deleted_at IS NULL
               AND source.status IN ('staged','ready','failed')",
        )
        .bind(s.lab_id)
        .bind(s.user_id)
        .fetch_one(&mut *t)
        .await
        .map_err(map_sqlx)?;
        if quota.0 >= MAX_ACTIVE_AI_CONVERSATION_SOURCES_PER_OWNER
            || quota.1
                > MAX_ACTIVE_AI_CONVERSATION_SOURCE_BYTES_PER_OWNER
                    .checked_sub(at.size_bytes)
                    .ok_or_else(|| {
                        StoreError::Conflict(AI_CONVERSATION_SOURCE_QUOTA_EXCEEDED.to_owned())
                    })?
        {
            return Err(StoreError::Conflict(
                AI_CONVERSATION_SOURCE_QUOTA_EXCEEDED.to_owned(),
            ));
        }
        let conversation_id = s.conversation_id.ok_or_else(|| {
            StoreError::Validation("AI source must be bound to a conversation".to_owned())
        })?;
        let conversation_project_id =
            writable_ai_conversation_scope(&mut t, conversation_id, s.lab_id, s.user_id).await?;
        if conversation_project_id != s.project_id {
            return Err(StoreError::Validation(
                "AI source conversation scope does not match".to_owned(),
            ));
        }
        if let Some(project_id) = s.project_id
            && lab(&mut t, "projects", project_id).await? != s.lab_id
        {
            return Err(StoreError::Validation(
                "AI source project crosses labs".to_owned(),
            ));
        }
        ia(&mut t, at).await?;
        sqlx::query(
            "INSERT INTO ai_conversation_sources(id,lab_id,user_id,conversation_id,project_id,attachment_id,kind,status,last_activity_at,expires_at,archived_at,error_code,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(s.id)
        .bind(s.lab_id)
        .bind(s.user_id)
        .bind(s.conversation_id)
        .bind(s.project_id)
        .bind(s.attachment_id)
        .bind(encode(&s.kind)?)
        .bind(encode(&s.status)?)
        .bind(s.last_activity_at)
        .bind(s.expires_at)
        .bind(s.archived_at)
        .bind(&s.error_code)
        .bind(s.meta.created_at)
        .bind(s.meta.updated_at)
        .bind(s.meta.deleted_at)
        .bind(s.meta.revision)
        .execute(&mut *t)
        .await
        .map_err(map_sqlx)?;
        for (entity_type, entity_id, project_id, after) in [
            (
                EntityType::Attachment,
                at.id,
                at.project_id,
                ai_source_attachment_audit_snapshot(at)
                    .map_err(|error| StoreError::Serialization(error.to_string()))?,
            ),
            (
                EntityType::AiConversationSource,
                s.id,
                None,
                ai_conversation_source_audit_snapshot(s)
                    .map_err(|error| StoreError::Serialization(error.to_string()))?,
            ),
        ] {
            write_audit(
                &mut t,
                s.lab_id,
                project_id,
                entity_type,
                entity_id,
                AuditAction::Create,
                a,
                None,
                Some(after),
            )
            .await?;
            insert_provenance(
                &mut t,
                &Provenance::from_audit(
                    s.lab_id,
                    project_id,
                    entity_type,
                    entity_id,
                    a,
                    s.meta.created_at,
                ),
            )
            .await?;
        }
        t.commit().await.map_err(map_sqlx)
    }
    async fn get_ai_conversation_source(&self, id: Uuid) -> StoreResult<AiConversationSource> {
        let row = sqlx::query(&format!(
            "SELECT {SC} FROM ai_conversation_sources WHERE id=$1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_conversation_source",
            id,
        })?;
        sr(&row)
    }
    async fn list_ai_conversation_sources(
        &self,
        f: &AiConversationSourceFilter,
    ) -> StoreResult<Vec<AiConversationSource>> {
        let mut query = QueryBuilder::<Postgres>::new(format!(
            "SELECT {SC} FROM ai_conversation_sources WHERE lab_id="
        ));
        query
            .push_bind(f.lab_id)
            .push(" AND user_id=")
            .push_bind(f.user_id)
            .push(" AND deleted_at IS NULL");
        if let Some(value) = f.conversation_id {
            query.push(" AND conversation_id=").push_bind(value);
        }
        if let Some(value) = f.project_id {
            query.push(" AND project_id=").push_bind(value);
        }
        if let Some(value) = f.status {
            query.push(" AND status=").push_bind(encode(&value)?);
        }
        if f.unconsumed_only {
            query.push(
                " AND NOT EXISTS (
                    SELECT 1
                    FROM ai_conversation_messages message,
                         jsonb_array_elements(message.source_refs_json) source_ref
                    WHERE message.deleted_at IS NULL
                      AND message.conversation_id=ai_conversation_sources.conversation_id
                      AND source_ref->>'sourceId'=ai_conversation_sources.id::text
                )",
            );
        }
        query.push(" ORDER BY created_at DESC,id");
        query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
            .iter()
            .map(sr)
            .collect()
    }
    async fn list_expired_ai_conversation_sources(
        &self,
        lab_id: Uuid,
        now: DateTime<Utc>,
        limit: i64,
    ) -> StoreResult<Vec<ExpiredAiConversationSource>> {
        if !(1..=MAX_AI_CONVERSATION_SOURCE_CLEANUP_BATCH).contains(&limit) {
            return Err(StoreError::Validation(
                "AI source cleanup limit must be between 1 and 100".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let rows = sqlx::query(&format!(
            "SELECT {SC} FROM ai_conversation_sources
             WHERE lab_id=$1 AND deleted_at IS NULL AND expires_at<=$2
               AND status IN ('staged','ready','failed')
             ORDER BY expires_at,id LIMIT $3"
        ))
        .bind(lab_id)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let mut expired = Vec::with_capacity(rows.len());
        for row in rows {
            let source = sr(&row)?;
            let attachment_row = sqlx::query(&format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM attachments
                 WHERE id=$1 AND deleted_at IS NULL"
            ))
            .bind(source.attachment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "attachment",
                id: source.attachment_id,
            })?;
            let attachment = attachment_from_row(&attachment_row)?;
            if attachment.lab_id != source.lab_id
                || attachment.project_id.is_some()
                || attachment.entity_type != "ai_conversation_source"
                || attachment.entity_id != source.id
            {
                return Err(StoreError::Validation(
                    "AI source cleanup attachment relationship is invalid".to_owned(),
                ));
            }
            expired.push(ExpiredAiConversationSource { source, attachment });
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(expired)
    }
    async fn list_pending_ai_conversation_source_object_deletions(
        &self,
        lab_id: Uuid,
        limit: i64,
    ) -> StoreResult<Vec<PendingAiConversationSourceObjectDeletion>> {
        if !(1..=MAX_AI_CONVERSATION_SOURCE_CLEANUP_BATCH).contains(&limit) {
            return Err(StoreError::Validation(
                "AI source cleanup limit must be between 1 and 100".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let queue_rows = sqlx::query(
            "SELECT source_id,attachment_id,enqueued_at
             FROM ai_conversation_source_object_deletions
             WHERE lab_id=$1 ORDER BY enqueued_at,source_id LIMIT $2",
        )
        .bind(lab_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let mut pending = Vec::with_capacity(queue_rows.len());
        for queue_row in queue_rows {
            let source_id = queue_row.try_get("source_id").map_err(map_sqlx)?;
            let attachment_id = queue_row.try_get("attachment_id").map_err(map_sqlx)?;
            let enqueued_at = queue_row.try_get("enqueued_at").map_err(map_sqlx)?;
            let source_row = sqlx::query(&format!(
                "SELECT {SC} FROM ai_conversation_sources WHERE id=$1"
            ))
            .bind(source_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "ai_conversation_source",
                id: source_id,
            })?;
            let source = sr(&source_row)?;
            let attachment_row = sqlx::query(&format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id=$1"
            ))
            .bind(attachment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "attachment",
                id: attachment_id,
            })?;
            let attachment = attachment_from_row(&attachment_row)?;
            if source.lab_id != lab_id
                || source.status != AiConversationSourceStatus::Expired
                || source.meta.deleted_at.is_none()
                || source.archived_at.is_some()
                || source.attachment_id != attachment_id
                || attachment.lab_id != lab_id
                || attachment.project_id.is_some()
                || attachment.meta.deleted_at.is_none()
                || attachment.entity_type != "ai_conversation_source"
                || attachment.entity_id != source_id
            {
                return Err(StoreError::Validation(
                    "AI source object cleanup queue relationship is invalid".to_owned(),
                ));
            }
            pending.push(PendingAiConversationSourceObjectDeletion {
                source,
                attachment,
                enqueued_at,
            });
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(pending)
    }
    async fn complete_ai_conversation_source_object_deletion(
        &self,
        source_id: Uuid,
        attachment_id: Uuid,
        cleaned_at: DateTime<Utc>,
        a: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let queued: Option<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT source_id,attachment_id
             FROM ai_conversation_source_object_deletions
             WHERE source_id=$1 FOR UPDATE",
        )
        .bind(source_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let Some((queued_source, queued_attachment)) = queued else {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(());
        };
        if queued_source != source_id || queued_attachment != attachment_id {
            return Err(StoreError::Conflict(
                "AI source object cleanup queue changed".to_owned(),
            ));
        }
        let source_row = sqlx::query(&format!(
            "SELECT {SC} FROM ai_conversation_sources WHERE id=$1 FOR UPDATE"
        ))
        .bind(source_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let source = sr(&source_row)?;
        if source.status != AiConversationSourceStatus::Expired
            || source.meta.deleted_at.is_none()
            || source.archived_at.is_some()
            || source.attachment_id != attachment_id
        {
            return Err(StoreError::Validation(
                "AI source object cleanup queue relationship is invalid".to_owned(),
            ));
        }
        let deleted = sqlx::query(
            "DELETE FROM ai_conversation_source_object_deletions
             WHERE source_id=$1 AND attachment_id=$2",
        )
        .bind(source_id)
        .bind(attachment_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if deleted.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI source object cleanup queue changed".to_owned(),
            ));
        }
        write_audit(
            &mut tx,
            source.lab_id,
            None,
            EntityType::AiConversationSource,
            source_id,
            AuditAction::Cleanup,
            a,
            Some(safe_source_snapshot(&source)?),
            Some(serde_json::json!({
                "object_removed": true,
            })),
        )
        .await?;
        insert_provenance(
            &mut tx,
            &Provenance::from_audit(
                source.lab_id,
                None,
                EntityType::AiConversationSource,
                source_id,
                a,
                cleaned_at,
            ),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }
    async fn archive_ai_conversation_source(
        &self,
        id: Uuid,
        project_id: Uuid,
        expected_revision: i64,
        archived_at: DateTime<Utc>,
        a: &AuditContext,
    ) -> StoreResult<AiConversationSource> {
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {SC} FROM ai_conversation_sources WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_conversation_source",
            id,
        })?;
        let mut source = sr(&row)?;
        if source.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "AI conversation source revision changed".to_owned(),
            ));
        }
        if !matches!(
            source.status,
            AiConversationSourceStatus::Staged | AiConversationSourceStatus::Ready
        ) || source
            .project_id
            .is_some_and(|bound_project| bound_project != project_id)
            || source.expires_at <= archived_at
        {
            return Err(StoreError::Conflict(
                "AI conversation source cannot be archived".to_owned(),
            ));
        }
        let conversation_id = source.conversation_id.ok_or_else(|| {
            StoreError::Validation("AI source must be bound to a conversation".to_owned())
        })?;
        let conversation_project_id =
            writable_ai_conversation_scope(&mut t, conversation_id, source.lab_id, source.user_id)
                .await?;
        if conversation_project_id != source.project_id
            || conversation_project_id != Some(project_id)
        {
            return Err(StoreError::Validation(
                "AI source conversation scope does not match its archive project".to_owned(),
            ));
        }
        if lab(&mut t, "projects", project_id).await? != source.lab_id {
            return Err(StoreError::Validation(
                "AI source archive project crosses labs".to_owned(),
            ));
        }
        let attachment_row = sqlx::query(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(source.attachment_id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "attachment",
            id: source.attachment_id,
        })?;
        let mut attachment = attachment_from_row(&attachment_row)?;
        if attachment.lab_id != source.lab_id
            || attachment.entity_type != "ai_conversation_source"
            || attachment.entity_id != source.id
        {
            return Err(StoreError::Validation(
                "AI source attachment relationship is invalid".to_owned(),
            ));
        }
        let before = safe_source_snapshot(&source)?;
        let attachment_before = safe_source_attachment_snapshot(&attachment)?;
        source.project_id = Some(project_id);
        source.status = AiConversationSourceStatus::Archived;
        source.archived_at = Some(archived_at);
        source.last_activity_at = archived_at;
        source.meta.touch(archived_at);
        let attachment_expected_revision = attachment.meta.revision;
        attachment.project_id = Some(project_id);
        attachment.meta.touch(archived_at);
        let updated = sqlx::query("UPDATE ai_conversation_sources SET project_id=$1,status=$2,last_activity_at=$3,archived_at=$4,updated_at=$5,revision=$6 WHERE id=$7 AND revision=$8")
            .bind(project_id)
            .bind(encode(&source.status)?)
            .bind(source.last_activity_at)
            .bind(source.archived_at)
            .bind(source.meta.updated_at)
            .bind(source.meta.revision)
            .bind(id)
            .bind(expected_revision)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI conversation source revision changed".to_owned(),
            ));
        }
        let attachment_updated = sqlx::query(
            "UPDATE attachments SET project_id=$1,updated_at=$2,revision=$3 WHERE id=$4 AND revision=$5 AND deleted_at IS NULL",
        )
        .bind(project_id)
        .bind(attachment.meta.updated_at)
        .bind(attachment.meta.revision)
        .bind(source.attachment_id)
        .bind(attachment_expected_revision)
        .execute(&mut *t)
        .await
        .map_err(map_sqlx)?;
        if attachment_updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI source attachment revision changed".to_owned(),
            ));
        }
        write_audit(
            &mut t,
            source.lab_id,
            Some(project_id),
            EntityType::AiConversationSource,
            id,
            AuditAction::Archive,
            a,
            Some(before),
            Some(safe_source_snapshot(&source)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance::from_audit(
                source.lab_id,
                Some(project_id),
                EntityType::AiConversationSource,
                id,
                a,
                archived_at,
            ),
        )
        .await?;
        write_audit(
            &mut t,
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            AuditAction::Update,
            a,
            Some(attachment_before),
            Some(safe_source_attachment_snapshot(&attachment)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance::from_audit(
                attachment.lab_id,
                attachment.project_id,
                EntityType::Attachment,
                attachment.id,
                a,
                archived_at,
            ),
        )
        .await?;
        t.commit().await.map_err(map_sqlx)?;
        Ok(source)
    }
    async fn discard_ai_conversation_source(
        &self,
        id: Uuid,
        expected_revision: i64,
        discarded_at: DateTime<Utc>,
        a: &AuditContext,
    ) -> StoreResult<AiConversationSource> {
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {SC} FROM ai_conversation_sources WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_conversation_source",
            id,
        })?;
        let mut source = sr(&row)?;
        if source.meta.revision != expected_revision {
            return Err(StoreError::Conflict(
                "AI conversation source revision changed".to_owned(),
            ));
        }
        if source.status == AiConversationSourceStatus::Archived {
            return Err(StoreError::Conflict(
                "archived AI source cannot be discarded".to_owned(),
            ));
        }
        let attachment_row = sqlx::query(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(source.attachment_id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "attachment",
            id: source.attachment_id,
        })?;
        let mut attachment = attachment_from_row(&attachment_row)?;
        if attachment.lab_id != source.lab_id
            || attachment.entity_type != "ai_conversation_source"
            || attachment.entity_id != source.id
        {
            return Err(StoreError::Validation(
                "AI source attachment relationship is invalid".to_owned(),
            ));
        }
        let before = safe_source_snapshot(&source)?;
        let attachment_before = safe_source_attachment_snapshot(&attachment)?;
        source.status = AiConversationSourceStatus::Expired;
        source.last_activity_at = discarded_at;
        source.meta.soft_delete(discarded_at);
        let attachment_expected_revision = attachment.meta.revision;
        attachment.meta.soft_delete(discarded_at);
        let updated = sqlx::query("UPDATE ai_conversation_sources SET status=$1,last_activity_at=$2,updated_at=$3,deleted_at=$4,revision=$5 WHERE id=$6 AND revision=$7")
            .bind(encode(&source.status)?)
            .bind(source.last_activity_at)
            .bind(source.meta.updated_at)
            .bind(source.meta.deleted_at)
            .bind(source.meta.revision)
            .bind(id)
            .bind(expected_revision)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI conversation source revision changed".to_owned(),
            ));
        }
        let attachment_updated = sqlx::query("UPDATE attachments SET updated_at=$1,deleted_at=$2,revision=$3 WHERE id=$4 AND revision=$5 AND deleted_at IS NULL")
            .bind(attachment.meta.updated_at)
            .bind(attachment.meta.deleted_at)
            .bind(attachment.meta.revision)
            .bind(source.attachment_id)
            .bind(attachment_expected_revision)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
        if attachment_updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "AI source attachment revision changed".to_owned(),
            ));
        }
        sqlx::query(
            "INSERT INTO ai_conversation_source_object_deletions
             (source_id,attachment_id,lab_id,enqueued_at) VALUES($1,$2,$3,$4)",
        )
        .bind(source.id)
        .bind(attachment.id)
        .bind(source.lab_id)
        .bind(discarded_at)
        .execute(&mut *t)
        .await
        .map_err(map_sqlx)?;
        write_audit(
            &mut t,
            source.lab_id,
            None,
            EntityType::AiConversationSource,
            id,
            AuditAction::SoftDelete,
            a,
            Some(before),
            Some(safe_source_snapshot(&source)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance::from_audit(
                source.lab_id,
                None,
                EntityType::AiConversationSource,
                id,
                a,
                discarded_at,
            ),
        )
        .await?;
        write_audit(
            &mut t,
            attachment.lab_id,
            attachment.project_id,
            EntityType::Attachment,
            attachment.id,
            AuditAction::SoftDelete,
            a,
            Some(attachment_before),
            Some(safe_source_attachment_snapshot(&attachment)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance::from_audit(
                attachment.lab_id,
                attachment.project_id,
                EntityType::Attachment,
                attachment.id,
                a,
                discarded_at,
            ),
        )
        .await?;
        t.commit().await.map_err(map_sqlx)?;
        Ok(source)
    }
    async fn create_ai_extraction_draft(
        &self,
        d: &AiExtractionDraft,
        a: &AuditContext,
    ) -> StoreResult<()> {
        d.validate().map_err(|e| StoreError::Validation(e.into()))?;
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        validate_versioned_extraction_postgres(&mut t, d).await?;
        let evidence = if d.evidence.is_empty() {
            vec![AiExtractionEvidence {
                display_order: 0,
                private_image_id: d.private_image_id,
                private_attachment_id: d.attachment_id,
                promoted_attachment_id: None,
                original_sha256: d.image_sha256.clone(),
                sanitized_sha256: d.image_sha256.clone(),
                meta: RecordMeta::new(d.meta.created_at),
            }]
        } else {
            d.evidence.clone()
        };
        let mut images = Vec::with_capacity(evidence.len());
        for item in &evidence {
            images.push(validate_extraction_evidence_postgres(&mut t, d, item).await?);
        }
        let cell = d.data_cell.as_ref();
        let trace = d.model_trace.as_ref();
        sqlx::query(
            "INSERT INTO ai_extraction_drafts(
                id,lab_id,user_id,project_id,experiment_id,experiment_event_id,
                private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,
                data_cell_definition_id,data_cell_subject_type,data_cell_subject_id,
                model_profile_id,model_profile_version,model_purpose,
                usage_input_tokens,usage_output_tokens,usage_total_tokens,
                provider_request_id,trace_json,status,items_json,error_code,
                created_at,updated_at,deleted_at,revision
             )VALUES(
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                $16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30
             )",
        )
        .bind(d.id)
        .bind(d.lab_id)
        .bind(d.user_id)
        .bind(d.project_id)
        .bind(d.experiment_id)
        .bind(d.experiment_event_id)
        .bind(d.private_image_id)
        .bind(d.attachment_id)
        .bind(&d.image_sha256)
        .bind(&d.provider)
        .bind(&d.model)
        .bind(d.tool_run_id)
        .bind(cell.map(|value| value.definition_id))
        .bind(cell.map(|value| encode(&value.subject_type)).transpose()?)
        .bind(cell.map(|value| value.subject_id))
        .bind(trace.map(|value| value.profile_id))
        .bind(trace.map(|value| value.profile_version))
        .bind(trace.map(|value| encode(&value.purpose)).transpose()?)
        .bind(
            trace
                .map(|value| postgres_i64(value.input_tokens, "input token usage"))
                .transpose()?,
        )
        .bind(
            trace
                .map(|value| postgres_i64(value.output_tokens, "output token usage"))
                .transpose()?,
        )
        .bind(
            trace
                .map(|value| postgres_i64(value.total_tokens, "total token usage"))
                .transpose()?,
        )
        .bind(trace.and_then(|value| value.provider_request_id.clone()))
        .bind(trace.map(|value| value.trace.clone()))
        .bind(encode(&d.status)?)
        .bind(
            serde_json::to_value(&d.items)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        )
        .bind(&d.error_code)
        .bind(d.meta.created_at)
        .bind(d.meta.updated_at)
        .bind(d.meta.deleted_at)
        .bind(d.meta.revision)
        .execute(&mut *t)
        .await
        .map_err(map_sqlx)?;
        for item in &d.evidence {
            sqlx::query(
                "INSERT INTO ai_extraction_evidence(
                    draft_id,display_order,private_image_id,private_attachment_id,
                    promoted_attachment_id,original_sha256,sanitized_sha256,
                    created_at,updated_at,revision
                 )VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            )
            .bind(d.id)
            .bind(item.display_order)
            .bind(item.private_image_id)
            .bind(item.private_attachment_id)
            .bind(item.promoted_attachment_id)
            .bind(&item.original_sha256)
            .bind(&item.sanitized_sha256)
            .bind(item.meta.created_at)
            .bind(item.meta.updated_at)
            .bind(item.meta.revision)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
        }
        for mut image in images {
            let before = snapshot(&image)?;
            image.status = PrivateImageStatus::PendingApproval;
            image.last_activity_at = d.meta.created_at;
            image.expires_at = d.meta.created_at + chrono::Duration::days(30);
            image.meta.touch(d.meta.created_at);
            sqlx::query(
                "UPDATE ai_private_images
                 SET status='pending_approval',last_activity_at=$1,expires_at=$2,
                     updated_at=$3,revision=$4
                 WHERE id=$5",
            )
            .bind(image.last_activity_at)
            .bind(image.expires_at)
            .bind(image.meta.updated_at)
            .bind(image.meta.revision)
            .bind(image.id)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut t,
                d.lab_id,
                None,
                EntityType::AiPrivateImage,
                image.id,
                AuditAction::Process,
                a,
                Some(before),
                Some(snapshot(&image)?),
            )
            .await?;
            insert_provenance(
                &mut t,
                &Provenance::from_audit(
                    d.lab_id,
                    None,
                    EntityType::AiPrivateImage,
                    image.id,
                    a,
                    d.meta.created_at,
                ),
            )
            .await?;
        }
        write_audit(
            &mut t,
            d.lab_id,
            None,
            EntityType::AiExtractionDraft,
            d.id,
            AuditAction::Process,
            a,
            None,
            Some(snapshot(d)?),
        )
        .await?;
        insert_provenance(
            &mut t,
            &Provenance {
                id: Uuid::new_v4(),
                lab_id: d.lab_id,
                project_id: None,
                entity_type: EntityType::AiExtractionDraft,
                entity_id: d.id,
                source: ProvenanceSource::Ai,
                actor_user_id: Some(d.user_id),
                import_job_id: None,
                import_commit_id: None,
                tool_run_id: d.tool_run_id,
                provider: Some(d.provider.clone()),
                model: Some(d.model.clone()),
                confidence: None,
                request_id: a.request_id.clone(),
                recorded_at: d.meta.created_at,
            },
        )
        .await?;
        t.commit().await.map_err(map_sqlx)
    }
    async fn get_ai_extraction_draft(&self, id: Uuid) -> StoreResult<AiExtractionDraft> {
        let r = sqlx::query(&format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE id=$1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_extraction_draft",
            id,
        })?;
        let mut draft = xr(&r)?;
        draft.evidence = load_evidence_postgres_pool(&self.pool, draft.id).await?;
        Ok(draft)
    }
    async fn list_ai_extraction_drafts(
        &self,
        l: Uuid,
        u: Uuid,
        p: Option<Uuid>,
    ) -> StoreResult<Vec<AiExtractionDraft>> {
        let mut q = QueryBuilder::<Postgres>::new(format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE lab_id="
        ));
        q.push_bind(l)
            .push(" AND user_id=")
            .push_bind(u)
            .push(" AND deleted_at IS NULL");
        if let Some(v) = p {
            q.push(" AND project_id=").push_bind(v);
        }
        q.push(" ORDER BY created_at DESC,id");
        let rows = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let mut drafts = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut draft = xr(row)?;
            draft.evidence = load_evidence_postgres_pool(&self.pool, draft.id).await?;
            drafts.push(draft);
        }
        Ok(drafts)
    }
    async fn apply_ai_extraction_draft(
        &self,
        id: Uuid,
        approval: &AiExtractionApprovalInput,
        a: &AuditContext,
    ) -> StoreResult<AppliedAiExtraction> {
        approval
            .validate()
            .map_err(|error| StoreError::Validation(error.to_owned()))?;
        let actor_user_id = a.actor.user_id.ok_or_else(|| {
            StoreError::Validation("AI extraction approval requires a human actor".to_owned())
        })?;
        if a.actor.actor_type != ActorType::Human
            || matches!(a.source, WriteSource::Ai | WriteSource::Mcp)
        {
            return Err(StoreError::Validation(
                "AI extraction approval requires a human actor".to_owned(),
            ));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let r = sqlx::query(&format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_extraction_draft",
            id,
        })?;
        let mut d = xr(&r)?;
        d.evidence = load_evidence_postgres(&mut t, d.id).await?;
        if actor_user_id != d.user_id {
            return Err(StoreError::Validation(
                "AI extraction approval actor must own the draft".to_owned(),
            ));
        }
        if d.data_cell.is_some() && d.evidence.is_empty() {
            return Err(StoreError::Validation(
                "versioned AI extraction evidence is missing".to_owned(),
            ));
        }
        if d.meta.revision != approval.expected_revision {
            return Err(StoreError::Conflict("extraction revision changed".into()));
        }
        if !matches!(
            d.status,
            AiExtractionStatus::Draft | AiExtractionStatus::PendingApproval
        ) {
            return Err(StoreError::Conflict("extraction already resolved".into()));
        }
        if d.data_cell.is_some() && approval.selections.len() != 1 {
            return Err(StoreError::Validation(
                "a data-cell extraction must approve exactly one candidate".to_owned(),
            ));
        }
        if approval
            .selections
            .iter()
            .any(|selection| selection.item_index >= d.items.len())
        {
            return Err(StoreError::Validation("selected index out of range".into()));
        }
        let b = snapshot(&d)?;
        let now = Utc::now();
        let mut os = Vec::new();
        for n in 0..d.items.len() {
            let selection = approval
                .selections
                .iter()
                .find(|selection| selection.item_index == n);
            d.items[n].selected = selection.is_some();
            if let Some(selection) = selection {
                let mut item = d.items[n].clone();
                item.value.value = selection.value.clone();
                item.value.notes = selection.notes.clone();
                item.value.recorded_by = Some(actor_user_id);
                item.value.recorded_at = now;
                item.value.meta = RecordMeta::new(now);
                item.observation.current_value_version = 1;
                item.observation.meta = RecordMeta::new(now);
                item.validate()
                    .map_err(|error| StoreError::Validation(error.to_owned()))?;
                d.items[n] = item.clone();
                io(&mut t, &d, &item, a).await?;
                os.push(item.observation);
            }
        }
        let mut attachments = Vec::with_capacity(d.evidence.len());
        let mut links = Vec::new();
        for index in 0..d.evidence.len() {
            let mut evidence = d.evidence[index].clone();
            let (attachment, mut evidence_links) = promote_extraction_evidence_postgres(
                &mut t,
                &d,
                &mut evidence,
                &os,
                actor_user_id,
                now,
                a,
            )
            .await?;
            d.evidence[index] = evidence;
            attachments.push(attachment);
            links.append(&mut evidence_links);
        }
        if d.evidence.is_empty() {
            let image_row = sqlx::query(&format!(
                "SELECT {IC} FROM ai_private_images
                 WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
            ))
            .bind(d.private_image_id)
            .fetch_optional(&mut *t)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "ai_private_image",
                id: d.private_image_id,
            })?;
            let mut image = ir(&image_row)?;
            let before = snapshot(&image)?;
            image.status = PrivateImageStatus::Active;
            image.last_activity_at = now;
            image.expires_at = now + chrono::Duration::days(30);
            image.meta.touch(now);
            sqlx::query(
                "UPDATE ai_private_images
                 SET status='active',last_activity_at=$1,expires_at=$2,updated_at=$3,revision=$4
                 WHERE id=$5",
            )
            .bind(image.last_activity_at)
            .bind(image.expires_at)
            .bind(image.meta.updated_at)
            .bind(image.meta.revision)
            .bind(image.id)
            .execute(&mut *t)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut t,
                d.lab_id,
                Some(d.project_id),
                EntityType::AiPrivateImage,
                image.id,
                AuditAction::Process,
                a,
                Some(before),
                Some(snapshot(&image)?),
            )
            .await?;
            insert_provenance(
                &mut t,
                &Provenance::from_audit(
                    d.lab_id,
                    Some(d.project_id),
                    EntityType::AiPrivateImage,
                    image.id,
                    a,
                    now,
                ),
            )
            .await?;
        }
        d.status = AiExtractionStatus::Approved;
        d.meta.touch(now);
        sqlx::query("UPDATE ai_extraction_drafts SET status=$1,items_json=$2,updated_at=$3,revision=$4 WHERE id=$5").bind(encode(&d.status)?).bind(serde_json::to_value(&d.items).map_err(|e|StoreError::Serialization(e.to_string()))?).bind(d.meta.updated_at).bind(d.meta.revision).bind(id).execute(&mut*t).await.map_err(map_sqlx)?;
        write_audit(
            &mut t,
            d.lab_id,
            Some(d.project_id),
            EntityType::AiExtractionDraft,
            id,
            AuditAction::Approve,
            a,
            Some(b),
            Some(snapshot(&d)?),
        )
        .await?;
        let mut approval_provenance = Provenance::from_audit(
            d.lab_id,
            Some(d.project_id),
            EntityType::AiExtractionDraft,
            d.id,
            a,
            now,
        );
        approval_provenance.tool_run_id = d.tool_run_id;
        approval_provenance.provider = Some(d.provider.clone());
        approval_provenance.model = Some(d.model.clone());
        insert_provenance(&mut t, &approval_provenance).await?;
        t.commit().await.map_err(map_sqlx)?;
        Ok(AppliedAiExtraction {
            draft: d,
            observations: os,
            attachments,
            links,
        })
    }
    async fn reject_ai_extraction_draft(
        &self,
        id: Uuid,
        rejection: &AiExtractionRejectionInput,
        a: &AuditContext,
    ) -> StoreResult<AiExtractionDraft> {
        rejection
            .validate()
            .map_err(|error| StoreError::Validation(error.to_owned()))?;
        let actor_user_id = a.actor.user_id.ok_or_else(|| {
            StoreError::Validation("AI extraction rejection requires a human actor".to_owned())
        })?;
        if a.actor.actor_type != ActorType::Human
            || matches!(a.source, WriteSource::Ai | WriteSource::Mcp)
        {
            return Err(StoreError::Validation(
                "AI extraction rejection requires a human actor".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let row = sqlx::query(&format!(
            "SELECT {XC} FROM ai_extraction_drafts
             WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or(StoreError::NotFound {
            entity: "ai_extraction_draft",
            id,
        })?;
        let mut draft = xr(&row)?;
        draft.evidence = load_evidence_postgres(&mut tx, draft.id).await?;
        if actor_user_id != draft.user_id {
            return Err(StoreError::Validation(
                "AI extraction rejection actor must own the draft".to_owned(),
            ));
        }
        if draft.meta.revision != rejection.expected_revision {
            return Err(StoreError::Conflict(
                "extraction revision changed".to_owned(),
            ));
        }
        if draft.status != AiExtractionStatus::PendingApproval {
            return Err(StoreError::Conflict(
                "only pending AI extraction drafts can be rejected".to_owned(),
            ));
        }
        if draft.data_cell.is_some() && draft.evidence.is_empty() {
            return Err(StoreError::Validation(
                "versioned AI extraction evidence is missing".to_owned(),
            ));
        }
        if draft
            .evidence
            .iter()
            .any(|evidence| evidence.promoted_attachment_id.is_some())
        {
            return Err(StoreError::Conflict(
                "promoted AI extraction evidence cannot be rejected".to_owned(),
            ));
        }
        let bindings = if draft.evidence.is_empty() {
            vec![(
                draft.private_image_id,
                draft.attachment_id,
                draft.image_sha256.clone(),
            )]
        } else {
            draft
                .evidence
                .iter()
                .map(|evidence| {
                    (
                        evidence.private_image_id,
                        evidence.private_attachment_id,
                        evidence.original_sha256.clone(),
                    )
                })
                .collect()
        };
        let mut images = Vec::with_capacity(bindings.len());
        for (image_id, attachment_id, original_sha256) in bindings {
            let image_row = sqlx::query(&format!(
                "SELECT {IC} FROM ai_private_images
                 WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
            ))
            .bind(image_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "ai_private_image",
                id: image_id,
            })?;
            let image = ir(&image_row)?;
            let attachment_row = sqlx::query(
                "SELECT project_id,entity_type,entity_id,sha256
                 FROM attachments WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
            )
            .bind(attachment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(StoreError::NotFound {
                entity: "attachment",
                id: attachment_id,
            })?;
            let attachment_project_id: Option<Uuid> =
                attachment_row.try_get("project_id").map_err(map_sqlx)?;
            let attachment_entity_type: String =
                attachment_row.try_get("entity_type").map_err(map_sqlx)?;
            let attachment_entity_id: Uuid =
                attachment_row.try_get("entity_id").map_err(map_sqlx)?;
            let attachment_sha256: String = attachment_row.try_get("sha256").map_err(map_sqlx)?;
            if (image.lab_id, image.user_id, image.attachment_id)
                != (draft.lab_id, draft.user_id, attachment_id)
                || image.project_id.is_some()
                || image.status != PrivateImageStatus::PendingApproval
                || attachment_project_id.is_some()
                || attachment_entity_type != "ai_private_image"
                || attachment_entity_id != image.id
                || attachment_sha256 != original_sha256
            {
                return Err(StoreError::Conflict(
                    "AI extraction evidence changed before rejection".to_owned(),
                ));
            }
            images.push(image);
        }

        let now = Utc::now();
        for mut image in images {
            let before = snapshot(&image)?;
            image.status = PrivateImageStatus::Active;
            image.last_activity_at = now;
            image.expires_at = now + chrono::Duration::days(30);
            image.archived_at = None;
            image.meta.touch(now);
            sqlx::query(
                "UPDATE ai_private_images
                 SET status='active',last_activity_at=$1,expires_at=$2,archived_at=NULL,
                     updated_at=$3,revision=$4
                 WHERE id=$5",
            )
            .bind(image.last_activity_at)
            .bind(image.expires_at)
            .bind(image.meta.updated_at)
            .bind(image.meta.revision)
            .bind(image.id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            write_audit(
                &mut tx,
                draft.lab_id,
                None,
                EntityType::AiPrivateImage,
                image.id,
                AuditAction::Process,
                a,
                Some(before),
                Some(snapshot(&image)?),
            )
            .await?;
            insert_provenance(
                &mut tx,
                &Provenance::from_audit(
                    draft.lab_id,
                    None,
                    EntityType::AiPrivateImage,
                    image.id,
                    a,
                    now,
                ),
            )
            .await?;
        }

        let before_draft = snapshot(&draft)?;
        draft.status = AiExtractionStatus::Rejected;
        draft.meta.touch(now);
        sqlx::query(
            "UPDATE ai_extraction_drafts SET status='rejected',updated_at=$1,revision=$2
             WHERE id=$3",
        )
        .bind(draft.meta.updated_at)
        .bind(draft.meta.revision)
        .bind(draft.id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            draft.lab_id,
            None,
            EntityType::AiExtractionDraft,
            draft.id,
            AuditAction::Revoke,
            a,
            Some(before_draft),
            Some(snapshot(&draft)?),
        )
        .await?;
        insert_provenance(
            &mut tx,
            &Provenance::from_audit(
                draft.lab_id,
                None,
                EntityType::AiExtractionDraft,
                draft.id,
                a,
                now,
            ),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(draft)
    }
    async fn list_project_attachments(&self, l: Uuid, p: Uuid) -> StoreResult<Vec<Attachment>> {
        let rows=sqlx::query(&format!("SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE lab_id=$1 AND project_id=$2 AND deleted_at IS NULL ORDER BY created_at DESC,id")).bind(l).bind(p).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(attachment_from_row).collect()
    }
    async fn record_workspace_operation(
        &self,
        operation: muriarc_core::WorkspaceOperationInput,
        audit: &AuditContext,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        write_audit(
            &mut tx,
            operation.lab_id,
            operation.project_id,
            operation.entity_type,
            operation.entity_id,
            operation.action,
            audit,
            operation.before,
            operation.after,
        )
        .await?;
        tx.commit().await.map_err(map_sqlx)
    }
}
