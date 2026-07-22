use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use muriarc_ai::{
    AiDataAccessContext, AiDataApplyResult, AiDataToolBackend, Citation, DomainToolExecutor,
    DomainToolOutput, DomainToolRequest, StoreDomainToolExecutor, StoreToolAccessContext,
    ToolExecutionError, ToolName, WriteDraft,
};
use muriarc_core::{
    Animal, AnimalEvent, AnimalEventKind, AuditFilter, Cage, EntityType, Experiment, Measurement,
    MeasurementValue, MuriArcStore, Participation, Project, Sample, Sex, WriteSource,
};
use muriarc_core::{AuditContext, Lab};
use muriarc_store_sqlite::SqliteStore;
use serde_json::{Value, json};
use uuid::Uuid;

struct FakeDataBackend;

#[async_trait]
impl AiDataToolBackend for FakeDataBackend {
    fn supported_tools(&self, access: &AiDataAccessContext) -> Vec<ToolName> {
        if access.can_import_anything() {
            vec![ToolName::ImportPreview]
        } else {
            Vec::new()
        }
    }

    async fn execute(
        &self,
        access: &AiDataAccessContext,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        if request.user_id != access.user_id() || request.tool != ToolName::ImportPreview {
            return Err(ToolExecutionError::Rejected {
                code: "fake_forbidden".to_owned(),
            });
        }
        Ok(DomainToolOutput::read(
            json!({"job_id": request.arguments["job_id"]}),
            Vec::new(),
        ))
    }

    async fn apply_import_draft(
        &self,
        _access: &AiDataAccessContext,
        _draft: &WriteDraft,
        _audit: &AuditContext,
    ) -> Result<AiDataApplyResult, ToolExecutionError> {
        Err(ToolExecutionError::Rejected {
            code: "fake_apply_forbidden".to_owned(),
        })
    }
}

struct Fixture {
    store: Arc<SqliteStore>,
    executor: StoreDomainToolExecutor,
    lab_id: Uuid,
    allowed_project_id: Uuid,
    forbidden_project_id: Uuid,
    foreign_project_id: Uuid,
    animal_id: Uuid,
    hidden_animal_id: Uuid,
    foreign_animal_id: Uuid,
    lab_event_id: Uuid,
    allowed_event_id: Uuid,
    forbidden_event_id: Uuid,
    measurement_id: Uuid,
}

