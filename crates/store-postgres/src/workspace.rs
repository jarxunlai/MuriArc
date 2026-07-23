use super::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_core::*;
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};
use std::collections::BTreeSet;
use uuid::Uuid;
const LC: &str = "id,lab_id,project_id,attachment_id,target_type,target_id,created_by,created_at,updated_at,deleted_at,revision";
const DC: &str = "id,lab_id,project_id,attachment_id,kind,media_type,relative_path,size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision";
const IC: &str = "id,lab_id,user_id,conversation_id,attachment_id,project_id,status,last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision";
const SC: &str = "id,lab_id,user_id,conversation_id,project_id,attachment_id,kind,status,last_activity_at,expires_at,archived_at,error_code,created_at,updated_at,deleted_at,revision";
const XC: &str = "id,lab_id,user_id,project_id,experiment_id,experiment_event_id,private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,status,items_json,error_code,created_at,updated_at,deleted_at,revision";
fn safe_source_snapshot(source: &AiConversationSource) -> StoreResult<serde_json::Value> {
    ai_conversation_source_audit_snapshot(source)
        .map_err(|error| StoreError::Serialization(error.to_string()))
}
fn safe_source_attachment_snapshot(attachment: &Attachment) -> StoreResult<serde_json::Value> {
    ai_source_attachment_audit_snapshot(attachment)
        .map_err(|error| StoreError::Serialization(error.to_string()))
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
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        items: serde_json::from_value(r.try_get("items_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: rm(r)?,
    })
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
            let x: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT lab_id,user_id FROM ai_conversations WHERE id=$1 AND deleted_at IS NULL",
            )
            .bind(c)
            .fetch_optional(&mut *t)
            .await
            .map_err(map_sqlx)?;
            if x != Some((i.lab_id, i.user_id)) {
                return Err(StoreError::Validation("conversation is not owned".into()));
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
        sqlx::query(
            "UPDATE attachments SET project_id=$1,updated_at=$2,revision=revision+1 WHERE id=$3",
        )
        .bind(p)
        .bind(now)
        .bind(i.attachment_id)
        .execute(&mut *t)
        .await
        .map_err(map_sqlx)?;
        write_audit(
            &mut t,
            i.lab_id,
            Some(p),
            EntityType::AiPrivateImage,
            id,
            AuditAction::Archive,
            a,
            Some(b),
            Some(snapshot(&i)?),
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
        let scope: Option<(Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
            "SELECT lab_id,user_id,project_id FROM ai_conversations WHERE id=$1 AND deleted_at IS NULL",
        )
        .bind(conversation_id)
        .fetch_optional(&mut *t)
        .await
        .map_err(map_sqlx)?;
        if scope != Some((s.lab_id, s.user_id, s.project_id)) {
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
        let x:Option<(Uuid,Uuid,Uuid,String)>=sqlx::query_as("SELECT i.lab_id,i.user_id,i.attachment_id,a.sha256 FROM ai_private_images i JOIN attachments a ON a.id=i.attachment_id WHERE i.id=$1 AND i.deleted_at IS NULL").bind(d.private_image_id).fetch_optional(&mut*t).await.map_err(map_sqlx)?;
        if x != Some((d.lab_id, d.user_id, d.attachment_id, d.image_sha256.clone())) {
            return Err(StoreError::Validation(
                "extraction image owner or SHA mismatch".into(),
            ));
        }
        sqlx::query("INSERT INTO ai_extraction_drafts(id,lab_id,user_id,project_id,experiment_id,experiment_event_id,private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,status,items_json,error_code,created_at,updated_at,deleted_at,revision)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)").bind(d.id).bind(d.lab_id).bind(d.user_id).bind(d.project_id).bind(d.experiment_id).bind(d.experiment_event_id).bind(d.private_image_id).bind(d.attachment_id).bind(&d.image_sha256).bind(&d.provider).bind(&d.model).bind(d.tool_run_id).bind(encode(&d.status)?).bind(serde_json::to_value(&d.items).map_err(|e|StoreError::Serialization(e.to_string()))?).bind(&d.error_code).bind(d.meta.created_at).bind(d.meta.updated_at).bind(d.meta.deleted_at).bind(d.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
        sqlx::query("UPDATE ai_private_images SET status='pending_approval',last_activity_at=$1,expires_at=$1+interval '30 days',updated_at=$1,revision=revision+1 WHERE id=$2").bind(d.meta.created_at).bind(d.private_image_id).execute(&mut*t).await.map_err(map_sqlx)?;
        write_audit(
            &mut t,
            d.lab_id,
            Some(d.project_id),
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
                project_id: Some(d.project_id),
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
        xr(&r)
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
        let r = q.build().fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter().map(xr).collect()
    }
    async fn apply_ai_extraction_draft(
        &self,
        id: Uuid,
        rev: i64,
        xs: &[usize],
        a: &AuditContext,
    ) -> StoreResult<AppliedAiExtraction> {
        let s: BTreeSet<usize> = xs.iter().copied().collect();
        if s.is_empty() || s.len() != xs.len() {
            return Err(StoreError::Validation(
                "selected indexes empty or duplicated".into(),
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
        if d.meta.revision != rev {
            return Err(StoreError::Conflict("extraction revision changed".into()));
        }
        if !matches!(
            d.status,
            AiExtractionStatus::Draft | AiExtractionStatus::PendingApproval
        ) {
            return Err(StoreError::Conflict("extraction already resolved".into()));
        }
        if s.iter().any(|v| *v >= d.items.len()) {
            return Err(StoreError::Validation("selected index out of range".into()));
        }
        let b = snapshot(&d)?;
        let mut os = Vec::new();
        for n in 0..d.items.len() {
            d.items[n].selected = s.contains(&n);
            if d.items[n].selected {
                let it = d.items[n].clone();
                io(&mut t, &d, &it, a).await?;
                os.push(it.observation)
            }
        }
        d.status = AiExtractionStatus::Approved;
        d.meta.touch(Utc::now());
        sqlx::query("UPDATE ai_extraction_drafts SET status=$1,items_json=$2,updated_at=$3,revision=$4 WHERE id=$5").bind(encode(&d.status)?).bind(serde_json::to_value(&d.items).map_err(|e|StoreError::Serialization(e.to_string()))?).bind(d.meta.updated_at).bind(d.meta.revision).bind(id).execute(&mut*t).await.map_err(map_sqlx)?;
        sqlx::query("UPDATE ai_private_images SET status='active',last_activity_at=$1,expires_at=$1+interval '30 days',updated_at=$1,revision=revision+1 WHERE id=$2").bind(d.meta.updated_at).bind(d.private_image_id).execute(&mut*t).await.map_err(map_sqlx)?;
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
        t.commit().await.map_err(map_sqlx)?;
        Ok(AppliedAiExtraction {
            draft: d,
            observations: os,
        })
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
