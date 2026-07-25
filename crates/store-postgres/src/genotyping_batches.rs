use super::*;

const BATCH_COLUMNS: &str = "id, lab_id, project_id, batch_number, genotype_definition_id, assessed_at, method, notes, status, created_by, source_attachment_id, preview_hash, preview_row_count, committed_at, cancelled_at, cancel_reason, created_at, updated_at, deleted_at, revision";

fn batch_from_row(row: &PgRow) -> StoreResult<GenotypingBatch> {
    Ok(GenotypingBatch {
        id: row.try_get("id").map_err(map_sqlx)?,
        lab_id: row.try_get("lab_id").map_err(map_sqlx)?,
        project_id: row.try_get("project_id").map_err(map_sqlx)?,
        batch_number: row.try_get("batch_number").map_err(map_sqlx)?,
        genotype_definition_id: row.try_get("genotype_definition_id").map_err(map_sqlx)?,
        assessed_at: row.try_get("assessed_at").map_err(map_sqlx)?,
        method: row.try_get("method").map_err(map_sqlx)?,
        notes: row.try_get("notes").map_err(map_sqlx)?,
        status: decode(row.try_get("status").map_err(map_sqlx)?)?,
        created_by: row.try_get("created_by").map_err(map_sqlx)?,
        source_attachment_id: row.try_get("source_attachment_id").map_err(map_sqlx)?,
        preview_hash: row.try_get("preview_hash").map_err(map_sqlx)?,
        preview_row_count: row.try_get("preview_row_count").map_err(map_sqlx)?,
        committed_at: row.try_get("committed_at").map_err(map_sqlx)?,
        cancelled_at: row.try_get("cancelled_at").map_err(map_sqlx)?,
        cancel_reason: row.try_get("cancel_reason").map_err(map_sqlx)?,
        meta: meta(row)?,
    })
}

pub(crate) async fn evidence_for_records(
    pool: &PgPool,
    record_ids: &[Uuid],
) -> StoreResult<BTreeMap<Uuid, GenotypingBatchEvidence>> {
    if record_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT
            gbr.record_id,
            gb.id AS batch_id,
            gb.batch_number,
            gb.status AS batch_status,
            gb.assessed_at AS batch_assessed_at,
            gb.method AS batch_method,
            gb.notes AS batch_notes,
            gb.revision AS batch_revision,
            attachment.id AS attachment_id,
            attachment.file_name AS attachment_file_name,
            attachment.media_type AS attachment_media_type,
            attachment.size_bytes AS attachment_size_bytes,
            attachment.version AS attachment_version,
            attachment.revision AS attachment_revision
         FROM genotyping_batch_records gbr
         JOIN genotyping_batches gb
           ON gb.id = gbr.batch_id
          AND gb.deleted_at IS NULL
         LEFT JOIN attachments attachment
           ON attachment.entity_type = 'genotyping_batch'
          AND attachment.entity_id = gb.id
          AND attachment.deleted_at IS NULL
          AND attachment.media_type LIKE 'image/%'
          AND (gb.source_attachment_id IS NULL OR attachment.id <> gb.source_attachment_id)
         WHERE gbr.record_id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for record_id in record_ids {
            separated.push_bind(*record_id);
        }
    }
    query.push(") ORDER BY gbr.record_id, attachment.created_at, attachment.id");

    let rows = query.build().fetch_all(pool).await.map_err(map_sqlx)?;
    let mut evidence = BTreeMap::new();
    for row in &rows {
        let record_id: Uuid = row.try_get("record_id").map_err(map_sqlx)?;
        let batch_id: Uuid = row.try_get("batch_id").map_err(map_sqlx)?;
        let batch_number = row.try_get("batch_number").map_err(map_sqlx)?;
        let status = decode(row.try_get("batch_status").map_err(map_sqlx)?)?;
        let assessed_at = row.try_get("batch_assessed_at").map_err(map_sqlx)?;
        let method = row.try_get("batch_method").map_err(map_sqlx)?;
        let notes = row.try_get("batch_notes").map_err(map_sqlx)?;
        let revision = row.try_get("batch_revision").map_err(map_sqlx)?;
        let item = evidence
            .entry(record_id)
            .or_insert_with(|| GenotypingBatchEvidence {
                id: batch_id,
                batch_number,
                status,
                assessed_at,
                method,
                notes,
                revision,
                gel_attachments: Vec::new(),
            });
        if item.id != batch_id {
            return Err(StoreError::Serialization(
                "one genotyping record is linked to multiple batches".to_owned(),
            ));
        }
        let attachment_id: Option<Uuid> = row.try_get("attachment_id").map_err(map_sqlx)?;
        if let Some(attachment_id) = attachment_id {
            item.gel_attachments.push(GenotypingEvidenceAttachment {
                id: attachment_id,
                file_name: row.try_get("attachment_file_name").map_err(map_sqlx)?,
                media_type: row.try_get("attachment_media_type").map_err(map_sqlx)?,
                size_bytes: row.try_get("attachment_size_bytes").map_err(map_sqlx)?,
                version: row.try_get("attachment_version").map_err(map_sqlx)?,
                revision: row.try_get("attachment_revision").map_err(map_sqlx)?,
            });
        }
    }
    Ok(evidence)
}