async fn fixture() -> Fixture {
    let store = Arc::new(SqliteStore::in_memory().await.unwrap());
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);

    let lab = Lab::new("AI executor lab", now).unwrap();
    let foreign_lab = Lab::new("Foreign lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    store.create_lab(&foreign_lab, &audit).await.unwrap();

    let allowed_project = Project::new(lab.id, "Allowed project", now).unwrap();
    let forbidden_project = Project::new(lab.id, "Forbidden project", now).unwrap();
    let foreign_project = Project::new(foreign_lab.id, "Foreign project", now).unwrap();
    for project in [&allowed_project, &forbidden_project, &foreign_project] {
        store.create_project(project, &audit).await.unwrap();
    }

    let cage = Cage::new(lab.id, "A", "C-001", now).unwrap();
    let second_cage = Cage::new(lab.id, "A", "C-002", now).unwrap();
    let foreign_cage = Cage::new(foreign_lab.id, "F", "FOREIGN", now).unwrap();
    for cage in [&cage, &second_cage, &foreign_cage] {
        store.create_cage(cage, &audit).await.unwrap();
    }

    let mut animal = Animal::new_mouse(lab.id, "M001", Sex::Female, now).unwrap();
    animal.current_cage_id = Some(cage.id);
    let mut hidden_animal = Animal::new_mouse(lab.id, "H001", Sex::Male, now).unwrap();
    hidden_animal.current_cage_id = Some(second_cage.id);
    let foreign_animal = Animal::new_mouse(foreign_lab.id, "F001", Sex::Male, now).unwrap();
    store.create_animal(&animal, &audit).await.unwrap();
    store.create_animal(&hidden_animal, &audit).await.unwrap();
    store.create_animal(&foreign_animal, &audit).await.unwrap();

    let experiment = Experiment::new(lab.id, allowed_project.id, "Allowed study", now).unwrap();
    let forbidden_experiment =
        Experiment::new(lab.id, forbidden_project.id, "Hidden study", now).unwrap();
    store.create_experiment(&experiment, &audit).await.unwrap();
    store
        .create_experiment(&forbidden_experiment, &audit)
        .await
        .unwrap();
    store
        .create_participation(
            &Participation::enroll(experiment.id, animal.id, now),
            &audit,
        )
        .await
        .unwrap();
    store
        .create_participation(
            &Participation::enroll(forbidden_experiment.id, hidden_animal.id, now),
            &audit,
        )
        .await
        .unwrap();

    let mut allowed_event = AnimalEvent::new(
        lab.id,
        animal.id,
        AnimalEventKind::Note {
            body: "allowed".to_owned(),
        },
        now,
        now,
    );
    allowed_event.project_id = Some(allowed_project.id);
    let lab_event = AnimalEvent::new(
        lab.id,
        animal.id,
        AnimalEventKind::Note {
            body: "lab-only".to_owned(),
        },
        now,
        now,
    );
    let mut forbidden_event = AnimalEvent::new(
        lab.id,
        animal.id,
        AnimalEventKind::Note {
            body: "hidden".to_owned(),
        },
        now,
        now,
    );
    forbidden_event.project_id = Some(forbidden_project.id);
    store.append_animal_event(&lab_event, &audit).await.unwrap();
    store
        .append_animal_event(&allowed_event, &audit)
        .await
        .unwrap();
    store
        .append_animal_event(&forbidden_event, &audit)
        .await
        .unwrap();

    let mut measurement = Measurement::draft(
        lab.id,
        allowed_project.id,
        animal.id,
        "body_weight",
        "Body weight",
        MeasurementValue::Number(22.5),
        now,
        now,
    )
    .unwrap();
    measurement.experiment_id = Some(experiment.id);
    measurement.unit = Some("g".to_owned());
    store
        .create_measurement(&measurement, &audit)
        .await
        .unwrap();

    let mut sample = Sample::new(lab.id, allowed_project.id, animal.id, "lung", now, now).unwrap();
    sample.experiment_id = Some(experiment.id);
    sample.set_quantity(1.0, "piece").unwrap();
    store.create_sample(&sample, &audit).await.unwrap();

    let access = StoreToolAccessContext::new(lab.id, [allowed_project.id, foreign_project.id])
        .with_lab_registry_read(true);
    let executor = StoreDomainToolExecutor::new(store.clone(), access);
    Fixture {
        store,
        executor,
        lab_id: lab.id,
        allowed_project_id: allowed_project.id,
        forbidden_project_id: forbidden_project.id,
        foreign_project_id: foreign_project.id,
        animal_id: animal.id,
        hidden_animal_id: hidden_animal.id,
        foreign_animal_id: foreign_animal.id,
        lab_event_id: lab_event.id,
        allowed_event_id: allowed_event.id,
        forbidden_event_id: forbidden_event.id,
        measurement_id: measurement.id,
    }
}

fn request(tool: ToolName, arguments: Value) -> DomainToolRequest {
    DomainToolRequest {
        tool_run_id: Uuid::new_v4(),
        provider_call_id: "integration-call".to_owned(),
        user_id: Uuid::new_v4(),
        tool,
        arguments,
    }
}

fn project_only_executor(fixture: &Fixture) -> StoreDomainToolExecutor {
    StoreDomainToolExecutor::new(
        fixture.store.clone(),
        StoreToolAccessContext::new(fixture.lab_id, [fixture.allowed_project_id]),
    )
}

fn read_output(output: DomainToolOutput) -> (Value, Vec<Citation>) {
    match output {
        DomainToolOutput::Read { data, citations } => (data, citations),
        DomainToolOutput::WriteDraft { .. } => panic!("read executor returned a write draft"),
    }
}

fn assert_rejected(result: Result<DomainToolOutput, ToolExecutionError>, expected: &str) {
    assert!(matches!(
        result,
        Err(ToolExecutionError::Rejected { ref code }) if code == expected
    ));
}

