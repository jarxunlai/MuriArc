use std::borrow::Cow;

use muriarc_core::AiOperationStore;
use muriarc_core::store_contract::{
    run_ai_conversation_contract, run_ai_conversation_source_retention_contract,
    run_ai_experiment_grouping_contract, run_ai_import_commit_atomicity_contract,
    run_ai_measurement_approval_contract, run_import_source_archive_contract,
    run_research_extensions_contract, run_store_contract,
};
use muriarc_store_sqlite::SqliteStore;
use sqlx::migrate::Migrator;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

#[tokio::test]
async fn sqlite_store_obeys_shared_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_store_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_ai_conversation_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_conversation_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_enforces_ai_source_quota_and_retention_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_conversation_source_retention_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_atomically_archives_import_sources() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_import_source_archive_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_research_extensions_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_research_extensions_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_ai_experiment_grouping_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_experiment_grouping_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_preserves_ai_measurement_provider_provenance() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_measurement_approval_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_atomically_commits_ai_import_resolution() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_import_commit_atomicity_contract(&store).await;
}

#[tokio::test]
async fn ai_conversation_management_migration_preserves_legacy_rows() {
    let store = SqliteStore::in_memory().await.unwrap();
    let through_0022 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 22)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0022.run(store.pool()).await.unwrap();

    let lab_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision) VALUES (?, 'Legacy AI Lab', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', NULL, 1)",
    )
    .bind(lab_id.to_string())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, 'Legacy AI User', 'active', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', NULL, 1)",
    )
    .bind(user_id.to_string())
    .bind(lab_id.to_string())
    .bind(format!("legacy-{user_id}@example.test"))
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_conversations (id, lab_id, project_id, user_id, title, created_at, updated_at, deleted_at, revision) VALUES (?, ?, NULL, ?, 'Legacy conversation', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', NULL, 1)",
    )
    .bind(conversation_id.to_string())
    .bind(lab_id.to_string())
    .bind(user_id.to_string())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_conversation_messages (id, conversation_id, lab_id, project_id, user_id, sequence, role, content, response_json, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, NULL, ?, 1, 'user', 'Legacy question', NULL, '2026-07-01T00:00:01Z', '2026-07-01T00:00:01Z', NULL, 1)",
    )
    .bind(message_id.to_string())
    .bind(conversation_id.to_string())
    .bind(lab_id.to_string())
    .bind(user_id.to_string())
    .execute(store.pool())
    .await
    .unwrap();

    MIGRATOR.run(store.pool()).await.unwrap();
    MIGRATOR.run(store.pool()).await.unwrap();
    let saved = store.get_ai_conversation(conversation_id).await.unwrap();
    assert_eq!(saved.title, "Legacy conversation");
    assert_eq!(saved.pinned_at, None);
    assert_eq!(saved.archived_at, None);
    assert_eq!(saved.meta.revision, 1);
    let messages = store
        .list_ai_conversation_messages(conversation_id, 20)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message_id);
    assert!(messages[0].source_refs.is_empty());
}
