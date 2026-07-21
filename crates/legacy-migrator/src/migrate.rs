use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use chrono::Utc;
use muriarc_core::LOCAL_LAB_ID;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::{
    EntityCounts, LegacyMigrationError, MigrationReport, RejectedPedigreeLink, Result,
    TOOL_VERSION, TargetVerification,
    audit::{audit_legacy, load_legacy_data, open_legacy, sha256_file},
    model::{LegacyCage, LegacyData, LegacyMouse, LegacyPedigree},
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

pub async fn migrate_legacy(source: &Path, target: &Path) -> Result<MigrationReport> {
    reject_existing_target(target)?;
    let parent = target_parent(target)?;
    let started_at = Utc::now();
    let audit = audit_legacy(source).await?;
    let blocking = audit.blocking_issues();
    if !blocking.is_empty() {
        return Err(LegacyMigrationError::AuditBlocked(blocking));
    }

    let source_sha256_before = audit.source.sha256_before.clone();
    let source_pool = open_legacy(source).await?;
    let data = load_legacy_data(&source_pool).await?;
    source_pool.close().await;
    let (_, hash_after_read) = sha256_file(source)?;
    if source_sha256_before != hash_after_read {
        return Err(LegacyMigrationError::SourceChanged {
            before: source_sha256_before,
            after: hash_after_read,
        });
    }

    // Build in a sibling temporary file. persist_noclobber makes the final
    // publication step fail rather than replace a target created concurrently.
    let staged = NamedTempFile::new_in(parent)?;
    let staged_path = staged.path().to_path_buf();
    let target_pool = open_target(&staged_path).await?;
    MIGRATOR.run(&target_pool).await?;
    validate_target_schema(&target_pool).await?;

    let (migrated, rejected_pedigree_links, expected_cage_counts) =
        insert_data(&target_pool, &data, &source_sha256_before, started_at).await?;
    let mut verification = verify_target(
        &target_pool,
        &data,
        &source_sha256_before,
        &migrated,
        &rejected_pedigree_links,
        &expected_cage_counts,
    )
    .await?;
    target_pool.close().await;

    let (_, source_sha256_after) = sha256_file(source)?;
    if source_sha256_before != source_sha256_after {
        return Err(LegacyMigrationError::SourceChanged {
            before: source_sha256_before,
            after: source_sha256_after,
        });
    }

    staged.as_file().sync_all()?;
    reject_existing_target(target)?;
    let persisted = staged.persist_noclobber(target).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            LegacyMigrationError::TargetExists(target.display().to_string())
        } else {
            LegacyMigrationError::Io(error.error)
        }
    })?;
    persisted.sync_all()?;
    drop(persisted);

    let (_, target_sha256) = sha256_file(target)?;
    verification.target_sha256 = target_sha256;
    verification.source_hash_unchanged = true;

    Ok(MigrationReport {
        tool_version: TOOL_VERSION.to_owned(),
        started_at,
        finished_at: Utc::now(),
        source_sha256_before,
        source_sha256_after,
        target_path: target.display().to_string(),
        audit,
        migrated,
        rejected_pedigree_links,
        verification,
    })
}

fn reject_existing_target(target: &Path) -> Result<()> {
    if fs::symlink_metadata(target).is_ok() {
        Err(LegacyMigrationError::TargetExists(
            target.display().to_string(),
        ))
    } else {
        Ok(())
    }
}

fn target_parent(target: &Path) -> Result<&Path> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent.is_dir() {
        Ok(parent)
    } else {
        Err(LegacyMigrationError::TargetParentMissing(
            parent.display().to_string(),
        ))
    }
}

async fn open_target(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true);
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

async fn validate_target_schema(pool: &SqlitePool) -> Result<()> {
    let required = [
        "labs",
        "cages",
        "animals",
        "gene_loci",
        "alleles",
        "genotypes",
        "pedigrees",
        "audit_entries",
    ];
    let tables: BTreeSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    for table in required {
        if !tables.contains(table) {
            return Err(LegacyMigrationError::Verification(format!(
                "target migration did not create required table {table}"
            )));
        }
    }
    Ok(())
}

