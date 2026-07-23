use chrono::Utc;
use muriarc_core::*;
use muriarc_store_sqlite::SqliteStore;
use serde_json::json;
use uuid::Uuid;

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
