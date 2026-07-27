use std::borrow::Cow;

use muriarc_core::store_contract::{
    run_ai_conversation_contract, run_ai_conversation_source_retention_contract,
    run_ai_experiment_grouping_contract, run_ai_import_commit_atomicity_contract,
    run_ai_measurement_approval_contract, run_ai_model_profile_contract,
    run_genotyping_batch_contract, run_import_source_archive_contract,
    run_research_extensions_contract, run_store_contract,
};
use muriarc_core::{
    Actor, AiExtractionApprovalInput, AiExtractionApprovalSelection, AiExtractionRejectionInput,
    AiExtractionStatus, AiOperationStore, AuditContext, MuriArcStore, ObservationValueData,
    PrivateImageStatus, StoreError, WorkspaceStore, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use sqlx::{Row, migrate::Migrator};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

fn migration_prefix(max_version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= max_version)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn assert_corrupted_second_evidence_rolls_back(
    store: &SqliteStore,
    provider: &str,
    approval: bool,
) {
    let draft_id: String = sqlx::query_scalar(
        "SELECT id FROM ai_extraction_drafts
         WHERE provider=? AND status='pending_approval'
         ORDER BY created_at DESC,id DESC LIMIT 1",
    )
    .bind(provider)
    .fetch_one(store.pool())
    .await
    .unwrap();
    let draft_id = uuid::Uuid::parse_str(&draft_id).unwrap();
    let draft = store.get_ai_extraction_draft(draft_id).await.unwrap();
    assert_eq!(draft.evidence.len(), 2);
    let first = &draft.evidence[0];
    let second = &draft.evidence[1];
    sqlx::query(
        "UPDATE ai_private_images
         SET status='active',updated_at=datetime(updated_at,'+1 second'),revision=revision+1
         WHERE id=?",
    )
    .bind(second.private_image_id.to_string())
    .execute(store.pool())
    .await
    .unwrap();

    let before_counts: (i64, i64, i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM observations")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM attachment_links")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM audit_entries")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM provenance")
            .fetch_one(store.pool())
            .await
            .unwrap(),
    );
    let first_image_before =
        sqlx::query("SELECT project_id,status,revision FROM ai_private_images WHERE id=?")
            .bind(first.private_image_id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    let first_attachment_before =
        sqlx::query("SELECT project_id,entity_type,entity_id,revision FROM attachments WHERE id=?")
            .bind(first.private_attachment_id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();

    let audit = AuditContext {
        actor: Actor::human(draft.user_id, "Atomicity owner"),
        source: WriteSource::Web,
        request_id: Some(format!("{provider}-atomicity")),
        reason: Some("second evidence corruption must roll back".to_owned()),
    };
    let result = if approval {
        store
            .apply_ai_extraction_draft(
                draft.id,
                &AiExtractionApprovalInput {
                    expected_revision: draft.meta.revision,
                    selections: vec![AiExtractionApprovalSelection {
                        item_index: 0,
                        value: ObservationValueData::Number(9.0),
                        notes: Some("must roll back".to_owned()),
                    }],
                },
                &audit,
            )
            .await
            .map(|_| ())
    } else {
        store
            .reject_ai_extraction_draft(
                draft.id,
                &AiExtractionRejectionInput {
                    expected_revision: draft.meta.revision,
                },
                &audit,
            )
            .await
            .map(|_| ())
    };
    assert!(matches!(result, Err(StoreError::Conflict(_))));

    let after_counts: (i64, i64, i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM observations")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM attachment_links")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM audit_entries")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM provenance")
            .fetch_one(store.pool())
            .await
            .unwrap(),
    );
    assert_eq!(after_counts, before_counts);
    assert!(matches!(
        store.get_observation(draft.items[0].observation.id).await,
        Err(StoreError::NotFound { .. })
    ));
    let first_image_after =
        sqlx::query("SELECT project_id,status,revision FROM ai_private_images WHERE id=?")
            .bind(first.private_image_id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        first_image_after.get::<Option<String>, _>("project_id"),
        first_image_before.get::<Option<String>, _>("project_id")
    );
    assert_eq!(
        first_image_after.get::<String, _>("status"),
        first_image_before.get::<String, _>("status")
    );
    assert_eq!(
        first_image_after.get::<i64, _>("revision"),
        first_image_before.get::<i64, _>("revision")
    );
    assert_eq!(
        first_image_after.get::<String, _>("status"),
        "pending_approval"
    );
    let first_attachment_after =
        sqlx::query("SELECT project_id,entity_type,entity_id,revision FROM attachments WHERE id=?")
            .bind(first.private_attachment_id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    for column in ["project_id", "entity_type", "entity_id"] {
        assert_eq!(
            first_attachment_after.get::<Option<String>, _>(column),
            first_attachment_before.get::<Option<String>, _>(column)
        );
    }
    assert_eq!(
        first_attachment_after.get::<i64, _>("revision"),
        first_attachment_before.get::<i64, _>("revision")
    );
    let persisted = store.get_ai_extraction_draft(draft.id).await.unwrap();
    assert_eq!(persisted.status, AiExtractionStatus::PendingApproval);
    assert_eq!(persisted.meta.revision, draft.meta.revision);
    assert!(
        persisted.evidence.iter().all(
            |evidence| evidence.promoted_attachment_id.is_none() && evidence.meta.revision == 1
        )
    );
    assert_eq!(
        store
            .get_private_ai_image(first.private_image_id)
            .await
            .unwrap()
            .status,
        PrivateImageStatus::PendingApproval
    );
}

