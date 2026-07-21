use std::path::{Path, PathBuf};

use muriarc_core::{
    AnimalFilter, AuditAction, AuditFilter, LOCAL_LAB_ID, MuriArcStore, WriteSource,
};
use muriarc_legacy_migrator::{LegacyMigrationError, audit_legacy, migrate_legacy};
use muriarc_store_sqlite::SqliteStore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tempfile::TempDir;

const FIXTURE: &str = include_str!("fixtures/minimal_legacy.sql");

async fn fixture_source(temp: &TempDir) -> PathBuf {
    let path = temp.path().join("legacy.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();
    pool.close().await;
    path
}

async fn scalar(path: &Path, statement: &str) -> i64 {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(true)
        .create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let value = sqlx::query_scalar(statement)
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    value
}

#[tokio::test]
async fn audit_reports_anomalies_without_changing_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture_source(&temp).await;
    let before = std::fs::read(&source).unwrap();

    let report = audit_legacy(&source).await.unwrap();

    assert_eq!(report.table_counts["mouse"], 3);
    assert_eq!(report.table_counts["cage"], 1);
    assert_eq!(report.table_counts["genotype"], 3);
    assert_eq!(report.table_counts["pedigree"], 2);
    assert_eq!(report.duplicate_identifiers.group_count, 1);
    assert_eq!(report.duplicate_identifiers.row_count, 2);
    assert_eq!(report.cage_count_mismatches.len(), 1);
    assert_eq!(report.cage_count_mismatches[0].cached_count, Some(9));
    assert_eq!(report.cage_count_mismatches[0].actual_count, 3);
    assert_eq!(report.orphan_pedigree_links.len(), 1);
    assert!(report.blocking_issues().is_empty());
    assert!(report.source.unchanged);
    assert_eq!(report.source.sha256_before, report.source.sha256_after);
    assert_eq!(before, std::fs::read(&source).unwrap());
}

#[tokio::test]
async fn migrate_preserves_rows_rejects_orphan_and_recalculates_cages() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture_source(&temp).await;
    let target = temp.path().join("muriarc.db");

    let report = migrate_legacy(&source, &target).await.unwrap();

    assert_eq!(report.migrated.animals, 3);
    assert_eq!(report.migrated.cages, 1);
    assert_eq!(report.migrated.genotypes, 3);
    assert_eq!(report.migrated.pedigrees, 1);
    assert_eq!(report.rejected_pedigree_links.len(), 1);
    assert_eq!(scalar(&target, "SELECT COUNT(*) FROM animals").await, 3);
    assert_eq!(scalar(&target, "SELECT COUNT(*) FROM genotypes").await, 3);
    assert_eq!(scalar(&target, "SELECT COUNT(*) FROM pedigrees").await, 1);
    assert_eq!(
        scalar(
            &target,
            "SELECT COUNT(*) FROM animals WHERE display_id = 'M1'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(
            &target,
            "SELECT COUNT(DISTINCT identifier_scope) FROM animals WHERE display_id = 'M1'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(
            &target,
            "SELECT COUNT(a.id) FROM cages c LEFT JOIN animals a ON a.current_cage_id = c.id"
        )
        .await,
        3
    );
    assert!(report.verification.source_hash_unchanged);
    assert!(report.verification.deterministic_ids_verified);
    assert!(report.verification.foreign_key_violations.is_empty());
    assert_eq!(report.verification.target_sha256.len(), 64);

    // The migrated target is immediately visible through the same Store path
    // used by DesktopState, without rewriting the reviewed source or target.
    let store = SqliteStore::connect_path(&target).await.unwrap();
    let animals = store
        .list_animals(&AnimalFilter {
            lab_id: LOCAL_LAB_ID,
            ..AnimalFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(animals.len(), 3);
    let audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    assert!(!audits.is_empty());
    assert!(audits.iter().all(|entry| {
        entry.action == AuditAction::Import && entry.source == WriteSource::Migration
    }));
}

#[tokio::test]
async fn deterministic_ids_are_stable_for_the_same_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture_source(&temp).await;
    let target_a = temp.path().join("a.db");
    let target_b = temp.path().join("b.db");
    migrate_legacy(&source, &target_a).await.unwrap();
    migrate_legacy(&source, &target_b).await.unwrap();

    async fn animal_ids(path: &Path) -> Vec<String> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .immutable(true)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let ids = sqlx::query_scalar("SELECT id FROM animals ORDER BY legacy_id")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;
        ids
    }

    assert_eq!(animal_ids(&target_a).await, animal_ids(&target_b).await);
}

#[tokio::test]
async fn existing_target_is_rejected_without_modification() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture_source(&temp).await;
    let target = temp.path().join("existing.db");
    std::fs::write(&target, b"do-not-replace").unwrap();

    let error = migrate_legacy(&source, &target).await.unwrap_err();

    assert!(matches!(error, LegacyMigrationError::TargetExists(_)));
    assert_eq!(std::fs::read(&target).unwrap(), b"do-not-replace");
}

#[tokio::test]
async fn incompatible_schema_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("invalid.db");
    let options = SqliteConnectOptions::new()
        .filename(&source)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE mouse (tid INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = audit_legacy(&source).await.unwrap_err();
    assert!(matches!(
        error,
        LegacyMigrationError::MissingTable(_) | LegacyMigrationError::IncompatibleSchema { .. }
    ));
}
