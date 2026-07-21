use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use chrono::{NaiveDate, Utc};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{
    CageCountMismatch, DuplicateIdentifierGroup, DuplicateIdentifierSummary, LegacyAuditReport,
    LegacyMigrationError, LegacySchemaReport, RejectedPedigreeLink, Result, SourceDigest,
    TOOL_VERSION, ValidationIssue, ValidationSeverity,
    model::{
        LegacyAllele, LegacyCage, LegacyData, LegacyGeneLocus, LegacyGenotype, LegacyMouse,
        LegacyPedigree,
    },
};

const REQUIRED_SCHEMA: &[(&str, &[&str])] = &[
    (
        "cage",
        &[
            "id",
            "section",
            "cage_id",
            "location",
            "cage_type",
            "order",
            "mice_birth_date",
            "mice_count",
            "mice_sex",
            "mice_genotype",
        ],
    ),
    (
        "mouse",
        &[
            "tid",
            "id",
            "sex",
            "live_status",
            "birth_date",
            "death_date",
            "cage_id",
            "strain",
            "tests_planned",
        ],
    ),
    ("gene_locus", &["id", "symbol", "description"]),
    (
        "allele",
        &["id", "symbol", "locus_id", "description", "is_wildtype"],
    ),
    (
        "genotype",
        &["id", "mouse_id", "locus_id", "allele1_id", "allele2_id"],
    ),
    ("pedigree", &["id", "mouse_id", "parent_id", "parent_type"]),
];

pub async fn audit_legacy(source: &Path) -> Result<LegacyAuditReport> {
    ensure_source(source)?;
    let (size_bytes, sha256_before) = sha256_file(source)?;
    let pool = open_legacy(source).await?;

    validate_required_schema(&pool).await?;
    let data = load_legacy_data(&pool).await?;
    let table_counts = table_counts(&pool).await?;
    let integrity_rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&pool)
        .await?;
    let integrity_check = integrity_rows.join("; ");

    let duplicate_identifiers = duplicate_identifiers(&data.mice);
    let cage_count_mismatches = cage_count_mismatches(&data.cages, &data.mice);
    let orphan_pedigree_links = orphan_pedigree_links(&data.pedigrees, &data.mice);
    let mut validation_issues = validate_data(&data, &integrity_check);
    append_foreign_key_issues(&pool, &mut validation_issues).await?;

    pool.close().await;
    let (_, sha256_after) = sha256_file(source)?;
    if sha256_before != sha256_after {
        return Err(LegacyMigrationError::SourceChanged {
            before: sha256_before,
            after: sha256_after,
        });
    }

    Ok(LegacyAuditReport {
        tool_version: TOOL_VERSION.to_owned(),
        audited_at: Utc::now(),
        source: SourceDigest {
            path: source.display().to_string(),
            size_bytes,
            sha256_before: sha256_before.clone(),
            sha256_after,
            unchanged: true,
        },
        schema: LegacySchemaReport {
            format: "murispro-sqlalchemy-v1".to_owned(),
            compatible: true,
            required_tables: REQUIRED_SCHEMA
                .iter()
                .map(|(table, _)| (*table).to_owned())
                .collect(),
        },
        integrity_check,
        table_counts,
        duplicate_identifiers,
        cage_count_mismatches,
        orphan_pedigree_links,
        validation_issues,
    })
}

pub(crate) fn ensure_source(source: &Path) -> Result<()> {
    if source.is_file() {
        Ok(())
    } else {
        Err(LegacyMigrationError::SourceNotFound(
            source.display().to_string(),
        ))
    }
}