#[tokio::test]
async fn sqlite_store_obeys_shared_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_store_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_ai_model_profile_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_model_profile_contract(&store).await;
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
    assert_corrupted_second_evidence_rolls_back(&store, "contract-atomic-approval", true).await;
    assert_corrupted_second_evidence_rolls_back(&store, "contract-atomic-rejection", false).await;
}

#[tokio::test]
async fn sqlite_store_obeys_genotyping_batch_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_genotyping_batch_contract(&store).await;
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
async fn ai_conversation_management_migration_preserves_legacy_rows_across_direct_and_staged_upgrades()
 {
    for stage_model_platform_stack in [false, true] {
        let store = SqliteStore::in_memory().await.unwrap();
        migration_prefix(22).run(store.pool()).await.unwrap();

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

        if stage_model_platform_stack {
            migration_prefix(25).run(store.pool()).await.unwrap();
            let stack_ledger: (i64, Option<i64>) =
                sqlx::query_as("SELECT count(*), max(version) FROM _sqlx_migrations")
                    .fetch_one(store.pool())
                    .await
                    .unwrap();
            assert_eq!(
                stack_ledger,
                (24, Some(25)),
                "the #12-#16 SQLite stack must end at 0025 (version 0005 is intentionally absent)"
            );
            let stack_conversation: (Option<String>, Option<i64>, i64, i64) = sqlx::query_as(
                "SELECT model_profile_id, model_profile_version,
                        legacy_read_only, revision
                 FROM ai_conversations WHERE id = ?",
            )
            .bind(conversation_id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
            assert_eq!(
                stack_conversation,
                (None, None, 1, 1),
                "the model-platform stack must preserve legacy history as read-only"
            );
        }

        MIGRATOR.run(store.pool()).await.unwrap();
        MIGRATOR.run(store.pool()).await.unwrap();
        let final_ledger: (i64, Option<i64>) =
            sqlx::query_as("SELECT count(*), max(version) FROM _sqlx_migrations")
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(
            final_ledger,
            (30, Some(31)),
            "the merged SQLite migration set must end at 0031 (version 0005 is intentionally absent)"
        );
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
        let source_tables: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM sqlite_master
             WHERE type = 'table'
               AND name IN (
                   'ai_conversation_sources',
                   'ai_conversation_source_object_deletions'
               )",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(source_tables, 2);
    }
}
