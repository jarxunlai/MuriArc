use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDigest {
    pub path: String,
    pub size_bytes: u64,
    pub sha256_before: String,
    pub sha256_after: String,
    pub unchanged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySchemaReport {
    pub format: String,
    pub compatible: bool,
    pub required_tables: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateIdentifierGroup {
    pub display_id: String,
    pub row_count: u64,
    pub legacy_tids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateIdentifierSummary {
    pub group_count: u64,
    pub row_count: u64,
    pub groups: Vec<DuplicateIdentifierGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CageCountMismatch {
    pub legacy_cage_id: i64,
    pub section: String,
    pub display_id: String,
    pub cached_count: Option<i64>,
    pub actual_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedPedigreeLink {
    pub legacy_pedigree_id: i64,
    pub child_legacy_tid: Option<i64>,
    pub parent_legacy_tid: Option<i64>,
    pub parent_type: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    pub entity: String,
    pub legacy_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAuditReport {
    pub tool_version: String,
    pub audited_at: DateTime<Utc>,
    pub source: SourceDigest,
    pub schema: LegacySchemaReport,
    pub integrity_check: String,
    pub table_counts: BTreeMap<String, u64>,
    pub duplicate_identifiers: DuplicateIdentifierSummary,
    pub cage_count_mismatches: Vec<CageCountMismatch>,
    pub orphan_pedigree_links: Vec<RejectedPedigreeLink>,
    pub validation_issues: Vec<ValidationIssue>,
}

impl LegacyAuditReport {
    pub fn blocking_issues(&self) -> Vec<ValidationIssue> {
        self.validation_issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityCounts {
    pub labs: u64,
    pub cages: u64,
    pub animals: u64,
    pub gene_loci: u64,
    pub alleles: u64,
    pub genotypes: u64,
    pub pedigrees: u64,
    pub audit_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetVerification {
    pub target_sha256: String,
    pub foreign_key_violations: Vec<String>,
    pub counts: EntityCounts,
    pub cage_actual_counts: BTreeMap<String, u64>,
    pub source_hash_unchanged: bool,
    pub deterministic_ids_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub tool_version: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub source_sha256_before: String,
    pub source_sha256_after: String,
    pub target_path: String,
    pub audit: LegacyAuditReport,
    pub migrated: EntityCounts,
    pub rejected_pedigree_links: Vec<RejectedPedigreeLink>,
    pub verification: TargetVerification,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyCage {
    pub id: i64,
    pub section: String,
    pub display_id: String,
    pub location: Option<String>,
    pub cage_type: Option<String>,
    pub sort_order: i64,
    pub mice_birth_date: Option<String>,
    pub mice_count: Option<i64>,
    pub mice_sex: Option<String>,
    pub mice_genotype: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyMouse {
    pub tid: i64,
    pub display_id: String,
    pub sex: Option<String>,
    pub live_status: Option<i64>,
    pub birth_date: Option<String>,
    pub death_date: Option<String>,
    pub cage_id: Option<i64>,
    pub strain: Option<String>,
    pub tests_planned: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyGeneLocus {
    pub id: i64,
    pub symbol: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyAllele {
    pub id: i64,
    pub symbol: String,
    pub locus_id: i64,
    pub description: Option<String>,
    pub is_wildtype: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyGenotype {
    pub id: i64,
    pub mouse_id: i64,
    pub locus_id: i64,
    pub allele1_id: Option<i64>,
    pub allele2_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyPedigree {
    pub id: i64,
    pub mouse_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub parent_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyData {
    pub cages: Vec<LegacyCage>,
    pub mice: Vec<LegacyMouse>,
    pub loci: Vec<LegacyGeneLocus>,
    pub alleles: Vec<LegacyAllele>,
    pub genotypes: Vec<LegacyGenotype>,
    pub pedigrees: Vec<LegacyPedigree>,
}