pub(crate) async fn open_legacy(source: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(source)
        .create_if_missing(false)
        .read_only(true)
        .immutable(true)
        .foreign_keys(false);
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

pub(crate) fn sha256_file(path: &Path) -> Result<(u64, String)> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

async fn validate_required_schema(pool: &SqlitePool) -> Result<()> {
    let tables: BTreeSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    for (table, required_columns) in REQUIRED_SCHEMA {
        if !tables.contains(*table) {
            return Err(LegacyMigrationError::MissingTable((*table).to_owned()));
        }
        let statement = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
        let columns: BTreeSet<String> = sqlx::query(&statement)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
        let missing: Vec<String> = required_columns
            .iter()
            .filter(|column| !columns.contains(**column))
            .map(|column| (*column).to_owned())
            .collect();
        if !missing.is_empty() {
            return Err(LegacyMigrationError::IncompatibleSchema {
                table: (*table).to_owned(),
                missing,
            });
        }
    }
    Ok(())
}

pub(crate) async fn load_legacy_data(pool: &SqlitePool) -> Result<LegacyData> {
    let cages = sqlx::query_as::<_, LegacyCage>(
        r#"SELECT id, section, cage_id AS display_id, location, cage_type,
                  "order" AS sort_order, mice_birth_date, mice_count,
                  mice_sex, mice_genotype
           FROM cage ORDER BY id"#,
    )
    .fetch_all(pool)
    .await?;
    let mice = sqlx::query_as::<_, LegacyMouse>(
        r#"SELECT tid, id AS display_id, sex, live_status, birth_date, death_date,
                  cage_id, strain, tests_planned
           FROM mouse ORDER BY tid"#,
    )
    .fetch_all(pool)
    .await?;
    let loci = sqlx::query_as::<_, LegacyGeneLocus>(
        "SELECT id, symbol, description FROM gene_locus ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let alleles = sqlx::query_as::<_, LegacyAllele>(
        "SELECT id, symbol, locus_id, description, is_wildtype FROM allele ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let genotypes = sqlx::query_as::<_, LegacyGenotype>(
        "SELECT id, mouse_id, locus_id, allele1_id, allele2_id FROM genotype ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let pedigrees = sqlx::query_as::<_, LegacyPedigree>(
        "SELECT id, mouse_id, parent_id, parent_type FROM pedigree ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    Ok(LegacyData {
        cages,
        mice,
        loci,
        alleles,
        genotypes,
        pedigrees,
    })
}

async fn table_counts(pool: &SqlitePool) -> Result<BTreeMap<String, u64>> {
    let table_names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    let mut counts = BTreeMap::new();
    for table in table_names {
        let quoted = table.replace('"', "\"\"");
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{quoted}\""))
            .fetch_one(pool)
            .await?;
        counts.insert(table, count.max(0) as u64);
    }
    Ok(counts)
}

fn duplicate_identifiers(mice: &[LegacyMouse]) -> DuplicateIdentifierSummary {
    let mut by_display_id: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for mouse in mice {
        by_display_id
            .entry(mouse.display_id.as_str())
            .or_default()
            .push(mouse.tid);
    }
    let groups: Vec<DuplicateIdentifierGroup> = by_display_id
        .into_iter()
        .filter(|(_, tids)| tids.len() > 1)
        .map(|(display_id, legacy_tids)| DuplicateIdentifierGroup {
            display_id: display_id.to_owned(),
            row_count: legacy_tids.len() as u64,
            legacy_tids,
        })
        .collect();
    DuplicateIdentifierSummary {
        group_count: groups.len() as u64,
        row_count: groups.iter().map(|group| group.row_count).sum(),
        groups,
    }
}

fn cage_count_mismatches(cages: &[LegacyCage], mice: &[LegacyMouse]) -> Vec<CageCountMismatch> {
    let mut actual_counts: BTreeMap<i64, i64> = BTreeMap::new();
    for cage_id in mice.iter().filter_map(|mouse| mouse.cage_id) {
        *actual_counts.entry(cage_id).or_default() += 1;
    }
    cages
        .iter()
        .filter_map(|cage| {
            let actual_count = actual_counts.get(&cage.id).copied().unwrap_or_default();
            (cage.mice_count != Some(actual_count)).then(|| CageCountMismatch {
                legacy_cage_id: cage.id,
                section: cage.section.clone(),
                display_id: cage.display_id.clone(),
                cached_count: cage.mice_count,
                actual_count,
            })
        })
        .collect()
}

fn orphan_pedigree_links(
    pedigrees: &[LegacyPedigree],
    mice: &[LegacyMouse],
) -> Vec<RejectedPedigreeLink> {
    let mouse_ids: BTreeSet<i64> = mice.iter().map(|mouse| mouse.tid).collect();
    pedigrees
        .iter()
        .filter_map(|pedigree| {
            let mut reasons = Vec::new();
            match pedigree.mouse_id {
                None => reasons.push("missing_child_legacy_tid".to_owned()),
                Some(id) if !mouse_ids.contains(&id) => reasons.push("child_not_found".to_owned()),
                Some(_) => {}
            }
            match pedigree.parent_id {
                None => reasons.push("missing_parent_legacy_tid".to_owned()),
                Some(id) if !mouse_ids.contains(&id) => reasons.push("parent_not_found".to_owned()),
                Some(_) => {}
            }
            (!reasons.is_empty()).then(|| RejectedPedigreeLink {
                legacy_pedigree_id: pedigree.id,
                child_legacy_tid: pedigree.mouse_id,
                parent_legacy_tid: pedigree.parent_id,
                parent_type: pedigree.parent_type.clone(),
                reasons,
            })
        })
        .collect()
}

