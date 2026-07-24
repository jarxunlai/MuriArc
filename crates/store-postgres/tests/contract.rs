use chrono::Utc;
use muriarc_core::{
    Actor, ActorType, AiActionCategory, AiAutonomyGrant, AiAutonomyMode, AiConversationMessage,
    AiConversationMessageRole, AiOperationStore, Animal, Approval, ApprovalDecision, AuditAction,
    AuditContext, AuditFilter, EntityType, Lab, Measurement, MeasurementValue, MuriArcStore,
    Project, RecordMeta, Sex, StoreError, ToolRun, ToolRunStatus, User, WriteSource,
    store_contract::{
        run_ai_conversation_contract, run_ai_model_profile_contract,
        run_research_extensions_contract, run_store_contract,
    },
};
use muriarc_store_postgres::PostgresStore;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn postgres_store_obeys_shared_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!("skipping PostgreSQL contract: MURIARC_TEST_DATABASE_URL is not set");
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_store_contract(&store).await;
}

#[tokio::test]
async fn postgres_store_obeys_ai_model_profile_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL AI model profile contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_ai_model_profile_contract(&store).await;
}

#[tokio::test]
async fn postgres_store_obeys_ai_conversation_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL AI conversation contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_ai_conversation_contract(&store).await;
}

