use chrono::{Duration, Utc};
use muriarc_core::{
    Actor, ActorType, AiModelCredentialState, AiModelProfile, AiModelProfileFilter,
    AiModelProfileSecretRef, AiModelProfileSecretRefStore, AiModelProfileStore,
    AiModelProfileVersion, AiProviderProtocol, AiProviderTransport, AiUserModelDefaults,
    AuditAction, AuditContext, AuditFilter, EntityType, Lab, MuriArcStore, RecordMeta, StoreError,
    User, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

fn version(profile_id: Uuid, number: i64, model_id: &str) -> AiModelProfileVersion {
    AiModelProfileVersion {
        profile_id,
        version: number,
        protocol: AiProviderProtocol::OpenaiChatCompletions,
        transport: AiProviderTransport::OpenAiCompatible,
        base_url: "https://provider.example.test/v1".to_owned(),
        normalized_base_url: "https://provider.example.test/v1".to_owned(),
        model_id: model_id.to_owned(),
        supports_vision: false,
        context_window_tokens: 131_072,
        max_input_tokens: 65_536,
        max_output_tokens: 4_096,
        history_token_budget: 32_768,
        history_turns: 20,
        temperature: 0.0,
        timeout_ms: 120_000,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn profile_versions_are_immutable_and_defaults_are_revision_checked() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("AI model profile contract", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let user = User::new(lab.id, "models@example.test", "Model Owner", now).unwrap();
    store.create_user(&user, &audit).await.unwrap();

    let mut profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        name: "Primary model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    let first = version(profile.id, 1, "model-v1");
    store
        .create_ai_model_profile(&profile, &first, &audit)
        .await
        .unwrap();

    profile.current_version = 2;
    profile.meta.touch(Utc::now());
    let second = version(profile.id, 2, "model-v2");
    store
        .append_ai_model_profile_version(&profile, &second, 1, &audit)
        .await
        .unwrap();

    assert_eq!(
        store
            .get_ai_model_profile_version(profile.id, 1)
            .await
            .unwrap()
            .model_id,
        "model-v1"
    );
    assert_eq!(
        store
            .get_ai_model_profile_version(profile.id, 2)
            .await
            .unwrap()
            .model_id,
        "model-v2"
    );
    assert!(
        store
            .append_ai_model_profile_version(&profile, &second, 1, &audit)
            .await
            .is_err()
    );

    let defaults = AiUserModelDefaults {
        user_id: user.id,
        default_conversation_profile_id: Some(profile.id),
        default_vision_profile_id: None,
        meta: RecordMeta::new(now),
    };
    store
        .save_ai_user_model_defaults(&defaults, None, &audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_ai_user_model_defaults(user.id)
            .await
            .unwrap()
            .unwrap()
            .default_conversation_profile_id,
        Some(profile.id)
    );

    let listed = store
        .list_ai_model_profiles(&AiModelProfileFilter {
            lab_id: lab.id,
            user_id: user.id,
            include_archived: false,
        })
        .await
        .unwrap();
    assert_eq!(listed, vec![profile]);
}

#[tokio::test]
async fn soft_deleted_profile_owners_cannot_mutate_profiles_or_defaults() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("Deleted AI model profile owner", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let user = User::new(
        lab.id,
        "deleted-model-owner@example.test",
        "Deleted Model Owner",
        now,
    )
    .unwrap();
    store.create_user(&user, &audit).await.unwrap();

    let profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        name: "Orphaned model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    let first = version(profile.id, 1, "model-v1");
    store
        .create_ai_model_profile(&profile, &first, &audit)
        .await
        .unwrap();

    sqlx::query("UPDATE users SET deleted_at = ? WHERE id = ?")
        .bind(now)
        .bind(user.id.to_string())
        .execute(store.pool())
        .await
        .expect("test fixture must soft-delete the profile owner");

    let mut next_profile = profile.clone();
    next_profile.current_version = 2;
    next_profile.meta.touch(now);
    let second = version(profile.id, 2, "model-v2");
    assert!(matches!(
        store
            .append_ai_model_profile_version(&next_profile, &second, 1, &audit)
            .await,
        Err(StoreError::NotFound { entity: "user", id }) if id == user.id
    ));

    let mut archived_profile = profile.clone();
    archived_profile.archived_at = Some(now);
    archived_profile.meta.touch(now);
    assert!(matches!(
        store
            .archive_ai_model_profile(&archived_profile, 1, &audit)
            .await,
        Err(StoreError::NotFound { entity: "user", id }) if id == user.id
    ));

    let defaults = AiUserModelDefaults {
        user_id: user.id,
        default_conversation_profile_id: Some(profile.id),
        default_vision_profile_id: None,
        meta: RecordMeta::new(now),
    };
    assert!(matches!(
        store
            .save_ai_user_model_defaults(&defaults, None, &audit)
            .await,
        Err(StoreError::NotFound { entity: "user", id }) if id == user.id
    ));

    assert_eq!(
        store.get_ai_model_profile(profile.id).await.unwrap(),
        profile
    );
    assert!(matches!(
        store.get_ai_model_profile_version(profile.id, 2).await,
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(
        store.get_ai_user_model_defaults(user.id).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn desktop_secret_refs_are_exact_versioned_revision_checked_and_redacted_in_audit() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let migration_audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("Desktop AI secret reference", now).unwrap();
    store.create_lab(&lab, &migration_audit).await.unwrap();
    let user = User::new(
        lab.id,
        "desktop-secret-owner@example.test",
        "Desktop Secret Owner",
        now,
    )
    .unwrap();
    store.create_user(&user, &migration_audit).await.unwrap();
    let profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        name: "Desktop model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_model_profile(
            &profile,
            &version(profile.id, 1, "desktop-model"),
            &migration_audit,
        )
        .await
        .unwrap();

    let desktop_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Desktop,
        request_id: Some("desktop-secret-ref-test".to_owned()),
        reason: Some("rotate_desktop_model_credential".to_owned()),
    };
    let keyring_account = format!("local-user-model-profile-{}-v1-api-key", profile.id);
    let mut reference = AiModelProfileSecretRef {
        profile_id: profile.id,
        profile_version: 1,
        keyring_account,
        credential_state: AiModelCredentialState::Present,
        created_at: now,
        updated_at: now,
        revision: 1,
    };
    store
        .save_ai_model_profile_secret_ref(&reference, None, &desktop_audit)
        .await
        .unwrap();

    reference.updated_at = now + Duration::seconds(1);
    reference.revision = 2;
    store
        .save_ai_model_profile_secret_ref(&reference, Some(1), &desktop_audit)
        .await
        .unwrap();

    reference.credential_state = AiModelCredentialState::Revoked;
    reference.updated_at = now + Duration::seconds(2);
    reference.revision = 3;
    store
        .save_ai_model_profile_secret_ref(&reference, Some(2), &desktop_audit)
        .await
        .unwrap();

    assert_eq!(
        store
            .get_ai_model_profile_secret_ref(profile.id, 1)
            .await
            .unwrap(),
        Some(reference.clone())
    );
    assert_eq!(
        store
            .list_ai_model_profile_secret_refs(profile.id)
            .await
            .unwrap(),
        vec![reference.clone()]
    );
    assert!(matches!(
        store
            .save_ai_model_profile_secret_ref(&reference, Some(2), &desktop_audit)
            .await,
        Err(StoreError::Conflict(_))
    ));

    let secret_ref_audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: Some(profile.id),
        })
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| {
            entry.entity_type == EntityType::AiModelProfile
                && entry
                    .after
                    .as_ref()
                    .and_then(|after| after.get("keyring_account"))
                    .is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(secret_ref_audits.len(), 3);
    assert!(secret_ref_audits.iter().all(|entry| {
        entry.source == WriteSource::Desktop
            && entry.actor.actor_type == ActorType::Human
            && entry.actor.user_id == Some(user.id)
    }));
    let actions = secret_ref_audits
        .iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    assert!(actions.contains(&AuditAction::Create));
    assert!(actions.contains(&AuditAction::Update));
    assert!(actions.contains(&AuditAction::Revoke));
    let serialized = serde_json::to_string(&secret_ref_audits).unwrap();
    assert!(!serialized.contains("credential-value-must-never-be-audited"));
    assert!(!serialized.contains("\"api_key\""));
}

#[tokio::test]
async fn desktop_secret_refs_are_batch_revoked_atomically_with_redacted_audits() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let migration_audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("Desktop AI batch secret revocation", now).unwrap();
    store.create_lab(&lab, &migration_audit).await.unwrap();
    let user = User::new(
        lab.id,
        "desktop-batch-secret-owner@example.test",
        "Desktop Batch Secret Owner",
        now,
    )
    .unwrap();
    store.create_user(&user, &migration_audit).await.unwrap();
    let mut profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        name: "Desktop batch model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_model_profile(
            &profile,
            &version(profile.id, 1, "desktop-batch-v1"),
            &migration_audit,
        )
        .await
        .unwrap();
    profile.current_version = 2;
    profile.meta.touch(now + Duration::seconds(1));
    store
        .append_ai_model_profile_version(
            &profile,
            &version(profile.id, 2, "desktop-batch-v2"),
            1,
            &migration_audit,
        )
        .await
        .unwrap();

    let desktop_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Desktop,
        request_id: Some("desktop-batch-secret-revoke-test".to_owned()),
        reason: Some("clear_desktop_model_credentials".to_owned()),
    };
    for profile_version in 1..=2 {
        store
            .save_ai_model_profile_secret_ref(
                &AiModelProfileSecretRef {
                    profile_id: profile.id,
                    profile_version,
                    keyring_account: format!(
                        "local-user-model-profile-{}-v{profile_version}-api-key",
                        profile.id
                    ),
                    credential_state: AiModelCredentialState::Present,
                    created_at: now,
                    updated_at: now,
                    revision: 1,
                },
                None,
                &desktop_audit,
            )
            .await
            .unwrap();
    }

    let revoked_at = now + Duration::seconds(2);
    let revoked = store
        .revoke_ai_model_profile_secret_refs(profile.id, revoked_at, &desktop_audit)
        .await
        .unwrap();
    assert_eq!(revoked.len(), 2);
    assert!(revoked.iter().all(|reference| {
        reference.credential_state == AiModelCredentialState::Revoked
            && reference.updated_at == revoked_at
            && reference.revision == 2
    }));
    assert_eq!(
        store
            .revoke_ai_model_profile_secret_refs(
                profile.id,
                revoked_at + Duration::seconds(1),
                &desktop_audit,
            )
            .await
            .unwrap(),
        revoked,
        "retrying an already revoked batch must be idempotent"
    );

    for mut reference in revoked {
        reference.credential_state = AiModelCredentialState::Present;
        reference.updated_at = now + Duration::seconds(4);
        reference.revision = 3;
        store
            .save_ai_model_profile_secret_ref(&reference, Some(2), &desktop_audit)
            .await
            .unwrap();
    }
    sqlx::query(
        "UPDATE ai_model_profile_secret_refs
         SET revision = ?
         WHERE profile_id = ? AND profile_version = 2",
    )
    .bind(i64::MAX)
    .bind(profile.id.to_string())
    .execute(store.pool())
    .await
    .unwrap();

    assert!(matches!(
        store
            .revoke_ai_model_profile_secret_refs(
                profile.id,
                now + Duration::seconds(5),
                &desktop_audit,
            )
            .await,
        Err(StoreError::Validation(_))
    ));
    let after_failed_batch = store
        .list_ai_model_profile_secret_refs(profile.id)
        .await
        .unwrap();
    assert_eq!(
        after_failed_batch[0].credential_state,
        AiModelCredentialState::Present,
        "an error in a later reference must roll back earlier revocations"
    );
    assert_eq!(after_failed_batch[0].revision, 3);
    assert_eq!(
        after_failed_batch[1].credential_state,
        AiModelCredentialState::Present
    );
    assert_eq!(after_failed_batch[1].revision, i64::MAX);

    let revoke_audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: Some(profile.id),
        })
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| {
            entry.entity_type == EntityType::AiModelProfile
                && entry.action == AuditAction::Revoke
                && entry
                    .after
                    .as_ref()
                    .and_then(|after| after.get("keyring_account"))
                    .is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        revoke_audits.len(),
        2,
        "the failed batch must roll back its partial audit entries"
    );
    let serialized = serde_json::to_string(&revoke_audits).unwrap();
    assert!(!serialized.contains("\"api_key\""));
}