fn validate_data(data: &LegacyData, integrity_check: &str) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if integrity_check != "ok" {
        issues.push(issue(
            ValidationSeverity::Error,
            "sqlite_integrity_check_failed",
            "database",
            None,
            integrity_check,
        ));
    }

    let cage_ids: BTreeSet<i64> = data.cages.iter().map(|cage| cage.id).collect();
    let mouse_ids: BTreeSet<i64> = data.mice.iter().map(|mouse| mouse.tid).collect();
    let locus_ids: BTreeSet<i64> = data.loci.iter().map(|locus| locus.id).collect();
    let allele_by_id: BTreeMap<i64, &LegacyAllele> = data
        .alleles
        .iter()
        .map(|allele| (allele.id, allele))
        .collect();

    let mut cage_keys = BTreeSet::new();
    for cage in &data.cages {
        if cage.section.trim().is_empty() || cage.display_id.trim().is_empty() {
            issues.push(issue(
                ValidationSeverity::Error,
                "empty_cage_identifier",
                "cage",
                Some(cage.id.to_string()),
                "section and cage_id must both be non-empty",
            ));
        }
        if !cage_keys.insert((cage.section.clone(), cage.display_id.clone())) {
            issues.push(issue(
                ValidationSeverity::Error,
                "duplicate_cage_identifier",
                "cage",
                Some(cage.id.to_string()),
                "section + cage_id is not unique",
            ));
        }
        if let Some(date) = cage.mice_birth_date.as_deref()
            && NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        {
            issues.push(issue(
                ValidationSeverity::Warning,
                "invalid_cached_cage_birth_date",
                "cage",
                Some(cage.id.to_string()),
                date,
            ));
        }
        if !matches!(
            cage.cage_type.as_deref(),
            None | Some("normal" | "breeding" | "experimental")
        ) {
            issues.push(issue(
                ValidationSeverity::Warning,
                "unknown_cage_type",
                "cage",
                Some(cage.id.to_string()),
                cage.cage_type.as_deref().unwrap_or("NULL"),
            ));
        }
    }

    for mouse in &data.mice {
        if mouse.display_id.trim().is_empty() {
            issues.push(issue(
                ValidationSeverity::Error,
                "empty_mouse_identifier",
                "mouse",
                Some(mouse.tid.to_string()),
                "mouse.id is empty",
            ));
        }
        if let Some(cage_id) = mouse.cage_id
            && !cage_ids.contains(&cage_id)
        {
            issues.push(issue(
                ValidationSeverity::Error,
                "mouse_cage_not_found",
                "mouse",
                Some(mouse.tid.to_string()),
                &format!("cage {cage_id} does not exist"),
            ));
        }
        validate_mouse_date(
            &mut issues,
            mouse.tid,
            "birth_date",
            mouse.birth_date.as_deref(),
        );
        validate_mouse_date(
            &mut issues,
            mouse.tid,
            "death_date",
            mouse.death_date.as_deref(),
        );
        if !matches!(mouse.sex.as_deref(), Some("M" | "F") | None) {
            issues.push(issue(
                ValidationSeverity::Warning,
                "unknown_mouse_sex",
                "mouse",
                Some(mouse.tid.to_string()),
                mouse.sex.as_deref().unwrap_or("NULL"),
            ));
        }
        if !matches!(mouse.live_status, Some(0 | 1)) {
            issues.push(issue(
                ValidationSeverity::Warning,
                "unknown_live_status",
                "mouse",
                Some(mouse.tid.to_string()),
                &format!("{:?}", mouse.live_status),
            ));
        }
        if mouse.live_status == Some(1) && mouse.death_date.is_some() {
            issues.push(issue(
                ValidationSeverity::Warning,
                "living_mouse_has_death_date",
                "mouse",
                Some(mouse.tid.to_string()),
                "live_status is 1 but death_date is present",
            ));
        }
    }

    let mut locus_symbols = BTreeSet::new();
    for locus in &data.loci {
        if locus.symbol.trim().is_empty() || !locus_symbols.insert(locus.symbol.clone()) {
            issues.push(issue(
                ValidationSeverity::Error,
                "invalid_gene_locus_symbol",
                "gene_locus",
                Some(locus.id.to_string()),
                &locus.symbol,
            ));
        }
    }

    let mut allele_keys = BTreeSet::new();
    for allele in &data.alleles {
        if !locus_ids.contains(&allele.locus_id) {
            issues.push(issue(
                ValidationSeverity::Error,
                "allele_locus_not_found",
                "allele",
                Some(allele.id.to_string()),
                &format!("locus {} does not exist", allele.locus_id),
            ));
        }
        if allele.symbol.trim().is_empty()
            || !allele_keys.insert((allele.locus_id, allele.symbol.clone()))
        {
            issues.push(issue(
                ValidationSeverity::Error,
                "invalid_or_duplicate_allele_symbol",
                "allele",
                Some(allele.id.to_string()),
                &allele.symbol,
            ));
        }
    }

    let mut genotype_keys = BTreeSet::new();
    for genotype in &data.genotypes {
        if !mouse_ids.contains(&genotype.mouse_id) {
            issues.push(issue(
                ValidationSeverity::Error,
                "genotype_mouse_not_found",
                "genotype",
                Some(genotype.id.to_string()),
                &format!("mouse {} does not exist", genotype.mouse_id),
            ));
        }
        if !locus_ids.contains(&genotype.locus_id) {
            issues.push(issue(
                ValidationSeverity::Error,
                "genotype_locus_not_found",
                "genotype",
                Some(genotype.id.to_string()),
                &format!("locus {} does not exist", genotype.locus_id),
            ));
        }
        if !genotype_keys.insert((genotype.mouse_id, genotype.locus_id)) {
            issues.push(issue(
                ValidationSeverity::Error,
                "duplicate_genotype",
                "genotype",
                Some(genotype.id.to_string()),
                "multiple genotype rows exist for the same mouse and locus",
            ));
        }
        for allele_id in [genotype.allele1_id, genotype.allele2_id]
            .into_iter()
            .flatten()
        {
            match allele_by_id.get(&allele_id) {
                None => issues.push(issue(
                    ValidationSeverity::Error,
                    "genotype_allele_not_found",
                    "genotype",
                    Some(genotype.id.to_string()),
                    &format!("allele {allele_id} does not exist"),
                )),
                Some(allele) if allele.locus_id != genotype.locus_id => issues.push(issue(
                    ValidationSeverity::Error,
                    "genotype_allele_locus_mismatch",
                    "genotype",
                    Some(genotype.id.to_string()),
                    &format!(
                        "allele {allele_id} belongs to locus {}, expected {}",
                        allele.locus_id, genotype.locus_id
                    ),
                )),
                Some(_) => {}
            }
        }
    }

    for pedigree in &data.pedigrees {
        if pedigree.mouse_id.is_some() && pedigree.mouse_id == pedigree.parent_id {
            issues.push(issue(
                ValidationSeverity::Warning,
                "self_parent_link_rejected",
                "pedigree",
                Some(pedigree.id.to_string()),
                "child and parent point to the same mouse",
            ));
        }
        if !matches!(
            pedigree.parent_type.as_deref(),
            Some("father" | "mother") | None
        ) {
            issues.push(issue(
                ValidationSeverity::Warning,
                "unknown_parent_type",
                "pedigree",
                Some(pedigree.id.to_string()),
                pedigree.parent_type.as_deref().unwrap_or("NULL"),
            ));
        }
    }

    issues
}