#[tokio::test]
async fn postgres_legacy_conversations_reject_messages_and_autonomy_changes_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!("skipping PostgreSQL legacy AI read-only contract: database is not configured");
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let bootstrap = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("PostgreSQL legacy AI read-only contract", now).unwrap();
    store.create_lab(&lab, &bootstrap).await.unwrap();
    let user = User::new(
        lab.id,
        format!("{}@legacy-ai.example.test", Uuid::new_v4()),
        "Legacy AI researcher",
        now,
    )
    .unwrap();
    store.create_user(&user, &bootstrap).await.unwrap();
    let ungranted_id = Uuid::new_v4();
    let granted_id = Uuid::new_v4();
    for (id, title) in [
        (ungranted_id, "Legacy conversation without grant"),
        (granted_id, "Legacy conversation with grant"),
    ] {
        sqlx::query(
            "INSERT INTO ai_conversations (
                id, lab_id, project_id, user_id, title, model_profile_id,
                model_profile_version, legacy_read_only, created_at, updated_at,
                deleted_at, revision
             ) VALUES ($1, $2, NULL, $3, $4, NULL, NULL, TRUE, $5, $5, NULL, 1)",
        )
        .bind(id)
        .bind(lab.id)
        .bind(user.id)
        .bind(title)
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
    }
    let existing_grant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai_autonomy_grants (
            id, conversation_id, lab_id, project_id, user_id, session_id, mode,
            allowed_categories_json, batch_limit, step_up_verified_at, last_used_at,
            expires_at, revoked_at, created_at, updated_at, deleted_at, revision
         ) VALUES ($1, $2, $3, NULL, $4, NULL, 'ask', $5, 1, NULL, $6, NULL, NULL, $6, $6, NULL, 1)",
    )
    .bind(existing_grant_id)
    .bind(granted_id)
    .bind(lab.id)
    .bind(user.id)
    .bind(serde_json::to_value(vec![AiActionCategory::Read]).unwrap())
    .bind(now)
    .execute(store.pool())
    .await
    .unwrap();
    let audit = AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(user.id),
            display_name: "MuriArc AI for legacy researcher".to_owned(),
        },
        source: WriteSource::Ai,
        request_id: Some("postgres-legacy-write-rejection".to_owned()),
        reason: Some("legacy read-only contract".to_owned()),
    };
    let user_message = AiConversationMessage::new(
        ungranted_id,
        lab.id,
        None,
        user.id,
        1,
        AiConversationMessageRole::User,
        "Must not be saved",
        None,
        now,
    )
    .unwrap();
    let assistant_message = AiConversationMessage::new(
        ungranted_id,
        lab.id,
        None,
        user.id,
        2,
        AiConversationMessageRole::Assistant,
        "Must not be saved",
        Some(json!({"content": "Must not be saved"})),
        now,
    )
    .unwrap();
    assert!(matches!(
        store
            .append_ai_turn_messages(&user_message, &assistant_message, 0, &audit)
            .await,
        Err(StoreError::Conflict(_))
    ));

    let new_grant = AiAutonomyGrant {
        id: Uuid::new_v4(),
        conversation_id: ungranted_id,
        lab_id: lab.id,
        project_id: None,
        user_id: user.id,
        session_id: None,
        mode: AiAutonomyMode::Ask,
        allowed_categories: vec![AiActionCategory::Read],
        batch_limit: 1,
        step_up_verified_at: None,
        last_used_at: now,
        expires_at: None,
        revoked_at: None,
        meta: RecordMeta::new(now),
    };
    assert!(matches!(
        store.save_ai_autonomy_grant(&new_grant, None, &audit).await,
        Err(StoreError::Conflict(_))
    ));

    let mut revoke = store
        .get_ai_autonomy_grant(granted_id)
        .await
        .unwrap()
        .unwrap();
    let expected_revision = revoke.meta.revision;
    revoke.revoked_at = Some(now + chrono::Duration::seconds(1));
    revoke.meta.touch(now + chrono::Duration::seconds(1));
    assert!(matches!(
        store
            .save_ai_autonomy_grant(&revoke, Some(expected_revision), &audit)
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .get_ai_autonomy_grant(granted_id)
            .await
            .unwrap()
            .unwrap()
            .revoked_at,
        None
    );

    let project = Project::new(lab.id, "Legacy PostgreSQL draft project", now).unwrap();
    store.create_project(&project, &bootstrap).await.unwrap();
    let animal = Animal::new_mouse(lab.id, "LEGACY-PG-DRAFT-1", Sex::Female, now).unwrap();
    store.create_animal(&animal, &bootstrap).await.unwrap();
    let draft_conversation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, legacy_read_only, created_at, updated_at,
            deleted_at, revision
         ) VALUES ($1, $2, $3, $4, 'Legacy conversation with pending draft',
            NULL, NULL, TRUE, $5, $5, NULL, 1)",
    )
    .bind(draft_conversation_id)
    .bind(lab.id)
    .bind(project.id)
    .bind(user.id)
    .bind(now)
    .execute(store.pool())
    .await
    .unwrap();
    let legacy_tool = ToolRun {
        id: Uuid::new_v4(),
        conversation_id: Some(draft_conversation_id),
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: user.id,
        tool_name: "mutation_draft".to_owned(),
        input: json!({"operation": "record_measurement"}),
        output: Some(json!({"draft": "legacy"})),
        status: ToolRunStatus::AwaitingApproval,
        source: WriteSource::Ai,
        started_at: Some(now),
        completed_at: None,
        error: None,
        meta: RecordMeta::new(now),
    };
    assert!(matches!(
        store.create_tool_run(&legacy_tool, &audit).await,
        Err(StoreError::Conflict(_))
    ));
    sqlx::query(
        "INSERT INTO ai_tool_runs (
            id, conversation_id, lab_id, project_id, user_id, tool_name,
            input_json, output_json, status, source, started_at, completed_at,
            error, created_at, updated_at, deleted_at, revision
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'awaiting_approval','ai',$9,
            NULL,NULL,$9,$9,NULL,1)",
    )
    .bind(legacy_tool.id)
    .bind(draft_conversation_id)
    .bind(lab.id)
    .bind(project.id)
    .bind(user.id)
    .bind(&legacy_tool.tool_name)
    .bind(&legacy_tool.input)
    .bind(legacy_tool.output.as_ref().unwrap())
    .bind(now)
    .execute(store.pool())
    .await
    .unwrap();
    let legacy_approval = Approval {
        id: Uuid::new_v4(),
        tool_run_id: legacy_tool.id,
        requested_diff: json!({"draft": "legacy"}),
        decision: ApprovalDecision::Pending,
        decided_by: None,
        decided_at: None,
        reason: None,
        meta: RecordMeta::new(now),
    };
    assert!(matches!(
        store.create_approval(&legacy_approval, &audit).await,
        Err(StoreError::Conflict(_))
    ));
    sqlx::query(
        "INSERT INTO ai_approvals (
            id, tool_run_id, requested_diff_json, decision, decided_by,
            decided_at, reason, created_at, updated_at, deleted_at, revision
         ) VALUES ($1,$2,$3,'pending',NULL,NULL,NULL,$4,$4,NULL,1)",
    )
    .bind(legacy_approval.id)
    .bind(legacy_tool.id)
    .bind(&legacy_approval.requested_diff)
    .bind(now)
    .execute(store.pool())
    .await
    .unwrap();

    let decided_at = now + chrono::Duration::seconds(1);
    let mut resolved_tool = legacy_tool.clone();
    resolved_tool.status = ToolRunStatus::Completed;
    resolved_tool.completed_at = Some(decided_at);
    resolved_tool.meta.touch(decided_at);
    let mut resolved_approval = legacy_approval.clone();
    resolved_approval.decision = ApprovalDecision::Approved;
    resolved_approval.decided_by = Some(user.id);
    resolved_approval.decided_at = Some(decided_at);
    resolved_approval.meta.touch(decided_at);
    let human_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Api,
        request_id: Some("postgres-legacy-draft-decision".to_owned()),
        reason: Some("must remain read-only".to_owned()),
    };
    assert!(matches!(
        store
            .finalize_ai_draft(&resolved_tool, 1, &resolved_approval, 1, &human_audit,)
            .await,
        Err(StoreError::Conflict(_))
    ));
    let measurement = Measurement::draft(
        lab.id,
        project.id,
        animal.id,
        "body_weight",
        "Body weight",
        MeasurementValue::Number(20.0),
        now,
        decided_at,
    )
    .unwrap();
    assert!(matches!(
        store
            .apply_ai_measurement_draft(
                &measurement,
                store.get_animal(animal.id).await.unwrap().meta.revision,
                &resolved_tool,
                1,
                &resolved_approval,
                1,
                &human_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.get_measurement(measurement.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(
        store.get_tool_run(legacy_tool.id).await.unwrap().status,
        ToolRunStatus::AwaitingApproval
    );
    assert_eq!(
        store
            .get_approval(legacy_approval.id)
            .await
            .unwrap()
            .decision,
        ApprovalDecision::Pending
    );
    for entity_id in [legacy_tool.id, legacy_approval.id, measurement.id] {
        assert!(
            store
                .list_audit_entries(&AuditFilter {
                    lab_id: lab.id,
                    project_id: None,
                    entity_id: Some(entity_id),
                })
                .await
                .unwrap()
                .is_empty(),
            "rejected legacy draft write must not create an audit entry"
        );
    }
}

#[tokio::test]
async fn postgres_store_obeys_research_extensions_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL research extensions contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_research_extensions_contract(&store).await;
}

#[tokio::test]
async fn postgres_audit_reader_accepts_security_lifecycle_entries_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL audit compatibility contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    store.migrate().await.unwrap();
    let audit = AuditContext::system(WriteSource::Migration);
    let now = Utc::now();
    let lab = Lab::new("Audit Compatibility Contract", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();

    let fixtures = [
        (
            "user_credential",
            "create",
            EntityType::UserCredential,
            AuditAction::Create,
        ),
        (
            "auth_session",
            "create",
            EntityType::AuthSession,
            AuditAction::Create,
        ),
        (
            "external_token",
            "revoke",
            EntityType::ExternalToken,
            AuditAction::Revoke,
        ),
    ];
    for (entity_type, action, _, _) in fixtures {
        sqlx::query(
            "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, $3, $4, $5, 'system', NULL, 'Audit compatibility contract', 'migration', NULL, NULL, NULL, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(lab.id)
        .bind(entity_type)
        .bind(Uuid::new_v4())
        .bind(action)
        .bind(json!({"redacted": true}))
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
    }

    let entries = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    for (_, _, entity_type, action) in fixtures {
        let entry = entries
            .iter()
            .find(|entry| entry.entity_type == entity_type && entry.action == action)
            .unwrap_or_else(|| panic!("missing {entity_type:?}/{action:?} audit entry"));
        assert_eq!(entry.actor.actor_type, ActorType::System);
        assert_eq!(entry.source, WriteSource::Migration);
    }
}