async fn locked_batch(tx: &mut PgTransaction<'_>, id: Uuid) -> StoreResult<GenotypingBatch> {
    let row = sqlx::query(&format!(
        "SELECT {BATCH_COLUMNS} FROM genotyping_batches WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "genotyping_batch",
        id,
    })?;
    batch_from_row(&row)
}

fn validate_actor(batch: &GenotypingBatch, audit: &AuditContext) -> StoreResult<()> {
    if audit.actor.actor_type == ActorType::Human && batch.created_by != audit.actor.user_id {
        return Err(StoreError::Validation(
            "genotyping batch created_by must match the human audit actor".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) async fn create(
    pool: &PgPool,
    batch: &GenotypingBatch,
    audit: &AuditContext,
) -> StoreResult<()> {
    batch
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    validate_actor(batch, audit)?;
    if batch.status != GenotypingBatchStatus::Draft
        || batch.source_attachment_id.is_some()
        || batch.preview_hash.is_some()
        || batch.preview_row_count.is_some()
    {
        return Err(StoreError::Validation(
            "new genotyping batch must be an empty draft".to_owned(),
        ));
    }
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let definition_lab = active_lab_id_for(
        &mut tx,
        "genotype_definitions",
        "genotype_definition",
        batch.genotype_definition_id,
    )
    .await?;
    if definition_lab != batch.lab_id {
        return Err(StoreError::Validation(
            "genotyping batch definition belongs to a different lab".to_owned(),
        ));
    }
    if let Some(project_id) = batch.project_id {
        ensure_project_in_lab(&mut tx, project_id, batch.lab_id).await?;
    }
    sqlx::query(
        "INSERT INTO genotyping_batches (
            id, lab_id, project_id, batch_number, genotype_definition_id,
            assessed_at, method, notes, status, created_by,
            source_attachment_id, preview_hash, preview_row_count,
            committed_at, cancelled_at, cancel_reason,
            created_at, updated_at, deleted_at, revision
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15, $16, $17, $18, $19, $20
         )",
    )
    .bind(batch.id)
    .bind(batch.lab_id)
    .bind(batch.project_id)
    .bind(&batch.batch_number)
    .bind(batch.genotype_definition_id)
    .bind(batch.assessed_at)
    .bind(&batch.method)
    .bind(&batch.notes)
    .bind(encode(&batch.status)?)
    .bind(batch.created_by)
    .bind(batch.source_attachment_id)
    .bind(&batch.preview_hash)
    .bind(batch.preview_row_count)
    .bind(batch.committed_at)
    .bind(batch.cancelled_at)
    .bind(&batch.cancel_reason)
    .bind(batch.meta.created_at)
    .bind(batch.meta.updated_at)
    .bind(batch.meta.deleted_at)
    .bind(batch.meta.revision)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    write_audit(
        &mut tx,
        batch.lab_id,
        batch.project_id,
        EntityType::GenotypingBatch,
        batch.id,
        AuditAction::Create,
        audit,
        None,
        Some(snapshot(batch)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        batch.lab_id,
        batch.project_id,
        EntityType::GenotypingBatch,
        batch.id,
        audit,
        batch.meta.created_at,
    );
    insert_provenance(&mut tx, &provenance).await?;
    tx.commit().await.map_err(map_sqlx)
}

pub(crate) async fn get(pool: &PgPool, id: Uuid) -> StoreResult<GenotypingBatch> {
    let row = sqlx::query(&format!(
        "SELECT {BATCH_COLUMNS} FROM genotyping_batches WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "genotyping_batch",
        id,
    })?;
    batch_from_row(&row)
}

pub(crate) async fn list(
    pool: &PgPool,
    filter: &GenotypingBatchFilter,
) -> StoreResult<Vec<GenotypingBatch>> {
    let rows = sqlx::query(&format!(
        "SELECT {BATCH_COLUMNS}
         FROM genotyping_batches
         WHERE lab_id = $1
           AND deleted_at IS NULL
           AND ($2::uuid IS NULL OR project_id = $2)
           AND ($3::text IS NULL OR status = $3)
         ORDER BY assessed_at DESC, created_at DESC, id"
    ))
    .bind(filter.lab_id)
    .bind(filter.project_id)
    .bind(filter.status.map(|status| encode(&status)).transpose()?)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(batch_from_row).collect()
}

async fn ensure_preview_attachment(
    tx: &mut PgTransaction<'_>,
    batch: &GenotypingBatch,
    attachment_id: Uuid,
) -> StoreResult<()> {
    let row = sqlx::query(
        "SELECT lab_id, project_id, entity_type, entity_id, media_type
         FROM attachments
         WHERE id = $1 AND deleted_at IS NULL
         FOR UPDATE",
    )
    .bind(attachment_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?
    .ok_or(StoreError::NotFound {
        entity: "attachment",
        id: attachment_id,
    })?;
    let lab_id: Uuid = row.try_get("lab_id").map_err(map_sqlx)?;
    let project_id: Option<Uuid> = row.try_get("project_id").map_err(map_sqlx)?;
    let entity_type: String = row.try_get("entity_type").map_err(map_sqlx)?;
    let entity_id: Uuid = row.try_get("entity_id").map_err(map_sqlx)?;
    let media_type: Option<String> = row.try_get("media_type").map_err(map_sqlx)?;
    if lab_id != batch.lab_id
        || project_id != batch.project_id
        || entity_type != "genotyping_batch"
        || entity_id != batch.id
        || media_type
            .as_deref()
            .is_some_and(|value| value.starts_with("image/"))
    {
        return Err(StoreError::Validation(
            "genotyping preview source attachment has an invalid scope or type".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) async fn set_preview(
    pool: &PgPool,
    id: Uuid,
    expected_revision: i64,
    preview: &GenotypingBatchPreview,
    updated_at: DateTime<Utc>,
    audit: &AuditContext,
) -> StoreResult<GenotypingBatch> {
    preview
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let before = locked_batch(&mut tx, id).await?;
    if before.status != GenotypingBatchStatus::Draft {
        return Err(StoreError::Conflict(
            "only a draft genotyping batch can receive a preview".to_owned(),
        ));
    }
    if before.meta.revision != expected_revision {
        return Err(StoreError::Conflict(
            "genotyping batch revision does not match".to_owned(),
        ));
    }
    ensure_preview_attachment(&mut tx, &before, preview.source_attachment_id).await?;
    let row = sqlx::query(&format!(
        "UPDATE genotyping_batches
         SET source_attachment_id = $2, preview_hash = $3, preview_row_count = $4,
             updated_at = $5, revision = $6
         WHERE id = $1 AND revision = $7 AND status = 'draft' AND deleted_at IS NULL
         RETURNING {BATCH_COLUMNS}"
    ))
    .bind(id)
    .bind(preview.source_attachment_id)
    .bind(&preview.preview_hash)
    .bind(preview.row_count)
    .bind(updated_at)
    .bind(expected_revision + 1)
    .bind(expected_revision)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::Conflict("genotyping batch changed concurrently".to_owned()))?;
    let after = batch_from_row(&row)?;
    after
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    write_audit(
        &mut tx,
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        AuditAction::Update,
        audit,
        Some(snapshot(&before)?),
        Some(snapshot(&after)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        audit,
        updated_at,
    );
    insert_provenance(&mut tx, &provenance).await?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(after)
}

async fn ensure_commit_evidence(
    tx: &mut PgTransaction<'_>,
    batch: &GenotypingBatch,
) -> StoreResult<()> {
    let source_id = batch.source_attachment_id.ok_or_else(|| {
        StoreError::Validation("genotyping batch has no source attachment".to_owned())
    })?;
    ensure_preview_attachment(tx, batch, source_id).await?;
    let image_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM attachments
         WHERE lab_id = $1
           AND project_id IS NOT DISTINCT FROM $2
           AND entity_type = 'genotyping_batch'
           AND entity_id = $3
           AND deleted_at IS NULL
           AND media_type LIKE 'image/%'",
    )
    .bind(batch.lab_id)
    .bind(batch.project_id)
    .bind(batch.id)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    if image_count < 1 {
        return Err(StoreError::Validation(
            "genotyping batch requires at least one gel image attachment".to_owned(),
        ));
    }
    Ok(())
}

async fn ensure_record_scope(
    tx: &mut PgTransaction<'_>,
    batch: &GenotypingBatch,
    record: &GenotypingRecord,
) -> StoreResult<()> {
    record
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    if record.lab_id != batch.lab_id
        || record.project_id != batch.project_id
        || record.genotype_definition_id != batch.genotype_definition_id
        || record.assessed_at != Some(batch.assessed_at)
        || record.method != batch.method
        || record.supersedes_record_id.is_some()
        || record.is_voided()
        || record.meta.deleted_at.is_some()
    {
        return Err(StoreError::Validation(
            "genotyping batch record does not match the batch scope".to_owned(),
        ));
    }
    let animal_lab = active_lab_id_for(tx, "animals", "animal", record.animal_id).await?;
    if animal_lab != batch.lab_id {
        return Err(StoreError::Validation(
            "genotyping batch animal belongs to a different lab".to_owned(),
        ));
    }
    if let Some(project_id) = batch.project_id {
        let assigned: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM project_animal_assignments
                WHERE project_id = $1 AND animal_id = $2 AND lab_id = $3 AND deleted_at IS NULL
             )",
        )
        .bind(project_id)
        .bind(record.animal_id)
        .bind(batch.lab_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        if !assigned {
            return Err(StoreError::Validation(
                "genotyping batch animal is not assigned to the selected project".to_owned(),
            ));
        }
    }
    Ok(())
}

async fn insert_batch_record(
    tx: &mut PgTransaction<'_>,
    batch: &GenotypingBatch,
    record: &GenotypingRecord,
    display_order: i32,
    audit: &AuditContext,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO genotyping_records (
            id, lab_id, project_id, animal_id, genotype_definition_id, state,
            assessed_at, method, notes, supersedes_record_id, voided_at, void_reason,
            created_at, updated_at, deleted_at, revision
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            $9, $10, $11, $12, $13, $14, $15, $16
         )",
    )
    .bind(record.id)
    .bind(record.lab_id)
    .bind(record.project_id)
    .bind(record.animal_id)
    .bind(record.genotype_definition_id)
    .bind(encode(&record.state)?)
    .bind(record.assessed_at)
    .bind(&record.method)
    .bind(&record.notes)
    .bind(record.supersedes_record_id)
    .bind(record.voided_at)
    .bind(&record.void_reason)
    .bind(record.meta.created_at)
    .bind(record.meta.updated_at)
    .bind(record.meta.deleted_at)
    .bind(record.meta.revision)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    sqlx::query(
        "INSERT INTO genotyping_batch_records (batch_id, record_id, display_order)
         VALUES ($1, $2, $3)",
    )
    .bind(batch.id)
    .bind(record.id)
    .bind(display_order)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    write_audit(
        tx,
        record.lab_id,
        record.project_id,
        EntityType::GenotypingRecord,
        record.id,
        AuditAction::Create,
        audit,
        None,
        Some(snapshot(record)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        record.lab_id,
        record.project_id,
        EntityType::GenotypingRecord,
        record.id,
        audit,
        record.meta.created_at,
    );
    insert_provenance(tx, &provenance).await?;
    let mut event = AnimalEvent::new(
        record.lab_id,
        record.animal_id,
        AnimalEventKind::GenotypingRecorded {
            record_id: record.id,
            genotype_definition_id: record.genotype_definition_id,
            state: record.state,
        },
        record.assessed_at.unwrap_or(record.meta.created_at),
        record.meta.created_at,
    );
    event.project_id = record.project_id;
    event.recorded_by = audit.actor.user_id;
    append_derived_animal_event(tx, &event, audit)
        .await
        .map(|_| ())
}

pub(crate) async fn commit(
    pool: &PgPool,
    commit: &GenotypingBatchCommit,
    audit: &AuditContext,
) -> StoreResult<GenotypingBatchReceipt> {
    commit
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let before = locked_batch(&mut tx, commit.batch_id).await?;
    if before.status != GenotypingBatchStatus::Draft {
        return Err(StoreError::Conflict(
            "genotyping batch is no longer a draft".to_owned(),
        ));
    }
    if before.meta.revision != commit.expected_revision {
        return Err(StoreError::Conflict(
            "genotyping batch revision does not match".to_owned(),
        ));
    }
    if before.preview_hash.as_deref() != Some(commit.preview_hash.as_str())
        || before.preview_row_count != i64::try_from(commit.records.len()).ok()
    {
        return Err(StoreError::Conflict(
            "genotyping preview no longer matches the confirmed rows".to_owned(),
        ));
    }
    ensure_commit_evidence(&mut tx, &before).await?;
    let definition_lab = active_lab_id_for(
        &mut tx,
        "genotype_definitions",
        "genotype_definition",
        before.genotype_definition_id,
    )
    .await?;
    if definition_lab != before.lab_id {
        return Err(StoreError::Validation(
            "genotyping batch definition belongs to a different lab".to_owned(),
        ));
    }
    if let Some(project_id) = before.project_id {
        ensure_project_in_lab(&mut tx, project_id, before.lab_id).await?;
    }

    let mut animal_ids = commit
        .records
        .iter()
        .map(|record| record.animal_id)
        .collect::<Vec<_>>();
    animal_ids.sort_unstable();
    for animal_id in animal_ids {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("genotype-snapshot:{animal_id}"))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    for record in &commit.records {
        ensure_record_scope(&mut tx, &before, record).await?;
    }
    for (index, record) in commit.records.iter().enumerate() {
        let display_order = i32::try_from(index).map_err(|_| {
            StoreError::Validation("genotyping batch record order is too large".to_owned())
        })?;
        insert_batch_record(&mut tx, &before, record, display_order, audit).await?;
    }

    let row = sqlx::query(&format!(
        "UPDATE genotyping_batches
         SET status = 'committed', committed_at = $2, updated_at = $2, revision = $3
         WHERE id = $1 AND revision = $4 AND status = 'draft' AND deleted_at IS NULL
         RETURNING {BATCH_COLUMNS}"
    ))
    .bind(before.id)
    .bind(commit.committed_at)
    .bind(commit.expected_revision + 1)
    .bind(commit.expected_revision)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::Conflict("genotyping batch changed concurrently".to_owned()))?;
    let after = batch_from_row(&row)?;
    after
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    write_audit(
        &mut tx,
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        AuditAction::Update,
        audit,
        Some(snapshot(&before)?),
        Some(snapshot(&after)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        audit,
        commit.committed_at,
    );
    insert_provenance(&mut tx, &provenance).await?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(GenotypingBatchReceipt {
        batch: after,
        records: commit.records.clone(),
    })
}

pub(crate) async fn cancel(
    pool: &PgPool,
    id: Uuid,
    expected_revision: i64,
    reason: &str,
    cancelled_at: DateTime<Utc>,
    audit: &AuditContext,
) -> StoreResult<GenotypingBatch> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(StoreError::Validation(
            "genotyping batch cancellation reason must not be empty".to_owned(),
        ));
    }
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let before = locked_batch(&mut tx, id).await?;
    if before.status != GenotypingBatchStatus::Draft {
        return Err(StoreError::Conflict(
            "only a draft genotyping batch can be cancelled".to_owned(),
        ));
    }
    if before.meta.revision != expected_revision {
        return Err(StoreError::Conflict(
            "genotyping batch revision does not match".to_owned(),
        ));
    }
    let row = sqlx::query(&format!(
        "UPDATE genotyping_batches
         SET status = 'cancelled', cancelled_at = $2, cancel_reason = $3,
             updated_at = $2, revision = $4
         WHERE id = $1 AND revision = $5 AND status = 'draft' AND deleted_at IS NULL
         RETURNING {BATCH_COLUMNS}"
    ))
    .bind(id)
    .bind(cancelled_at)
    .bind(reason)
    .bind(expected_revision + 1)
    .bind(expected_revision)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::Conflict("genotyping batch changed concurrently".to_owned()))?;
    let after = batch_from_row(&row)?;
    after
        .validate()
        .map_err(|error| StoreError::Validation(error.to_string()))?;
    write_audit(
        &mut tx,
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        AuditAction::Update,
        audit,
        Some(snapshot(&before)?),
        Some(snapshot(&after)?),
    )
    .await?;
    let provenance = Provenance::from_audit(
        after.lab_id,
        after.project_id,
        EntityType::GenotypingBatch,
        after.id,
        audit,
        cancelled_at,
    );
    insert_provenance(&mut tx, &provenance).await?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(after)
}

pub(crate) async fn list_records(
    pool: &PgPool,
    batch_id: Uuid,
) -> StoreResult<Vec<GenotypingRecord>> {
    get(pool, batch_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT {GENOTYPING_RECORD_COLUMNS}
         FROM genotyping_records gr
         JOIN genotyping_batch_records gbr ON gbr.record_id = gr.id
         WHERE gbr.batch_id = $1
         ORDER BY gbr.display_order, gr.id"
    ))
    .bind(batch_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    rows.iter().map(genotyping_record_from_row).collect()
}

pub(crate) async fn find_for_record(
    pool: &PgPool,
    record_id: Uuid,
) -> StoreResult<Option<GenotypingBatch>> {
    let row = sqlx::query(&format!(
        "SELECT {BATCH_COLUMNS}
         FROM genotyping_batches gb
         JOIN genotyping_batch_records gbr ON gbr.batch_id = gb.id
         WHERE gbr.record_id = $1 AND gb.deleted_at IS NULL"
    ))
    .bind(record_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?;
    row.as_ref().map(batch_from_row).transpose()
}