async fn append_foreign_key_issues(
    pool: &SqlitePool,
    issues: &mut Vec<ValidationIssue>,
) -> Result<()> {
    for row in sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?
    {
        let table: String = row.try_get("table")?;
        let rowid: Option<i64> = row.try_get("rowid")?;
        let parent: String = row.try_get("parent")?;
        let severity = if table == "pedigree" {
            ValidationSeverity::Warning
        } else {
            ValidationSeverity::Error
        };
        issues.push(issue(
            severity,
            "legacy_foreign_key_violation",
            &table,
            rowid.map(|value| value.to_string()),
            &format!("referenced parent table {parent}"),
        ));
    }
    Ok(())
}

fn validate_mouse_date(
    issues: &mut Vec<ValidationIssue>,
    tid: i64,
    field: &str,
    value: Option<&str>,
) {
    if let Some(value) = value
        && NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err()
    {
        issues.push(issue(
            ValidationSeverity::Error,
            "invalid_mouse_date",
            "mouse",
            Some(tid.to_string()),
            &format!("{field}={value}"),
        ));
    }
}

fn issue(
    severity: ValidationSeverity,
    code: &str,
    entity: &str,
    legacy_id: Option<String>,
    message: &str,
) -> ValidationIssue {
    ValidationIssue {
        severity,
        code: code.to_owned(),
        entity: entity.to_owned(),
        legacy_id,
        message: message.to_owned(),
    }
}