async fn insert_data(
    pool: &SqlitePool,
    data: &LegacyData,
    source_hash: &str,
    migrated_at: chrono::DateTime<Utc>,
) -> Result<(
    EntityCounts,
    Vec<RejectedPedigreeLink>,
    BTreeMap<String, u64>,
)> {
    let timestamp = migrated_at.to_rfc3339();
    let lab_id = LOCAL_LAB_ID;
    let mut counts = EntityCounts::default();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, NULL, 1)",
    )
    .bind(lab_id.to_string())
    .bind("MuriArc legacy import")
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await?;
    counts.labs = 1;
    insert_audit(
        &mut tx,
        source_hash,
        lab_id,
        "lab",
        &lab_id.to_string(),
        "database",
        source_hash,
        json!({"source_sha256": source_hash}),
        &timestamp,
    )
    .await?;
    counts.audit_entries += 1;

    let actual_legacy_cage_counts = actual_legacy_cage_counts(data);
    let mut cage_ids = BTreeMap::new();
    let mut expected_cage_counts = BTreeMap::new();
    for cage in &data.cages {
        let cage_id = stable_uuid(source_hash, "cage", &cage.id.to_string());
        cage_ids.insert(cage.id, cage_id);
        let actual_count = actual_legacy_cage_counts
            .get(&cage.id)
            .copied()
            .unwrap_or_default();
        expected_cage_counts.insert(cage_id.to_string(), actual_count);
        sqlx::query(
            r#"INSERT INTO cages
               (id, lab_id, section, display_id, location, kind, sort_order,
                created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 1)"#,
        )
        .bind(cage_id.to_string())
        .bind(lab_id.to_string())
        .bind(&cage.section)
        .bind(&cage.display_id)
        .bind(normalize_optional(cage.location.as_deref()))
        .bind(cage_kind(cage))
        .bind(cage.sort_order)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.cages += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "cage",
            &cage_id.to_string(),
            "cage",
            &cage.id.to_string(),
            json!({
                "legacy_id": cage.id,
                "legacy_cached_mouse_count": cage.mice_count,
                "recalculated_mouse_count": actual_count,
                "legacy_mice_birth_date": cage.mice_birth_date,
                "legacy_mice_sex": cage.mice_sex,
                "legacy_mice_genotype": cage.mice_genotype,
            }),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    let source_scope = &source_hash[..12];
    let mut mouse_ids = BTreeMap::new();
    for mouse in &data.mice {
        let animal_id = stable_uuid(source_hash, "mouse", &mouse.tid.to_string());
        mouse_ids.insert(mouse.tid, animal_id);
        let current_cage_id = mouse
            .cage_id
            .map(|legacy_id| {
                cage_ids.get(&legacy_id).copied().ok_or_else(|| {
                    LegacyMigrationError::InvalidReference(format!(
                        "mouse {} references cage {legacy_id}",
                        mouse.tid
                    ))
                })
            })
            .transpose()?;
        sqlx::query(
            r#"INSERT INTO animals
               (id, lab_id, identifier_scope, display_id, legacy_id, species,
                strain, sex, birth_date, death_date, current_cage_id,
                current_status, created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 1)"#,
        )
        .bind(animal_id.to_string())
        .bind(lab_id.to_string())
        .bind(format!(
            "legacy:murispro:{source_scope}:mouse:{}",
            mouse.tid
        ))
        .bind(&mouse.display_id)
        .bind(format!("murispro:mouse:{}", mouse.tid))
        .bind("Mus musculus")
        .bind(normalize_optional(mouse.strain.as_deref()))
        .bind(mouse_sex(mouse))
        .bind(&mouse.birth_date)
        .bind(&mouse.death_date)
        .bind(current_cage_id.map(|id| id.to_string()))
        .bind(mouse_status(mouse))
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.animals += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "animal",
            &animal_id.to_string(),
            "mouse",
            &mouse.tid.to_string(),
            json!({
                "legacy_tid": mouse.tid,
                "legacy_display_id": mouse.display_id,
                "legacy_tests_planned": mouse.tests_planned,
            }),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    let mut locus_ids = BTreeMap::new();
    for locus in &data.loci {
        let locus_id = stable_uuid(source_hash, "gene_locus", &locus.id.to_string());
        locus_ids.insert(locus.id, locus_id);
        sqlx::query(
            r#"INSERT INTO gene_loci
               (id, lab_id, symbol, description, created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, ?, NULL, 1)"#,
        )
        .bind(locus_id.to_string())
        .bind(lab_id.to_string())
        .bind(&locus.symbol)
        .bind(normalize_optional(locus.description.as_deref()))
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.gene_loci += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "gene_locus",
            &locus_id.to_string(),
            "gene_locus",
            &locus.id.to_string(),
            json!({"legacy_id": locus.id, "symbol": locus.symbol}),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    let mut allele_ids = BTreeMap::new();
    for allele in &data.alleles {
        let allele_id = stable_uuid(source_hash, "allele", &allele.id.to_string());
        allele_ids.insert(allele.id, allele_id);
        let locus_id = lookup(&locus_ids, allele.locus_id, "allele locus")?;
        sqlx::query(
            r#"INSERT INTO alleles
               (id, locus_id, symbol, description, is_wild_type,
                created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 1)"#,
        )
        .bind(allele_id.to_string())
        .bind(locus_id.to_string())
        .bind(&allele.symbol)
        .bind(normalize_optional(allele.description.as_deref()))
        .bind(if allele.is_wildtype == Some(1) {
            1_i64
        } else {
            0_i64
        })
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.alleles += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "allele",
            &allele_id.to_string(),
            "allele",
            &allele.id.to_string(),
            json!({"legacy_id": allele.id, "legacy_is_wildtype": allele.is_wildtype}),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    for genotype in &data.genotypes {
        let genotype_id = stable_uuid(source_hash, "genotype", &genotype.id.to_string());
        let animal_id = lookup(&mouse_ids, genotype.mouse_id, "genotype mouse")?;
        let locus_id = lookup(&locus_ids, genotype.locus_id, "genotype locus")?;
        let allele_1_id = optional_lookup(&allele_ids, genotype.allele1_id, "genotype allele1")?;
        let allele_2_id = optional_lookup(&allele_ids, genotype.allele2_id, "genotype allele2")?;
        sqlx::query(
            r#"INSERT INTO genotypes
               (id, animal_id, locus_id, allele_1_id, allele_2_id, assessed_at,
                created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, NULL, ?, ?, NULL, 1)"#,
        )
        .bind(genotype_id.to_string())
        .bind(animal_id.to_string())
        .bind(locus_id.to_string())
        .bind(allele_1_id.map(|id| id.to_string()))
        .bind(allele_2_id.map(|id| id.to_string()))
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.genotypes += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "genotype",
            &genotype_id.to_string(),
            "genotype",
            &genotype.id.to_string(),
            json!({"legacy_id": genotype.id}),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    let mut rejected = Vec::new();
    for pedigree in &data.pedigrees {
        let reasons = pedigree_rejection_reasons(pedigree, &mouse_ids);
        if !reasons.is_empty() {
            rejected.push(RejectedPedigreeLink {
                legacy_pedigree_id: pedigree.id,
                child_legacy_tid: pedigree.mouse_id,
                parent_legacy_tid: pedigree.parent_id,
                parent_type: pedigree.parent_type.clone(),
                reasons,
            });
            continue;
        }
        let child_legacy_id = pedigree.mouse_id.expect("validated child id");
        let parent_legacy_id = pedigree.parent_id.expect("validated parent id");
        let child_id = lookup(&mouse_ids, child_legacy_id, "pedigree child")?;
        let parent_id = lookup(&mouse_ids, parent_legacy_id, "pedigree parent")?;
        let pedigree_id = stable_uuid(source_hash, "pedigree", &pedigree.id.to_string());
        sqlx::query(
            r#"INSERT INTO pedigrees
               (id, animal_id, parent_id, parent_type, created_at, updated_at, deleted_at, revision)
               VALUES (?, ?, ?, ?, ?, ?, NULL, 1)"#,
        )
        .bind(pedigree_id.to_string())
        .bind(child_id.to_string())
        .bind(parent_id.to_string())
        .bind(parent_type(pedigree.parent_type.as_deref()))
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
        counts.pedigrees += 1;
        insert_audit(
            &mut tx,
            source_hash,
            lab_id,
            "pedigree",
            &pedigree_id.to_string(),
            "pedigree",
            &pedigree.id.to_string(),
            json!({"legacy_id": pedigree.id}),
            &timestamp,
        )
        .await?;
        counts.audit_entries += 1;
    }

    tx.commit().await?;
    Ok((counts, rejected, expected_cage_counts))
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &mut Transaction<'_, Sqlite>,
    source_hash: &str,
    lab_id: Uuid,
    entity_type: &str,
    entity_id: &str,
    legacy_table: &str,
    legacy_pk: &str,
    after_json: Value,
    occurred_at: &str,
) -> Result<()> {
    let audit_id = stable_uuid(source_hash, "audit", &format!("{entity_type}:{entity_id}"));
    sqlx::query(
        r#"INSERT INTO audit_entries
           (id, lab_id, project_id, entity_type, entity_id, action, actor_type,
            actor_user_id, actor_display_name, source, request_id, reason,
            before_json, after_json, occurred_at)
           VALUES (?, ?, NULL, ?, ?, 'import', 'system', NULL,
                   'MuriArc legacy migrator', 'migration', NULL, ?, NULL, ?, ?)"#,
    )
    .bind(audit_id.to_string())
    .bind(lab_id.to_string())
    .bind(entity_type)
    .bind(entity_id)
    .bind(format!("MurisPro {legacy_table} row {legacy_pk}"))
    .bind(serde_json::to_string(&after_json)?)
    .bind(occurred_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn verify_target(
    pool: &SqlitePool,
    data: &LegacyData,
    source_hash: &str,
    migrated: &EntityCounts,
    rejected: &[RejectedPedigreeLink],
    expected_cage_counts: &BTreeMap<String, u64>,
) -> Result<TargetVerification> {
    let counts = target_counts(pool).await?;
    let expected = EntityCounts {
        labs: 1,
        cages: data.cages.len() as u64,
        animals: data.mice.len() as u64,
        gene_loci: data.loci.len() as u64,
        alleles: data.alleles.len() as u64,
        genotypes: data.genotypes.len() as u64,
        pedigrees: (data.pedigrees.len() - rejected.len()) as u64,
        audit_entries: migrated.audit_entries,
    };
    if counts != expected || migrated != &expected {
        return Err(LegacyMigrationError::Verification(format!(
            "entity counts differ: expected {expected:?}, inserted {migrated:?}, queried {counts:?}"
        )));
    }

    let foreign_key_violations = foreign_key_violations(pool).await?;
    if !foreign_key_violations.is_empty() {
        return Err(LegacyMigrationError::Verification(format!(
            "target has foreign key violations: {foreign_key_violations:?}"
        )));
    }

    let rows = sqlx::query(
        r#"SELECT c.id, COUNT(a.id) AS actual_count
           FROM cages c
           LEFT JOIN animals a ON a.current_cage_id = c.id AND a.deleted_at IS NULL
           GROUP BY c.id ORDER BY c.id"#,
    )
    .fetch_all(pool)
    .await?;
    let cage_actual_counts: BTreeMap<String, u64> = rows
        .into_iter()
        .map(|row| {
            let id: String = row.get("id");
            let count: i64 = row.get("actual_count");
            (id, count.max(0) as u64)
        })
        .collect();
    if &cage_actual_counts != expected_cage_counts {
        return Err(LegacyMigrationError::Verification(format!(
            "recalculated cage counts differ: expected {expected_cage_counts:?}, got {cage_actual_counts:?}"
        )));
    }

    let animals = sqlx::query("SELECT id, legacy_id FROM animals ORDER BY legacy_id")
        .fetch_all(pool)
        .await?;
    let deterministic_ids_verified = animals.into_iter().all(|row| {
        let id: String = row.get("id");
        let legacy_id: Option<String> = row.get("legacy_id");
        legacy_id
            .and_then(|value| value.strip_prefix("murispro:mouse:").map(str::to_owned))
            .map(|legacy_pk| stable_uuid(source_hash, "mouse", &legacy_pk).to_string() == id)
            .unwrap_or(false)
    });
    if !deterministic_ids_verified {
        return Err(LegacyMigrationError::Verification(
            "one or more animal UUIDs are not deterministic legacy IDs".to_owned(),
        ));
    }

    Ok(TargetVerification {
        target_sha256: String::new(),
        foreign_key_violations,
        counts,
        cage_actual_counts,
        source_hash_unchanged: false,
        deterministic_ids_verified,
    })
}

async fn target_counts(pool: &SqlitePool) -> Result<EntityCounts> {
    Ok(EntityCounts {
        labs: count(pool, "labs").await?,
        cages: count(pool, "cages").await?,
        animals: count(pool, "animals").await?,
        gene_loci: count(pool, "gene_loci").await?,
        alleles: count(pool, "alleles").await?,
        genotypes: count(pool, "genotypes").await?,
        pedigrees: count(pool, "pedigrees").await?,
        audit_entries: count(pool, "audit_entries").await?,
    })
}

async fn count(pool: &SqlitePool, table: &str) -> Result<u64> {
    let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await?;
    Ok(count.max(0) as u64)
}

async fn foreign_key_violations(pool: &SqlitePool) -> Result<Vec<String>> {
    Ok(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| {
            let table: String = row.get("table");
            let rowid: Option<i64> = row.get("rowid");
            let parent: String = row.get("parent");
            format!("table={table}, rowid={rowid:?}, parent={parent}")
        })
        .collect())
}

fn stable_uuid(source_hash: &str, entity: &str, legacy_pk: &str) -> Uuid {
    let digest = Sha256::digest(
        format!("MuriArc\0MurisPro\0{source_hash}\0{entity}\0{legacy_pk}").as_bytes(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUIDv8 is explicitly reserved for application-defined deterministic IDs.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn actual_legacy_cage_counts(data: &LegacyData) -> BTreeMap<i64, u64> {
    let mut counts = BTreeMap::new();
    for cage_id in data.mice.iter().filter_map(|mouse| mouse.cage_id) {
        *counts.entry(cage_id).or_default() += 1;
    }
    counts
}

fn cage_kind(cage: &LegacyCage) -> &'static str {
    if cage.display_id.contains("暂存") {
        "temporary"
    } else {
        match cage.cage_type.as_deref() {
            Some("breeding") => "breeding",
            Some("experimental") => "experimental",
            _ => "standard",
        }
    }
}

fn mouse_sex(mouse: &LegacyMouse) -> &'static str {
    match mouse.sex.as_deref() {
        Some("M") => "male",
        Some("F") => "female",
        _ => "unknown",
    }
}

fn mouse_status(mouse: &LegacyMouse) -> &'static str {
    match mouse.live_status {
        Some(1) => "alive",
        Some(0) => "deceased",
        _ => "archived",
    }
}

fn parent_type(value: Option<&str>) -> &'static str {
    match value {
        Some("father") => "father",
        Some("mother") => "mother",
        _ => "unknown",
    }
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn lookup(map: &BTreeMap<i64, Uuid>, key: i64, relation: &str) -> Result<Uuid> {
    map.get(&key).copied().ok_or_else(|| {
        LegacyMigrationError::InvalidReference(format!("{relation} references {key}"))
    })
}

fn optional_lookup(
    map: &BTreeMap<i64, Uuid>,
    key: Option<i64>,
    relation: &str,
) -> Result<Option<Uuid>> {
    key.map(|key| lookup(map, key, relation)).transpose()
}

fn pedigree_rejection_reasons(
    pedigree: &LegacyPedigree,
    mouse_ids: &BTreeMap<i64, Uuid>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    match pedigree.mouse_id {
        None => reasons.push("missing_child_legacy_tid".to_owned()),
        Some(id) if !mouse_ids.contains_key(&id) => reasons.push("child_not_found".to_owned()),
        Some(_) => {}
    }
    match pedigree.parent_id {
        None => reasons.push("missing_parent_legacy_tid".to_owned()),
        Some(id) if !mouse_ids.contains_key(&id) => reasons.push("parent_not_found".to_owned()),
        Some(_) => {}
    }
    if pedigree.mouse_id.is_some() && pedigree.mouse_id == pedigree.parent_id {
        reasons.push("self_parent_relation".to_owned());
    }
    reasons
}
