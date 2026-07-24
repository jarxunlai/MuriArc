use chrono::Utc;
use muriarc_core::*;
use muriarc_store_sqlite::SqliteStore;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn legacy_conversations_reject_messages_and_autonomy_changes() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let bootstrap = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("Legacy AI read-only contract", now).unwrap();
    store.create_lab(&lab, &bootstrap).await.unwrap();
    let user = User::new(
        lab.id,
        "legacy-ai@example.test",
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
             ) VALUES (?, ?, NULL, ?, ?, NULL, NULL, 1, ?, ?, NULL, 1)",
        )
        .bind(id.to_string())
        .bind(lab.id.to_string())
        .bind(user.id.to_string())
        .bind(title)
        .bind(now)
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
         ) VALUES (?, ?, ?, NULL, ?, NULL, 'ask', ?, 1, NULL, ?, NULL, NULL, ?, ?, NULL, 1)",
    )
    .bind(existing_grant_id.to_string())
    .bind(granted_id.to_string())
    .bind(lab.id.to_string())
    .bind(user.id.to_string())
    .bind(serde_json::to_string(&vec![AiActionCategory::Read]).unwrap())
    .bind(now)
    .bind(now)
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
        request_id: Some("legacy-write-rejection".to_owned()),
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

    let project = Project::new(lab.id, "Legacy draft project", now).unwrap();
    store.create_project(&project, &bootstrap).await.unwrap();
    let animal = Animal::new_mouse(lab.id, "LEGACY-DRAFT-1", Sex::Female, now).unwrap();
    store.create_animal(&animal, &bootstrap).await.unwrap();
    let draft_conversation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, legacy_read_only, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, ?, ?, 'Legacy conversation with pending draft',
            NULL, NULL, 1, ?, ?, NULL, 1)",
    )
    .bind(draft_conversation_id.to_string())
    .bind(lab.id.to_string())
    .bind(project.id.to_string())
    .bind(user.id.to_string())
    .bind(now)
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
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'awaiting_approval', 'ai', ?,
            NULL, NULL, ?, ?, NULL, 1)",
    )
    .bind(legacy_tool.id.to_string())
    .bind(draft_conversation_id.to_string())
    .bind(lab.id.to_string())
    .bind(project.id.to_string())
    .bind(user.id.to_string())
    .bind(&legacy_tool.tool_name)
    .bind(legacy_tool.input.to_string())
    .bind(legacy_tool.output.as_ref().unwrap().to_string())
    .bind(now)
    .bind(now)
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
         ) VALUES (?, ?, ?, 'pending', NULL, NULL, NULL, ?, ?, NULL, 1)",
    )
    .bind(legacy_approval.id.to_string())
    .bind(legacy_tool.id.to_string())
    .bind(legacy_approval.requested_diff.to_string())
    .bind(now)
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
        source: WriteSource::Desktop,
        request_id: Some("legacy-draft-decision".to_owned()),
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
async fn approved_ai_measurement_is_applied_atomically_as_unsigned_draft() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let bootstrap = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("AI contract", now).unwrap();
    store.create_lab(&lab, &bootstrap).await.unwrap();
    let user = User::new(lab.id, "researcher@example.test", "Researcher", now).unwrap();
    store.create_user(&user, &bootstrap).await.unwrap();
    let project = Project::new(lab.id, "Study", now).unwrap();
    store.create_project(&project, &bootstrap).await.unwrap();
    let profile = AiModelProfile {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        name: "AI operations test model".to_owned(),
        current_version: 1,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    let profile_version = AiModelProfileVersion {
        profile_id: profile.id,
        version: 1,
        protocol: AiProviderProtocol::OpenaiChatCompletions,
        transport: AiProviderTransport::OpenAiCompatible,
        base_url: "https://provider.example.test/v1".to_owned(),
        normalized_base_url: "https://provider.example.test/v1".to_owned(),
        model_id: "operations-test-model".to_owned(),
        supports_vision: false,
        context_window_tokens: 16_384,
        max_input_tokens: 8_192,
        max_output_tokens: 2_048,
        history_token_budget: 4_096,
        history_turns: 20,
        temperature: 0.0,
        timeout_ms: 30_000,
        created_at: now,
    };
    store
        .create_ai_model_profile(&profile, &profile_version, &bootstrap)
        .await
        .unwrap();
    let animal = Animal::new_mouse(lab.id, "M001", Sex::Female, now).unwrap();
    store.create_animal(&animal, &bootstrap).await.unwrap();
    let experiment = Experiment::new(lab.id, project.id, "Weights", now).unwrap();
    store
        .create_experiment(&experiment, &bootstrap)
        .await
        .unwrap();
    let participation = Participation::enroll(experiment.id, animal.id, now);
    store
        .create_participation(&participation, &bootstrap)
        .await
        .unwrap();
    let enrolled_animal = store.get_animal(animal.id).await.unwrap();

    let ai_audit = AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(user.id),
            display_name: "MuriArc AI for Researcher".to_owned(),
        },
        source: WriteSource::Ai,
        request_id: Some("turn-1".to_owned()),
        reason: Some("AI proposed a structured measurement".to_owned()),
    };
    let conversation = AiConversation {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: user.id,
        title: "Record weight".to_owned(),
        model_profile: Some(AiModelProfileBinding {
            profile_id: profile.id,
            profile_version: 1,
        }),
        legacy_read_only: false,
        pinned_at: None,
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_conversation(&conversation, &ai_audit)
        .await
        .unwrap();
    let mut tool = ToolRun {
        id: Uuid::new_v4(),
        conversation_id: Some(conversation.id),
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: user.id,
        tool_name: "mutation_draft".to_owned(),
        input: json!({"operation": "record_measurement"}),
        output: Some(json!({"draftId": "pending"})),
        status: ToolRunStatus::AwaitingApproval,
        source: WriteSource::Ai,
        started_at: Some(now),
        completed_at: None,
        error: None,
        meta: RecordMeta::new(now),
    };
    store.create_tool_run(&tool, &ai_audit).await.unwrap();
    let mut approval = Approval {
        id: Uuid::new_v4(),
        tool_run_id: tool.id,
        requested_diff: json!({"changes": [{"path": "/measurements/new"}]}),
        decision: ApprovalDecision::Pending,
        decided_by: None,
        decided_at: None,
        reason: None,
        meta: RecordMeta::new(now),
    };
    store.create_approval(&approval, &ai_audit).await.unwrap();

    let mut measurement = Measurement::draft(
        lab.id,
        project.id,
        animal.id,
        "body_weight",
        "Body weight",
        MeasurementValue::Number(22.4),
        now,
        now,
    )
    .unwrap();
    measurement.experiment_id = Some(experiment.id);
    measurement.unit = Some("g".to_owned());
    let decided_at = Utc::now();
    let expected_tool_revision = tool.meta.revision;
    tool.status = ToolRunStatus::Completed;
    tool.completed_at = Some(decided_at);
    tool.meta.touch(decided_at);
    let expected_approval_revision = approval.meta.revision;
    approval.decision = ApprovalDecision::Approved;
    approval.decided_by = Some(user.id);
    approval.decided_at = Some(decided_at);
    approval.reason = Some("Verified source value".to_owned());
    approval.meta.touch(decided_at);
    let human_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Desktop,
        request_id: Some("approval-1".to_owned()),
        reason: approval.reason.clone(),
    };
    store
        .apply_ai_measurement_draft(
            &measurement,
            enrolled_animal.meta.revision,
            &tool,
            expected_tool_revision,
            &approval,
            expected_approval_revision,
            &human_audit,
        )
        .await
        .unwrap();

    let saved = store.get_measurement(measurement.id).await.unwrap();
    assert_eq!(saved.status, RecordStatus::Draft);
    assert_eq!(saved.signed_by, None);
    assert_eq!(
        store.get_approval(approval.id).await.unwrap().decision,
        ApprovalDecision::Approved
    );
    assert!(store.list_animal_events(animal.id).await.unwrap().iter().any(|event| {
        matches!(event.kind, AnimalEventKind::MeasurementRecorded { measurement_id } if measurement_id == measurement.id)
    }));
    let provenance = store
        .list_provenance(&ProvenanceFilter {
            lab_id: lab.id,
            entity_type: Some(EntityType::Measurement),
            entity_id: Some(measurement.id),
            ..ProvenanceFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0].source, ProvenanceSource::Ai);
    assert_eq!(provenance[0].tool_run_id, Some(tool.id));
}
