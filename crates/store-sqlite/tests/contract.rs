use muriarc_core::store_contract::{
    run_ai_conversation_contract, run_ai_model_profile_contract, run_research_extensions_contract,
    run_store_contract,
};
use muriarc_core::{
    Actor, AiExtractionApprovalInput, AiExtractionApprovalSelection, AiExtractionRejectionInput,
    AiExtractionStatus, AuditContext, MuriArcStore, ObservationValueData, PrivateImageStatus,
    StoreError, WorkspaceStore, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use sqlx::Row;

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
async fn sqlite_store_obeys_research_extensions_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_research_extensions_contract(&store).await;
    assert_corrupted_second_evidence_rolls_back(&store, "contract-atomic-approval", true).await;
    assert_corrupted_second_evidence_rolls_back(&store, "contract-atomic-rejection", false).await;
}
