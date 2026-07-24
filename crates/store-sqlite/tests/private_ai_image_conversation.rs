use chrono::{Duration, Utc};
use muriarc_core::{
    Actor, Attachment, AuditContext, MuriArcStore, PrivateAiImage, PrivateImageStatus, RecordMeta,
    StoreError, User, WorkspaceStore, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

fn image_fixture(
    lab_id: Uuid,
    user_id: Uuid,
    conversation_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> (Attachment, PrivateAiImage) {
    let image_id = Uuid::new_v4();
    let attachment = Attachment {
        id: Uuid::new_v4(),
        lab_id,
        project_id: None,
        entity_type: "ai_private_image".to_owned(),
        entity_id: image_id,
        file_name: "private-evidence.png".to_owned(),
        media_type: Some("image/png".to_owned()),
        relative_path: format!("ai-private/{image_id}.png"),
        size_bytes: 16,
        sha256: "a".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    let image = PrivateAiImage {
        id: image_id,
        lab_id,
        user_id,
        conversation_id: Some(conversation_id),
        attachment_id: attachment.id,
        project_id: None,
        status: PrivateImageStatus::Active,
        last_activity_at: now,
        expires_at: now + Duration::days(30),
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    (attachment, image)
}

#[tokio::test]
async fn sqlite_private_image_rejects_missing_foreign_and_legacy_conversations_atomically() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let bootstrap = AuditContext::system(WriteSource::Migration);
    let lab = muriarc_core::Lab::new("Private image conversation scope", now).unwrap();
    store.create_lab(&lab, &bootstrap).await.unwrap();
    let owner = User::new(
        lab.id,
        format!("{}@private-image.test", Uuid::new_v4()),
        "Private image owner",
        now,
    )
    .unwrap();
    let other = User::new(
        lab.id,
        format!("{}@private-image.test", Uuid::new_v4()),
        "Other conversation owner",
        now,
    )
    .unwrap();
    store.create_user(&owner, &bootstrap).await.unwrap();
    store.create_user(&other, &bootstrap).await.unwrap();
    let owner_legacy_id = Uuid::new_v4();
    let other_legacy_id = Uuid::new_v4();
    for (conversation_id, user_id) in [(owner_legacy_id, owner.id), (other_legacy_id, other.id)] {
        sqlx::query(
            "INSERT INTO ai_conversations (
                id, lab_id, project_id, user_id, title, model_profile_id,
                model_profile_version, legacy_read_only, created_at, updated_at,
                deleted_at, revision
             ) VALUES (?, ?, NULL, ?, 'Legacy conversation', NULL, NULL, 1, ?, ?, NULL, 1)",
        )
        .bind(conversation_id.to_string())
        .bind(lab.id.to_string())
        .bind(user_id.to_string())
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
    }
    let audit = AuditContext {
        actor: Actor::human(owner.id, owner.display_name.clone()),
        source: WriteSource::Web,
        request_id: Some("private-image-conversation-scope".to_owned()),
        reason: Some("verify atomic conversation validation".to_owned()),
    };
    let missing_id = Uuid::new_v4();
    let cases = [
        (owner_legacy_id, "legacy"),
        (missing_id, "missing"),
        (other_legacy_id, "foreign"),
    ];
    let mut entity_ids = Vec::new();
    for (conversation_id, label) in cases {
        let (attachment, image) = image_fixture(lab.id, owner.id, conversation_id, now);
        entity_ids.extend([attachment.id, image.id]);
        let error = store
            .create_private_ai_image(&attachment, &image, &audit)
            .await
            .expect_err(label);
        match label {
            "legacy" => assert!(matches!(error, StoreError::Conflict(_))),
            "missing" => assert!(matches!(error, StoreError::NotFound { .. })),
            "foreign" => assert!(matches!(error, StoreError::Validation(_))),
            _ => unreachable!(),
        }
    }

    let attachment_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE entity_type='ai_private_image'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    let image_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ai_private_images")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!((attachment_count, image_count), (0, 0));
    for entity_id in entity_ids {
        let audit_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_entries WHERE entity_id=?")
                .bind(entity_id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(audit_count, 0);
    }
}
