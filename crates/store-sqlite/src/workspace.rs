use super::*;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use muriarc_core::*;
use sqlx::{QueryBuilder, Row, Sqlite, sqlite::SqliteRow};
use std::collections::BTreeSet;
use uuid::Uuid;
const LC: &str = "id,lab_id,project_id,attachment_id,target_type,target_id,created_by,created_at,updated_at,deleted_at,revision";
const DC: &str = "id,lab_id,project_id,attachment_id,kind,media_type,relative_path,size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision";
const IC: &str = "id,lab_id,user_id,conversation_id,attachment_id,project_id,status,last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision";
const XC: &str = "id,lab_id,user_id,project_id,experiment_id,experiment_event_id,private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,status,items_json,error_code,created_at,updated_at,deleted_at,revision";
fn lr(r: &SqliteRow) -> StoreResult<AttachmentLink> {
    Ok(AttachmentLink {
        id: uuid(r.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(r.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: uuid(r.try_get("project_id").map_err(map_sqlx)?)?,
        attachment_id: uuid(r.try_get("attachment_id").map_err(map_sqlx)?)?,
        target_type: decode(r.try_get("target_type").map_err(map_sqlx)?)?,
        target_id: uuid(r.try_get("target_id").map_err(map_sqlx)?)?,
        created_by: uuid(r.try_get("created_by").map_err(map_sqlx)?)?,
        meta: meta(r)?,
    })
}
fn dr(r: &SqliteRow) -> StoreResult<AttachmentDerivative> {
    Ok(AttachmentDerivative {
        id: uuid(r.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(r.try_get("lab_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(r.try_get("project_id").map_err(map_sqlx)?)?,
        attachment_id: uuid(r.try_get("attachment_id").map_err(map_sqlx)?)?,
        kind: decode(r.try_get("kind").map_err(map_sqlx)?)?,
        media_type: r.try_get("media_type").map_err(map_sqlx)?,
        relative_path: r.try_get("relative_path").map_err(map_sqlx)?,
        size_bytes: r.try_get("size_bytes").map_err(map_sqlx)?,
        sha256: r.try_get("sha256").map_err(map_sqlx)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: meta(r)?,
    })
}
fn ir(r: &SqliteRow) -> StoreResult<PrivateAiImage> {
    Ok(PrivateAiImage {
        id: uuid(r.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(r.try_get("lab_id").map_err(map_sqlx)?)?,
        user_id: uuid(r.try_get("user_id").map_err(map_sqlx)?)?,
        conversation_id: optional_uuid(r.try_get("conversation_id").map_err(map_sqlx)?)?,
        attachment_id: uuid(r.try_get("attachment_id").map_err(map_sqlx)?)?,
        project_id: optional_uuid(r.try_get("project_id").map_err(map_sqlx)?)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        last_activity_at: r.try_get("last_activity_at").map_err(map_sqlx)?,
        expires_at: r.try_get("expires_at").map_err(map_sqlx)?,
        archived_at: r.try_get("archived_at").map_err(map_sqlx)?,
        meta: meta(r)?,
    })
}
fn xr(r: &SqliteRow) -> StoreResult<AiExtractionDraft> {
    Ok(AiExtractionDraft {
        id: uuid(r.try_get("id").map_err(map_sqlx)?)?,
        lab_id: uuid(r.try_get("lab_id").map_err(map_sqlx)?)?,
        user_id: uuid(r.try_get("user_id").map_err(map_sqlx)?)?,
        project_id: uuid(r.try_get("project_id").map_err(map_sqlx)?)?,
        experiment_id: uuid(r.try_get("experiment_id").map_err(map_sqlx)?)?,
        experiment_event_id: uuid(r.try_get("experiment_event_id").map_err(map_sqlx)?)?,
        private_image_id: uuid(r.try_get("private_image_id").map_err(map_sqlx)?)?,
        attachment_id: uuid(r.try_get("attachment_id").map_err(map_sqlx)?)?,
        image_sha256: r.try_get("image_sha256").map_err(map_sqlx)?,
        provider: r.try_get("provider").map_err(map_sqlx)?,
        model: r.try_get("model").map_err(map_sqlx)?,
        tool_run_id: optional_uuid(r.try_get("tool_run_id").map_err(map_sqlx)?)?,
        status: decode(r.try_get("status").map_err(map_sqlx)?)?,
        items: serde_json::from_str(r.try_get("items_json").map_err(map_sqlx)?)
            .map_err(|e| StoreError::Serialization(e.to_string()))?,
        error_code: r.try_get("error_code").map_err(map_sqlx)?,
        meta: meta(r)?,
    })
}
async fn lab(tx: &mut sqlx::Transaction<'_, Sqlite>, t: &str, id: Uuid) -> StoreResult<Uuid> {
    let v: Option<String> = sqlx::query_scalar(&format!(
        "SELECT lab_id FROM {t} WHERE id=? AND deleted_at IS NULL"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    uuid(&v.ok_or(StoreError::NotFound {
        entity: "workspace_scope",
        id,
    })?)
}
async fn ia(tx: &mut sqlx::Transaction<'_, Sqlite>, a: &Attachment) -> StoreResult<()> {
    sqlx::query("INSERT INTO attachments(id,lab_id,project_id,entity_type,entity_id,file_name,media_type,relative_path,size_bytes,sha256,version,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(a.id.to_string()).bind(a.lab_id.to_string()).bind(a.project_id.map(|v|v.to_string())).bind(&a.entity_type).bind(a.entity_id.to_string()).bind(&a.file_name).bind(&a.media_type).bind(&a.relative_path).bind(a.size_bytes).bind(&a.sha256).bind(a.version).bind(a.meta.created_at).bind(a.meta.updated_at).bind(a.meta.deleted_at).bind(a.meta.revision).execute(&mut**tx).await.map_err(map_sqlx)?;
    Ok(())
}
async fn io(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    d: &AiExtractionDraft,
    it: &AiExtractionItem,
    a: &AuditContext,
) -> StoreResult<()> {
    let o = &it.observation;
    let v = &it.value;
    let experiment_status: String =
        sqlx::query_scalar("SELECT status FROM experiments WHERE id=? AND deleted_at IS NULL")
            .bind(o.experiment_id.to_string())
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
    validate_observation_scope_sqlite(tx, o.lab_id, o.project_id, o.experiment_id).await?;
    let n:i64=sqlx::query_scalar("SELECT count(*) FROM observations WHERE experiment_event_id=? AND definition_id=? AND subject_type=? AND subject_id=? AND deleted_at IS NULL").bind(o.experiment_event_id.to_string()).bind(o.definition_id.to_string()).bind(encode(&o.subject_type)?).bind(o.subject_id.to_string()).fetch_one(&mut**tx).await.map_err(map_sqlx)?;
    if n != 0 {
        return Err(StoreError::Conflict(
            "AI extraction cannot overwrite an existing observation cell".into(),
        ));
    }
    let er=sqlx::query("SELECT lab_id,project_id,experiment_id FROM experiment_events WHERE id=? AND deleted_at IS NULL").bind(o.experiment_event_id.to_string()).fetch_optional(&mut**tx).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound{entity:"experiment_event",id:o.experiment_event_id})?;
    if (
        uuid(er.try_get("lab_id").map_err(map_sqlx)?)?,
        uuid(er.try_get("project_id").map_err(map_sqlx)?)?,
        uuid(er.try_get("experiment_id").map_err(map_sqlx)?)?,
    ) != (d.lab_id, d.project_id, d.experiment_id)
    {
        return Err(StoreError::Validation(
            "extraction event scope invalid".into(),
        ));
    }
    let rr=sqlx::query(&format!("SELECT {OBSERVATION_DEFINITION_COLUMNS} FROM observation_definitions WHERE id=? AND deleted_at IS NULL")).bind(o.definition_id.to_string()).fetch_optional(&mut**tx).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound{entity:"observation_definition",id:o.definition_id})?;
    let def = observation_definition_from_row(&rr)?;
    if (def.lab_id, def.project_id, def.experiment_id) != (d.lab_id, d.project_id, d.experiment_id)
    {
        return Err(StoreError::Validation(
            "extraction definition scope invalid".into(),
        ));
    }
    def.validate_value(&v.value)
        .map_err(|e| StoreError::Validation(e.to_string()))?;
    validate_observation_subject_sqlite(tx, o).await?;
    sqlx::query("INSERT INTO observations(id,lab_id,project_id,experiment_id,experiment_event_id,definition_id,subject_type,subject_id,context_json,current_value_version,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(o.id.to_string()).bind(o.lab_id.to_string()).bind(o.project_id.to_string()).bind(o.experiment_id.to_string()).bind(o.experiment_event_id.to_string()).bind(o.definition_id.to_string()).bind(encode(&o.subject_type)?).bind(o.subject_id.to_string()).bind(o.context.to_string()).bind(o.current_value_version).bind(o.meta.created_at).bind(o.meta.updated_at).bind(o.meta.deleted_at).bind(o.meta.revision).execute(&mut**tx).await.map_err(map_sqlx)?;
    insert_observation_value_sqlite(tx, v).await?;
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
        insert_provenance_tx(
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
impl WorkspaceStore for SqliteStore {
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
        sqlx::query("INSERT INTO attachment_links(id,lab_id,project_id,attachment_id,target_type,target_id,created_by,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(l.id.to_string()).bind(l.lab_id.to_string()).bind(l.project_id.to_string()).bind(l.attachment_id.to_string()).bind(encode(&l.target_type)?).bind(l.target_id.to_string()).bind(l.created_by.to_string()).bind(l.meta.created_at).bind(l.meta.updated_at).bind(l.meta.deleted_at).bind(l.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
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
        let r=sqlx::query(&format!("SELECT {LC} FROM attachment_links WHERE attachment_id=? AND deleted_at IS NULL ORDER BY created_at,id")).bind(id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter().map(lr).collect()
    }
    async fn create_attachment_derivative(
        &self,
        d: &AttachmentDerivative,
        a: &AuditContext,
    ) -> StoreResult<()> {
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        if lab(&mut t, "attachments", d.attachment_id).await? != d.lab_id {
            return Err(StoreError::Validation("derivative crosses labs".into()));
        }
        sqlx::query("INSERT INTO attachment_derivatives(id,lab_id,project_id,attachment_id,kind,media_type,relative_path,size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(d.id.to_string()).bind(d.lab_id.to_string()).bind(d.project_id.map(|v|v.to_string())).bind(d.attachment_id.to_string()).bind(encode(&d.kind)?).bind(&d.media_type).bind(&d.relative_path).bind(d.size_bytes).bind(&d.sha256).bind(encode(&d.status)?).bind(&d.error_code).bind(d.meta.created_at).bind(d.meta.updated_at).bind(d.meta.deleted_at).bind(d.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
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
        let r=sqlx::query(&format!("SELECT {DC} FROM attachment_derivatives WHERE attachment_id=? AND deleted_at IS NULL ORDER BY kind,id")).bind(id.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
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
        {
            return Err(StoreError::Validation("invalid private image".into()));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        if lab(&mut t, "users", i.user_id).await? != i.lab_id {
            return Err(StoreError::Validation("private owner crosses labs".into()));
        }
        ia(&mut t, at).await?;
        sqlx::query("INSERT INTO ai_private_images(id,lab_id,user_id,conversation_id,attachment_id,project_id,status,last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(i.id.to_string()).bind(i.lab_id.to_string()).bind(i.user_id.to_string()).bind(i.conversation_id.map(|v|v.to_string())).bind(i.attachment_id.to_string()).bind(i.project_id.map(|v|v.to_string())).bind(encode(&i.status)?).bind(i.last_activity_at).bind(i.expires_at).bind(i.archived_at).bind(i.meta.created_at).bind(i.meta.updated_at).bind(i.meta.deleted_at).bind(i.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
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
            insert_provenance_tx(
                &mut t,
                &Provenance::from_audit(i.lab_id, None, et, id, a, i.meta.created_at),
            )
            .await?
        }
        t.commit().await.map_err(map_sqlx)
    }
    async fn get_private_ai_image(&self, id: Uuid) -> StoreResult<PrivateAiImage> {
        let r = sqlx::query(&format!(
            "SELECT {IC} FROM ai_private_images WHERE id=? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
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
        let mut q = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {IC} FROM ai_private_images WHERE lab_id="
        ));
        q.push_bind(f.lab_id.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(v) = f.user_id {
            q.push(" AND user_id=").push_bind(v.to_string());
        }
        if let Some(v) = f.conversation_id {
            q.push(" AND conversation_id=").push_bind(v.to_string());
        }
        if let Some(v) = f.project_id {
            q.push(" AND project_id=").push_bind(v.to_string());
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
            "SELECT {IC} FROM ai_private_images WHERE id=? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
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
        sqlx::query("UPDATE ai_private_images SET project_id=?,status=?,last_activity_at=?,archived_at=?,updated_at=?,revision=? WHERE id=?").bind(p.to_string()).bind(encode(&i.status)?).bind(i.last_activity_at).bind(i.archived_at).bind(i.meta.updated_at).bind(i.meta.revision).bind(id.to_string()).execute(&mut*t).await.map_err(map_sqlx)?;
        sqlx::query(
            "UPDATE attachments SET project_id=?,updated_at=?,revision=revision+1 WHERE id=?",
        )
        .bind(p.to_string())
        .bind(now)
        .bind(i.attachment_id.to_string())
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
        let r=sqlx::query("SELECT i.user_id,count(*) image_count,coalesce(sum(a.size_bytes),0) total_size_bytes,sum(CASE WHEN i.status NOT IN('archived','expired') AND i.expires_at<=? THEN 1 ELSE 0 END) expiring_count,sum(CASE WHEN i.status='failed' THEN 1 ELSE 0 END) failed_count FROM ai_private_images i JOIN attachments a ON a.id=i.attachment_id WHERE i.lab_id=? AND i.deleted_at IS NULL GROUP BY i.user_id ORDER BY i.user_id").bind(n+Duration::days(7)).bind(l.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
        r.iter()
            .map(|x| {
                Ok(PrivateImageStats {
                    user_id: uuid(x.try_get("user_id").map_err(map_sqlx)?)?,
                    image_count: x.try_get("image_count").map_err(map_sqlx)?,
                    total_size_bytes: x.try_get("total_size_bytes").map_err(map_sqlx)?,
                    expiring_count: x.try_get("expiring_count").map_err(map_sqlx)?,
                    failed_count: x.try_get("failed_count").map_err(map_sqlx)?,
                })
            })
            .collect()
    }
    async fn create_ai_extraction_draft(
        &self,
        d: &AiExtractionDraft,
        a: &AuditContext,
    ) -> StoreResult<()> {
        d.validate().map_err(|e| StoreError::Validation(e.into()))?;
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let r=sqlx::query("SELECT i.lab_id,i.user_id,i.attachment_id,a.sha256 FROM ai_private_images i JOIN attachments a ON a.id=i.attachment_id WHERE i.id=? AND i.deleted_at IS NULL").bind(d.private_image_id.to_string()).fetch_optional(&mut*t).await.map_err(map_sqlx)?.ok_or(StoreError::NotFound{entity:"ai_private_image",id:d.private_image_id})?;
        if (
            uuid(r.try_get("lab_id").map_err(map_sqlx)?)?,
            uuid(r.try_get("user_id").map_err(map_sqlx)?)?,
            uuid(r.try_get("attachment_id").map_err(map_sqlx)?)?,
            r.try_get::<String, _>("sha256").map_err(map_sqlx)?,
        ) != (d.lab_id, d.user_id, d.attachment_id, d.image_sha256.clone())
        {
            return Err(StoreError::Validation("extraction image mismatch".into()));
        }
        sqlx::query("INSERT INTO ai_extraction_drafts(id,lab_id,user_id,project_id,experiment_id,experiment_event_id,private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,status,items_json,error_code,created_at,updated_at,deleted_at,revision)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").bind(d.id.to_string()).bind(d.lab_id.to_string()).bind(d.user_id.to_string()).bind(d.project_id.to_string()).bind(d.experiment_id.to_string()).bind(d.experiment_event_id.to_string()).bind(d.private_image_id.to_string()).bind(d.attachment_id.to_string()).bind(&d.image_sha256).bind(&d.provider).bind(&d.model).bind(d.tool_run_id.map(|v|v.to_string())).bind(encode(&d.status)?).bind(serde_json::to_string(&d.items).map_err(|e|StoreError::Serialization(e.to_string()))?).bind(&d.error_code).bind(d.meta.created_at).bind(d.meta.updated_at).bind(d.meta.deleted_at).bind(d.meta.revision).execute(&mut*t).await.map_err(map_sqlx)?;
        sqlx::query("UPDATE ai_private_images SET status='pending_approval',last_activity_at=?,expires_at=?,updated_at=?,revision=revision+1 WHERE id=?").bind(d.meta.created_at).bind(d.meta.created_at+Duration::days(30)).bind(d.meta.created_at).bind(d.private_image_id.to_string()).execute(&mut*t).await.map_err(map_sqlx)?;
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
        t.commit().await.map_err(map_sqlx)
    }
    async fn get_ai_extraction_draft(&self, id: Uuid) -> StoreResult<AiExtractionDraft> {
        let r = sqlx::query(&format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE id=? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
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
        let mut q = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE lab_id="
        ));
        q.push_bind(l.to_string())
            .push(" AND user_id=")
            .push_bind(u.to_string())
            .push(" AND deleted_at IS NULL");
        if let Some(v) = p {
            q.push(" AND project_id=").push_bind(v.to_string());
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
            return Err(StoreError::Validation("selected indexes invalid".into()));
        }
        let mut t = self.pool.begin().await.map_err(map_sqlx)?;
        let r = sqlx::query(&format!(
            "SELECT {XC} FROM ai_extraction_drafts WHERE id=? AND deleted_at IS NULL"
        ))
        .bind(id.to_string())
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
            return Err(StoreError::Conflict("extraction resolved".into()));
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
        sqlx::query("UPDATE ai_extraction_drafts SET status=?,items_json=?,updated_at=?,revision=? WHERE id=?").bind(encode(&d.status)?).bind(serde_json::to_string(&d.items).map_err(|e|StoreError::Serialization(e.to_string()))?).bind(d.meta.updated_at).bind(d.meta.revision).bind(id.to_string()).execute(&mut*t).await.map_err(map_sqlx)?;
        sqlx::query("UPDATE ai_private_images SET status='active',last_activity_at=?,expires_at=?,updated_at=?,revision=revision+1 WHERE id=?").bind(d.meta.updated_at).bind(d.meta.updated_at+Duration::days(30)).bind(d.meta.updated_at).bind(d.private_image_id.to_string()).execute(&mut*t).await.map_err(map_sqlx)?;
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
        let rows=sqlx::query(&format!("SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE lab_id=? AND project_id=? AND deleted_at IS NULL ORDER BY created_at DESC,id")).bind(l.to_string()).bind(p.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx)?;
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