#[tokio::test]
async fn all_declared_tools_execute_bounded_reads_with_citations() {
    let fixture = fixture().await;
    let audit_filter = AuditFilter {
        lab_id: fixture.lab_id,
        project_id: None,
        entity_id: None,
    };
    let audit_count_before = fixture
        .store
        .list_audit_entries(&audit_filter)
        .await
        .unwrap()
        .len();
    let supported = fixture.executor.supported_tools();
    assert_eq!(supported.len(), 7);
    assert!(!supported.contains(&ToolName::ImportPreview));
    assert!(!supported.contains(&ToolName::MutationDraft));

    let calls = [
        (
            ToolName::AnimalSearch,
            json!({"project_id": fixture.allowed_project_id, "query": "M001"}),
        ),
        (
            ToolName::AnimalTimeline,
            json!({
                "animal_id": fixture.animal_id,
                "project_id": fixture.allowed_project_id
            }),
        ),
        (ToolName::CageList, json!({"limit": 1})),
        (ToolName::ProjectList, json!({"status": "active"})),
        (
            ToolName::ExperimentStatus,
            json!({"project_id": fixture.allowed_project_id}),
        ),
        (
            ToolName::MeasurementQuery,
            json!({
                "project_id": fixture.allowed_project_id,
                "measurement_key": "body_weight"
            }),
        ),
        (
            ToolName::SampleInventory,
            json!({"project_id": fixture.allowed_project_id, "sample_type": "lung"}),
        ),
    ];
    for (tool, arguments) in calls {
        let (data, citations) = read_output(
            fixture
                .executor
                .execute(request(tool, arguments))
                .await
                .unwrap(),
        );
        let expected = if tool == ToolName::AnimalTimeline {
            4
        } else {
            1
        };
        assert_eq!(
            data["items"].as_array().unwrap().len(),
            expected,
            "{tool:?}"
        );
        assert!(!citations.is_empty(), "{tool:?}");
        assert!(
            citations
                .iter()
                .all(|citation| { citation.revision.is_none_or(|revision| revision > 0) })
        );
    }
    let audit_count_after = fixture
        .store
        .list_audit_entries(&audit_filter)
        .await
        .unwrap()
        .len();
    assert_eq!(audit_count_after, audit_count_before);
}

#[tokio::test]
async fn injected_data_backend_is_the_only_way_data_tools_are_advertised_and_dispatched() {
    let fixture = fixture().await;
    let user_id = Uuid::new_v4();
    let executor = StoreDomainToolExecutor::new(
        fixture.store.clone(),
        StoreToolAccessContext::new(fixture.lab_id, [fixture.allowed_project_id]),
    )
    .with_data_tools(
        AiDataAccessContext::new(
            fixture.lab_id,
            user_id,
            [fixture.allowed_project_id],
            std::iter::empty(),
            false,
        ),
        Arc::new(FakeDataBackend),
    );
    assert!(
        executor
            .supported_tools()
            .contains(&ToolName::ImportPreview)
    );
    assert!(!executor.supported_tools().contains(&ToolName::ExportCreate));

    let job_id = Uuid::new_v4();
    let output = executor
        .execute(DomainToolRequest {
            tool_run_id: Uuid::new_v4(),
            provider_call_id: "fake-call".to_owned(),
            user_id,
            tool: ToolName::ImportPreview,
            arguments: json!({"job_id": job_id}),
        })
        .await
        .unwrap();
    let (data, citations) = read_output(output);
    assert_eq!(data["job_id"], job_id.to_string());
    assert!(citations.is_empty());
}

#[tokio::test]
async fn project_and_lab_boundaries_cannot_be_bypassed() {
    let fixture = fixture().await;
    assert_rejected(
        fixture
            .executor
            .execute(request(
                ToolName::MeasurementQuery,
                json!({"project_id": fixture.forbidden_project_id}),
            ))
            .await,
        "project_forbidden",
    );
    assert_rejected(
        fixture
            .executor
            .execute(request(
                ToolName::SampleInventory,
                json!({"project_id": fixture.foreign_project_id}),
            ))
            .await,
        "project_forbidden",
    );
    assert_rejected(
        fixture
            .executor
            .execute(request(
                ToolName::AnimalTimeline,
                json!({"animal_id": fixture.foreign_animal_id}),
            ))
            .await,
        "animal_forbidden",
    );

    let (projects, _) = read_output(
        fixture
            .executor
            .execute(request(ToolName::ProjectList, json!({})))
            .await
            .unwrap(),
    );
    assert_eq!(projects["items"].as_array().unwrap().len(), 1);
    assert_eq!(projects["items"][0]["lab_id"], json!(fixture.lab_id));
}

#[tokio::test]
async fn project_only_access_cannot_fall_back_to_the_lab_registry() {
    let fixture = fixture().await;
    let executor = project_only_executor(&fixture);
    assert!(!executor.supported_tools().contains(&ToolName::CageList));

    assert_rejected(
        executor
            .execute(request(ToolName::AnimalSearch, json!({})))
            .await,
        "project_required",
    );
    assert_rejected(
        executor
            .execute(request(ToolName::CageList, json!({})))
            .await,
        "lab_registry_forbidden",
    );
    assert_rejected(
        executor
            .execute(request(
                ToolName::AnimalTimeline,
                json!({"animal_id": fixture.hidden_animal_id}),
            ))
            .await,
        "project_required",
    );
    assert_rejected(
        executor
            .execute(request(
                ToolName::AnimalTimeline,
                json!({
                    "animal_id": fixture.hidden_animal_id,
                    "project_id": fixture.allowed_project_id
                }),
            ))
            .await,
        "animal_forbidden",
    );

    let (data, _) = read_output(
        executor
            .execute(request(
                ToolName::AnimalSearch,
                json!({"project_id": fixture.allowed_project_id}),
            ))
            .await
            .unwrap(),
    );
    let animal_ids = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|animal| animal["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(animal_ids, vec![fixture.animal_id.to_string()]);
    assert!(!animal_ids.contains(&fixture.hidden_animal_id.to_string()));

    let (timeline, _) = read_output(
        executor
            .execute(request(
                ToolName::AnimalTimeline,
                json!({
                    "animal_id": fixture.animal_id,
                    "project_id": fixture.allowed_project_id
                }),
            ))
            .await
            .unwrap(),
    );
    let event_ids = timeline["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(event_ids.contains(&fixture.allowed_event_id.to_string()));
    assert!(!event_ids.contains(&fixture.lab_event_id.to_string()));
    assert!(!event_ids.contains(&fixture.forbidden_event_id.to_string()));
    assert_eq!(
        event_ids.len(),
        4,
        "allowed note plus enrollment, measurement and sample events"
    );
}

#[tokio::test]
async fn timelines_hide_events_from_projects_outside_the_context() {
    let fixture = fixture().await;
    let (data, citations) = read_output(
        fixture
            .executor
            .execute(request(
                ToolName::AnimalTimeline,
                json!({"animal_id": fixture.animal_id}),
            ))
            .await
            .unwrap(),
    );
    let event_ids = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(event_ids.contains(&fixture.allowed_event_id.to_string().as_str()));
    assert!(!event_ids.contains(&fixture.forbidden_event_id.to_string().as_str()));
    assert!(citations.iter().any(|citation| {
        citation.entity_type == EntityType::AnimalEvent
            && citation.entity_id == fixture.allowed_event_id
    }));
}

#[tokio::test]
async fn unknown_fields_lengths_and_page_bounds_are_rejected() {
    let fixture = fixture().await;
    assert_rejected(
        fixture
            .executor
            .execute(request(
                ToolName::MeasurementQuery,
                json!({
                    "project_id": fixture.allowed_project_id,
                    "raw_sql": "select * from measurements"
                }),
            ))
            .await,
        "invalid_arguments",
    );
    assert_rejected(
        fixture
            .executor
            .execute(request(
                ToolName::AnimalSearch,
                json!({"query": "x".repeat(257)}),
            ))
            .await,
        "query_too_long",
    );
    assert_rejected(
        fixture
            .executor
            .execute(request(ToolName::CageList, json!({"limit": 101})))
            .await,
        "limit_out_of_range",
    );
    assert_rejected(
        fixture
            .executor
            .execute(request(ToolName::CageList, json!({"offset": 10001})))
            .await,
        "offset_out_of_range",
    );

    let (data, _) = read_output(
        fixture
            .executor
            .execute(request(
                ToolName::CageList,
                json!({"limit": 1, "offset": 1}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(data["items"].as_array().unwrap().len(), 1);
    assert_eq!(data["page"]["limit"], 1);
    assert_eq!(data["page"]["offset"], 1);
    assert_eq!(data["page"]["total"], 2);
}

#[tokio::test]
async fn measurement_result_is_grounded_in_its_store_revision() {
    let fixture = fixture().await;
    let (_, citations) = read_output(
        fixture
            .executor
            .execute(request(
                ToolName::MeasurementQuery,
                json!({"project_id": fixture.allowed_project_id}),
            ))
            .await
            .unwrap(),
    );
    assert!(citations.contains(&Citation::new(
        EntityType::Measurement,
        fixture.measurement_id,
        Some(1),
    )));
}

#[tokio::test]
async fn write_and_import_tools_are_never_executed() {
    let fixture = fixture().await;
    for tool in [
        ToolName::ImportPreview,
        ToolName::ImportCommitDraft,
        ToolName::ExperimentTemplateDraft,
        ToolName::MutationDraft,
    ] {
        assert_rejected(
            fixture.executor.execute(request(tool, json!({}))).await,
            "unsupported_tool",
        );
    }
    assert_rejected(
        fixture
            .executor
            .execute(request(ToolName::ExportCreate, json!({})))
            .await,
        "autonomy_confirmation_required",
    );
}

#[tokio::test]
async fn mutation_tool_rejects_breeding_and_animal_record_operations() {
    let fixture = fixture().await;
    let animal = fixture.store.get_animal(fixture.animal_id).await.unwrap();
    let executor = StoreDomainToolExecutor::new(
        fixture.store.clone(),
        StoreToolAccessContext::new(fixture.lab_id, [fixture.allowed_project_id])
            .with_writable_projects([fixture.allowed_project_id]),
    );

    for operation in ["create_mating_event", "update_animal"] {
        assert_rejected(
            executor
                .execute(request(
                    ToolName::MutationDraft,
                    json!({
                        "operation": operation,
                        "project_id": fixture.allowed_project_id,
                        "animal_id": fixture.animal_id,
                        "animal_revision": animal.meta.revision,
                        "key": "not_applicable",
                        "label": "Not applicable",
                        "value": {"type": "text", "value": "not applicable"},
                        "measured_at": Utc::now(),
                    }),
                ))
                .await,
            "invalid_arguments",
        );
    }
}

#[tokio::test]
async fn measurement_write_is_only_a_scoped_reviewable_draft() {
    let fixture = fixture().await;
    let animal = fixture.store.get_animal(fixture.animal_id).await.unwrap();
    let executor = StoreDomainToolExecutor::new(
        fixture.store.clone(),
        StoreToolAccessContext::new(fixture.lab_id, [fixture.allowed_project_id])
            .with_writable_projects([fixture.allowed_project_id]),
    );
    assert!(
        executor
            .supported_tools()
            .contains(&ToolName::MutationDraft)
    );
    let before = fixture
        .store
        .list_measurements(&muriarc_core::MeasurementFilter {
            project_id: fixture.allowed_project_id,
            experiment_id: None,
            animal_id: Some(fixture.animal_id),
        })
        .await
        .unwrap()
        .len();
    let output = executor
        .execute(request(
            ToolName::MutationDraft,
            json!({
                "operation": "record_measurement",
                "project_id": fixture.allowed_project_id,
                "animal_id": fixture.animal_id,
                "animal_revision": animal.meta.revision,
                "key": "body_weight",
                "label": "Body weight",
                "value": {"type": "number", "value": 23.1},
                "unit": "g",
                "measured_at": Utc::now(),
            }),
        ))
        .await
        .unwrap();
    let DomainToolOutput::WriteDraft { draft, citations } = output else {
        panic!("mutation tool must return a write draft");
    };
    assert_eq!(draft.project_id(), Some(fixture.allowed_project_id));
    assert_eq!(draft.status(), muriarc_ai::DraftStatus::PendingApproval);
    assert!(
        citations
            .iter()
            .any(|citation| citation.entity_id == fixture.animal_id)
    );
    let after = fixture
        .store
        .list_measurements(&muriarc_core::MeasurementFilter {
            project_id: fixture.allowed_project_id,
            experiment_id: None,
            animal_id: Some(fixture.animal_id),
        })
        .await
        .unwrap()
        .len();
    assert_eq!(
        after, before,
        "draft construction must not write a measurement"
    );

    assert_rejected(
        executor
            .execute(request(
                ToolName::MutationDraft,
                json!({
                    "operation": "record_measurement",
                    "project_id": fixture.forbidden_project_id,
                    "animal_id": fixture.animal_id,
                    "animal_revision": animal.meta.revision,
                    "key": "body_weight",
                    "label": "Body weight",
                    "value": {"type": "number", "value": 23.1},
                    "unit": "g",
                    "measured_at": Utc::now(),
                }),
            ))
            .await,
        "project_write_forbidden",
    );
}
