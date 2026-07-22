use std::{fs, io::Cursor, path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use muriarc_core::{
    Actor, AiScope, Animal, AnimalEvent, AnimalEventKind, Attachment, AuditContext, Cage,
    EntityType, Experiment, ExperimentTemplateVersion, FieldValueType, Lab, LabRole, Measurement,
    MeasurementFilter, MeasurementValue, MuriArcStore, ParentType, Participation, Pedigree,
    Project, ProjectAnimalAssignment, ProjectRole, ProvenanceFilter, ProvenanceSource, RecordMeta,
    RecordStatus, Sample, Sex, TemplateField, User, WriteSource,
};
use muriarc_data::DataFiles;
use muriarc_importer::read_xlsx;
use muriarc_store_sqlite::SqliteStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState, AuthPrincipal, StaticTokenAuthenticator, StoreJobRepository, application_router,
};

const HUMAN_TOKEN: &str = "human-token-000000000000000000000000";
const AI_TOKEN: &str = "external-ai-token-0000000000000000";
const PROJECT_TOKEN: &str = "project-viewer-token-0000000000000000";
const PROJECT_AI_TOKEN: &str = "project-ai-token-00000000000000000000";
const PROJECT_EDITOR_TOKEN: &str = "project-editor-token-0000000000000000";
const ANIMAL_MANAGER_TOKEN: &str = "animal-manager-token-00000000000000000";
const OTHER_ADMIN_TOKEN: &str = "other-admin-token-000000000000000000";

struct Fixture {
    app: Router,
    store: Arc<SqliteStore>,
    lab_id: Uuid,
    user_id: Uuid,
    project_id: Uuid,
    attachment_root: PathBuf,
    _data_dir: tempfile::TempDir,
}

impl Fixture {
    async fn new(ui_dir: Option<PathBuf>) -> Self {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = chrono::Utc::now();
        let lab = Lab::new("Test lab", now).unwrap();
        store
            .create_lab(&lab, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();
        let project = Project::new(lab.id, "Visible project", now).unwrap();
        store
            .create_project(&project, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();
        let user_id = Uuid::new_v4();
        let mut user =
            User::new(lab.id, "animal-manager@example.test", "Animal manager", now).unwrap();
        user.id = user_id;
        store
            .create_user(&user, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();
        let human = AuthPrincipal::human(user_id, "Animal manager", lab.id, [LabRole::LabAdmin]);
        let external = human.clone().with_ai_scopes([AiScope::Read]);
        let project_user =
            User::new(lab.id, "project-viewer@example.test", "Project viewer", now).unwrap();
        store
            .create_user(&project_user, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();
        let project_viewer = AuthPrincipal::human(project_user.id, "Project viewer", lab.id, [])
            .with_project_role(project.id, ProjectRole::Viewer);
        let project_external = project_viewer.clone().with_ai_scopes([AiScope::Read]);
        let editor_user =
            User::new(lab.id, "project-editor@example.test", "Project editor", now).unwrap();
        store
            .create_user(&editor_user, &AuditContext::system(WriteSource::Migration))
            .await
            .unwrap();
        let project_editor = AuthPrincipal::human(editor_user.id, "Project editor", lab.id, [])
            .with_project_role(project.id, ProjectRole::Editor);
        let animal_manager_user = User::new(
            lab.id,
            "genetics-manager@example.test",
            "Genetics animal manager",
            now,
        )
        .unwrap();
        store
            .create_user(
                &animal_manager_user,
                &AuditContext::system(WriteSource::Migration),
            )
            .await
            .unwrap();
        let animal_manager = AuthPrincipal::human(
            animal_manager_user.id,
            "Genetics animal manager",
            lab.id,
            [LabRole::AnimalManager],
        );
        let other_admin = AuthPrincipal::human(
            Uuid::new_v4(),
            "Other administrator",
            lab.id,
            [LabRole::LabAdmin],
        );
        let authenticator = StaticTokenAuthenticator::new([
            (HUMAN_TOKEN.to_owned(), human),
            (AI_TOKEN.to_owned(), external),
            (PROJECT_TOKEN.to_owned(), project_viewer),
            (PROJECT_AI_TOKEN.to_owned(), project_external),
            (PROJECT_EDITOR_TOKEN.to_owned(), project_editor),
            (ANIMAL_MANAGER_TOKEN.to_owned(), animal_manager),
            (OTHER_ADMIN_TOKEN.to_owned(), other_admin),
        ])
        .unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let attachment_root = data_dir.path().join("attachments");
        fs::create_dir_all(&attachment_root).unwrap();
        let state = AppState::new(
            store.clone(),
            Arc::new(authenticator),
            Arc::new(StoreJobRepository::new(store.clone())),
        )
        .with_data_storage(
            DataFiles::new(data_dir.path().join("data")),
            attachment_root.clone(),
        );
        Self {
            app: application_router(state, ui_dir),
            store,
            lab_id: lab.id,
            user_id,
            project_id: project.id,
            attachment_root,
            _data_dir: data_dir,
        }
    }

    fn request(&self, method: Method, uri: &str, token: &str, value: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    }
}

#[tokio::test]
async fn create_animal_uses_shared_normalization_and_validation() {
    let fixture = Fixture::new(None).await;
    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/animals",
            HUMAN_TOKEN,
            json!({
                "display_id": "  M-APP-001  ",
                "sex": "female",
                "strain": "   ",
                "legacy_id": "  legacy-001  "
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await;
    assert_eq!(body["data"]["display_id"], "M-APP-001");
    assert!(body["data"]["strain"].is_null());
    assert_eq!(body["data"]["legacy_id"], "legacy-001");

    let animal_id = Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();
    let stored = fixture.store.get_animal(animal_id).await.unwrap();
    assert_eq!(stored.display_id, "M-APP-001");
    assert_eq!(stored.strain, None);
    assert_eq!(stored.legacy_id.as_deref(), Some("legacy-001"));

    let rejected = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/animals",
            HUMAN_TOKEN,
            json!({
                "display_id": "x".repeat(65),
                "sex": "male"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let rejected = response_json(rejected).await;
    assert_eq!(rejected["error"]["code"], "validation_error");
    assert_eq!(
        rejected["error"]["message"],
        "animal.display_id must not exceed 64 characters"
    );
}

#[tokio::test]
async fn project_scoped_timeline_never_leaks_other_project_or_unscoped_events() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let animal = Animal::new_mouse(fixture.lab_id, "M-project", Sex::Female, now).unwrap();
    fixture.store.create_animal(&animal, &audit).await.unwrap();
    let experiment = Experiment::new(
        fixture.lab_id,
        fixture.project_id,
        "Visible experiment",
        now,
    )
    .unwrap();
    fixture
        .store
        .create_experiment(&experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_participation(
            &Participation::enroll(experiment.id, animal.id, now),
            &audit,
        )
        .await
        .unwrap();

    let other_project = Project::new(fixture.lab_id, "Hidden project", now).unwrap();
    fixture
        .store
        .create_project(&other_project, &audit)
        .await
        .unwrap();
    for (project_id, body) in [
        (Some(fixture.project_id), "visible-project-event"),
        (Some(other_project.id), "hidden-project-event"),
        (None, "unscoped-lab-event"),
    ] {
        let mut event = AnimalEvent::new(
            fixture.lab_id,
            animal.id,
            AnimalEventKind::Note {
                body: body.to_owned(),
            },
            now,
            now,
        );
        event.project_id = project_id;
        fixture
            .store
            .append_animal_event(&event, &audit)
            .await
            .unwrap();
    }

    let rest = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/animals/{}/events?project_id={}",
                animal.id, fixture.project_id
            ),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(rest.status(), StatusCode::OK);
    let rest = response_json(rest).await;
    assert_eq!(rest["count"], json!(2));
    assert!(
        rest["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["project_id"] == json!(fixture.project_id))
    );
    let rest_text = rest.to_string();
    assert!(rest_text.contains("visible-project-event"));
    assert!(!rest_text.contains("hidden-project-event"));
    assert!(!rest_text.contains("unscoped-lab-event"));

    let mcp = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/mcp",
            PROJECT_AI_TOKEN,
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {
                    "name": "animal.timeline",
                    "arguments": {
                        "animal_id": animal.id,
                        "project_id": fixture.project_id,
                        "limit": 20
                    }
                }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(mcp.status(), StatusCode::OK);
    let mcp = response_json(mcp).await;
    assert_eq!(mcp["result"]["structuredContent"]["count"], json!(2));
    assert!(
        mcp["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["project_id"] == json!(fixture.project_id))
    );
    let mcp_text = mcp.to_string();
    assert!(mcp_text.contains("visible-project-event"));
    assert!(!mcp_text.contains("hidden-project-event"));
    assert!(!mcp_text.contains("unscoped-lab-event"));
}

#[tokio::test]
async fn project_viewer_animal_detail_is_isolated_and_audit_summaries_are_redacted() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let hidden_project = Project::new(fixture.lab_id, "Hidden detail project", now).unwrap();
    fixture
        .store
        .create_project(&hidden_project, &audit)
        .await
        .unwrap();

    let visible_experiment = Experiment::new(
        fixture.lab_id,
        fixture.project_id,
        "Visible detail experiment",
        now,
    )
    .unwrap();
    let second_visible_experiment = Experiment::new(
        fixture.lab_id,
        fixture.project_id,
        "Second visible detail experiment",
        now,
    )
    .unwrap();
    let hidden_experiment = Experiment::new(
        fixture.lab_id,
        hidden_project.id,
        "Hidden detail experiment",
        now,
    )
    .unwrap();
    fixture
        .store
        .create_experiment(&visible_experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_experiment(&second_visible_experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_experiment(&hidden_experiment, &audit)
        .await
        .unwrap();

    let animal = Animal::new_mouse(fixture.lab_id, "DETAIL-SUBJECT", Sex::Female, now).unwrap();
    let visible_parent =
        Animal::new_mouse(fixture.lab_id, "DETAIL-VISIBLE-PARENT", Sex::Female, now).unwrap();
    let hidden_parent =
        Animal::new_mouse(fixture.lab_id, "DETAIL-HIDDEN-PARENT", Sex::Male, now).unwrap();
    for candidate in [&animal, &visible_parent, &hidden_parent] {
        fixture
            .store
            .create_animal(candidate, &audit)
            .await
            .unwrap();
    }

    for (experiment_id, animal_id) in [
        (visible_experiment.id, animal.id),
        (second_visible_experiment.id, animal.id),
        (hidden_experiment.id, animal.id),
        (visible_experiment.id, visible_parent.id),
        (hidden_experiment.id, hidden_parent.id),
    ] {
        fixture
            .store
            .create_participation(
                &Participation::enroll(experiment_id, animal_id, now),
                &audit,
            )
            .await
            .unwrap();
    }

    for (project_id, note) in [
        (fixture.project_id, "visible-detail-event"),
        (hidden_project.id, "hidden-detail-event"),
    ] {
        let mut event = AnimalEvent::new(
            fixture.lab_id,
            animal.id,
            AnimalEventKind::Note {
                body: note.to_owned(),
            },
            now,
            now,
        );
        event.project_id = Some(project_id);
        fixture
            .store
            .append_animal_event(&event, &audit)
            .await
            .unwrap();
    }

    for (project_id, experiment_id, key, label, value, measured_at) in [
        (
            fixture.project_id,
            visible_experiment.id,
            "body_weight",
            "Visible detail measurement",
            21.5,
            now,
        ),
        (
            hidden_project.id,
            hidden_experiment.id,
            "body_weight",
            "Hidden detail measurement",
            27.5,
            now + chrono::Duration::seconds(1),
        ),
    ] {
        let mut measurement = Measurement::draft(
            fixture.lab_id,
            project_id,
            animal.id,
            key,
            label,
            MeasurementValue::Number(value),
            measured_at,
            now,
        )
        .unwrap();
        measurement.experiment_id = Some(experiment_id);
        measurement.unit = Some("g".to_owned());
        fixture
            .store
            .create_measurement(&measurement, &audit)
            .await
            .unwrap();
    }

    for (project_id, experiment_id, sample_type) in [
        (
            fixture.project_id,
            visible_experiment.id,
            "visible-detail-sample",
        ),
        (
            hidden_project.id,
            hidden_experiment.id,
            "hidden-detail-sample",
        ),
    ] {
        let mut sample =
            Sample::new(fixture.lab_id, project_id, animal.id, sample_type, now, now).unwrap();
        sample.experiment_id = Some(experiment_id);
        fixture.store.create_sample(&sample, &audit).await.unwrap();
    }

    for (project_id, file_name, fill) in [
        (fixture.project_id, "visible-detail.txt", 'a'),
        (hidden_project.id, "hidden-detail.txt", 'b'),
    ] {
        let attachment = Attachment {
            id: Uuid::new_v4(),
            lab_id: fixture.lab_id,
            project_id: Some(project_id),
            entity_type: "animal".to_owned(),
            entity_id: animal.id,
            file_name: file_name.to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: format!("objects/{}", Uuid::new_v4()),
            size_bytes: 1,
            sha256: fill.to_string().repeat(64),
            version: 1,
            meta: RecordMeta::new(now),
        };
        fixture
            .store
            .create_attachment(&attachment, &audit)
            .await
            .unwrap();
    }

    for (parent_id, parent_type) in [
        (visible_parent.id, ParentType::Mother),
        (hidden_parent.id, ParentType::Father),
    ] {
        fixture
            .store
            .create_pedigree(
                &Pedigree {
                    id: Uuid::new_v4(),
                    animal_id: animal.id,
                    parent_id,
                    parent_type,
                    meta: RecordMeta::new(now),
                },
                &audit,
            )
            .await
            .unwrap();
    }

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/animal-overviews?project_id={}&limit=500",
                fixture.project_id
            ),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let overviews = response_json(response).await;
    let overview_rows = overviews["data"].as_array().unwrap();
    let overview_animal_ids = overview_rows
        .iter()
        .map(|overview| serde_json::from_value::<Uuid>(overview["animal"]["id"].clone()).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        overview_animal_ids,
        std::collections::BTreeSet::from([animal.id, visible_parent.id])
    );
    let subject_overview = overview_rows
        .iter()
        .find(|overview| overview["animal"]["id"] == json!(animal.id))
        .unwrap();
    let subject_project_ids = subject_overview["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| serde_json::from_value::<Uuid>(project["id"].clone()).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        (
            subject_project_ids,
            subject_overview["latest_weight"]["value"].as_f64().unwrap(),
        ),
        (std::collections::BTreeSet::from([fixture.project_id]), 21.5,)
    );
    for overview in overview_rows {
        let projects = overview["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"], json!(fixture.project_id));
    }

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/experiments?project_id={}", fixture.project_id),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let experiments = response_json(response).await;
    let experiment_ids = experiments["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|experiment| serde_json::from_value::<Uuid>(experiment["id"].clone()).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        experiment_ids,
        std::collections::BTreeSet::from([visible_experiment.id, second_visible_experiment.id,])
    );

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/animals/{}/detail?project_id={}&limit=500",
                animal.id, fixture.project_id
            ),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    let data = &detail["data"];
    assert!(
        data["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["project_id"] == json!(fixture.project_id))
    );
    let detail_experiments = data["experiments"].as_array().unwrap();
    assert!(
        detail_experiments
            .iter()
            .all(|experiment| experiment["project"]["id"] == json!(fixture.project_id))
    );
    let detail_experiment_ids = detail_experiments
        .iter()
        .map(|experiment| {
            serde_json::from_value::<Uuid>(experiment["experiment"]["id"].clone()).unwrap()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        detail_experiment_ids,
        std::collections::BTreeSet::from([visible_experiment.id, second_visible_experiment.id,])
    );
    assert_eq!(data["measurements"].as_array().unwrap().len(), 1);
    assert_eq!(
        data["measurements"][0]["project_id"],
        json!(fixture.project_id)
    );
    assert_eq!(data["samples"].as_array().unwrap().len(), 1);
    assert_eq!(data["samples"][0]["project_id"], json!(fixture.project_id));
    assert_eq!(data["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(
        data["attachments"][0]["project_id"],
        json!(fixture.project_id)
    );
    assert_eq!(data["pedigree"].as_array().unwrap().len(), 1);
    assert_eq!(
        data["pedigree"][0]["related_animal"]["id"],
        json!(visible_parent.id)
    );
    assert_eq!(data["audit_visible"], false);
    assert_eq!(data["audits"], json!([]));
    assert_eq!(data["provenance"], json!([]));
    let detail_text = detail.to_string();
    for hidden in [
        "Hidden detail project",
        "Hidden detail experiment",
        "hidden-detail-event",
        "Hidden detail measurement",
        "hidden-detail-sample",
        "hidden-detail.txt",
        "DETAIL-HIDDEN-PARENT",
    ] {
        assert!(!detail_text.contains(hidden), "leaked {hidden}");
    }

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/animals/{}/detail?limit=500", animal.id),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    assert_eq!(detail["data"]["audit_visible"], true);
    let audits = detail["data"]["audits"].as_array().unwrap();
    assert!(!audits.is_empty());
    for entry in audits {
        assert!(entry.get("before").is_none());
        assert!(entry.get("after").is_none());
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn unconfigured_ai_runtime_fails_closed_without_echoing_credentials() {
    let fixture = Fixture::new(None).await;
    let secret = "server-user-api-key-must-never-be-returned";
    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::PUT,
            "/api/v1/ai/settings",
            HUMAN_TOKEN,
            json!({
                "enabled": true,
                "providerKind": "open_ai_compatible",
                "model": "test-model",
                "baseUrl": "https://example.test/v1",
                "apiKey": secret
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], json!("ai_runtime_not_configured"));
    assert!(!body.to_string().contains(secret));
}

#[tokio::test]
async fn external_ai_tokens_cannot_change_settings_or_approve_drafts() {
    let fixture = Fixture::new(None).await;
    let settings = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::PUT,
            "/api/v1/ai/settings",
            AI_TOKEN,
            json!({
                "enabled": false,
                "providerKind": "open_ai_compatible",
                "model": "test-model",
                "baseUrl": "https://example.test/v1"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(settings.status(), StatusCode::FORBIDDEN);

    let decision = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/ai/approvals/{}/decision", Uuid::new_v4()),
            AI_TOKEN,
            json!({
                "expectedRevision": 1,
                "decision": "approve",
                "statement": "approved"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(decision.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_cage_accepts_capacity_and_rejects_nonpositive_values() {
    let fixture = Fixture::new(None).await;
    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/cages",
            HUMAN_TOKEN,
            json!({
                "section": "Room A",
                "display_id": "A-01",
                "capacity": 9
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response_json(response).await["data"]["capacity"], json!(9));

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/cages",
            HUMAN_TOKEN,
            json!({
                "section": "Room A",
                "display_id": "A-02",
                "capacity": 0
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn transfer_route_uses_atomic_store_and_records_the_human_actor() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let mut cage = Cage::new(fixture.lab_id, "Room A", "A-01", now).unwrap();
    cage.set_capacity(2).unwrap();
    fixture
        .store
        .create_cage(&cage, &AuditContext::system(WriteSource::Migration))
        .await
        .unwrap();
    let animal = Animal::new_mouse(fixture.lab_id, "M-001", Sex::Female, now).unwrap();
    fixture
        .store
        .create_animal(&animal, &AuditContext::system(WriteSource::Migration))
        .await
        .unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/animals/transfer",
            HUMAN_TOKEN,
            json!({"animal_ids": [animal.id], "target_cage_id": cage.id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fixture
            .store
            .get_animal(animal.id)
            .await
            .unwrap()
            .current_cage_id,
        Some(cage.id)
    );
    let events = fixture.store.list_animal_events(animal.id).await.unwrap();
    let event = events.last().expect("transfer event");
    assert_eq!(event.recorded_by, Some(fixture.user_id));
    assert!(matches!(
        event.kind,
        AnimalEventKind::Transferred {
            to_cage_id: Some(id),
            ..
        } if id == cage.id
    ));

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/animals/transfer",
            HUMAN_TOKEN,
            json!({
                "animal_ids": [animal.id],
                "target_cage_id": cage.id,
                "notes": "x".repeat(2_001)
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn ordinary_api_json_has_an_explicit_one_megabyte_limit() {
    let fixture = Fixture::new(None).await;
    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/cages",
            HUMAN_TOKEN,
            json!({
                "section": "Room A",
                "display_id": "A-oversized",
                "location": "x".repeat(1024 * 1024)
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn mcp_requires_scoped_external_token_and_lists_only_fixed_tools() {
    let fixture = Fixture::new(None).await;
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let forbidden = fixture
        .app
        .clone()
        .oneshot(fixture.request(Method::POST, "/mcp", HUMAN_TOKEN, call.clone()))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(Method::POST, "/mcp", AI_TOKEN, call))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["result"]["tools"].as_array().unwrap().len(), 5);
    assert!(
        body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| !tool["name"].as_str().unwrap().contains("sql"))
    );
}

#[tokio::test]
async fn mcp_rejects_project_ids_from_other_labs_for_every_project_scoped_tool() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let other_lab = Lab::new("Other MCP lab", now).unwrap();
    fixture.store.create_lab(&other_lab, &audit).await.unwrap();
    let other_project = Project::new(other_lab.id, "Other MCP project", now).unwrap();
    fixture
        .store
        .create_project(&other_project, &audit)
        .await
        .unwrap();
    let animal = Animal::new_mouse(fixture.lab_id, "MCP-LOCAL", Sex::Female, now).unwrap();
    fixture.store.create_animal(&animal, &audit).await.unwrap();

    let calls = [
        ("animal.search", json!({"project_id": other_project.id})),
        (
            "animal.timeline",
            json!({"animal_id": animal.id, "project_id": other_project.id}),
        ),
        ("experiment.status", json!({"project_id": other_project.id})),
        ("measurement.query", json!({"project_id": other_project.id})),
        ("sample.inventory", json!({"project_id": other_project.id})),
    ];

    for (index, (name, arguments)) in calls.into_iter().enumerate() {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(
                Method::POST,
                "/mcp",
                AI_TOKEN,
                json!({
                    "jsonrpc": "2.0",
                    "id": index,
                    "method": "tools/call",
                    "params": {"name": name, "arguments": arguments}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], json!(-32004), "{name}");
        assert!(body.get("result").is_none(), "{name}");
    }
}

#[tokio::test]
async fn spa_fallback_never_replaces_api_json_errors() {
    let directory = std::env::temp_dir().join(format!("muriarc-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("index.html"), "<html>MuriArc SPA</html>").unwrap();
    let fixture = Fixture::new(Some(directory.clone())).await;

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(response).await["error"]["code"], "not_found");

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/animals/deep-link")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("MuriArc SPA"));

    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn research_routes_publish_templates_with_revision_checks_and_audit() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/experiment-template-versions",
            HUMAN_TOKEN,
            json!({
                "project_id": fixture.project_id,
                "template_key": "body-weight",
                "version": 1,
                "name": "Body weight",
                "description": "Daily weighing",
                "fields": [{
                    "key": "body_weight",
                    "label": "Body weight",
                    "value_type": "number",
                    "unit": "g",
                    "required": true,
                    "categories": [],
                    "minimum": 0.0,
                    "maximum": 100.0,
                    "display_order": 0,
                    "ai_writable": true
                }]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let template = response_json(response).await;
    let template_id: Uuid = serde_json::from_value(template["data"]["id"].clone()).unwrap();
    let template_revision = template["data"]["meta"]["revision"].as_i64().unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/experiment-template-versions/{template_id}/publish"),
            HUMAN_TOKEN,
            json!({
                "project_id": fixture.project_id,
                "expected_revision": template_revision
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let published = response_json(response).await;
    assert_eq!(published["data"]["status"], "published");
    assert_eq!(published["data"]["published_by"], json!(fixture.user_id));
    assert_eq!(
        published["data"]["meta"]["revision"],
        json!(template_revision + 1)
    );

    let stale = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/experiment-template-versions/{template_id}/publish"),
            HUMAN_TOKEN,
            json!({
                "project_id": fixture.project_id,
                "expected_revision": template_revision
            }),
        ))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    let template_audits = fixture
        .store
        .list_audit_entries(&muriarc_core::AuditFilter {
            lab_id: fixture.lab_id,
            project_id: None,
            entity_id: Some(template_id),
        })
        .await
        .unwrap();
    assert_eq!(template_audits.len(), 2);
    assert_eq!(
        template_audits[1].action,
        muriarc_core::AuditAction::Publish
    );
    assert_eq!(template_audits[1].actor.user_id, Some(fixture.user_id));

    let experiment = Experiment::new(fixture.lab_id, fixture.project_id, "Study", now).unwrap();
    fixture
        .store
        .create_experiment(&experiment, &audit)
        .await
        .unwrap();
    let animal = Animal::new_mouse(fixture.lab_id, "REST-001", Sex::Female, now).unwrap();
    fixture.store.create_animal(&animal, &audit).await.unwrap();
    fixture
        .store
        .assign_animals_to_project(
            &[ProjectAnimalAssignment::new(
                fixture.lab_id,
                fixture.project_id,
                animal.id,
                Some(fixture.user_id),
                Some("REST research test assignment".to_owned()),
                now,
            )],
            &audit,
        )
        .await
        .unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/cohorts",
            HUMAN_TOKEN,
            json!({
                "experiment_id": experiment.id,
                "name": "Control",
                "description": "Vehicle"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let cohort_id: Uuid =
        serde_json::from_value(response_json(response).await["data"]["id"].clone()).unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/participations",
            HUMAN_TOKEN,
            json!({
                "experiment_id": experiment.id,
                "animal_id": animal.id,
                "cohort_id": cohort_id
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let participation_body = response_json(response).await;
    let participation_id: Uuid =
        serde_json::from_value(participation_body["data"]["id"].clone()).unwrap();
    let participation_revision = participation_body["data"]["meta"]["revision"]
        .as_i64()
        .unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/procedures",
            HUMAN_TOKEN,
            json!({
                "experiment_id": experiment.id,
                "animal_id": animal.id,
                "name": "Weigh",
                "performed_at": now,
                "status": "completed",
                "details": {"day": 0}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    for uri in [
        format!(
            "/api/v1/participations?project_id={}&experiment_id={}",
            fixture.project_id, experiment.id
        ),
        format!("/api/v1/cohorts?experiment_id={}", experiment.id),
        format!("/api/v1/procedures?experiment_id={}", experiment.id),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(Method::GET, &uri, PROJECT_TOKEN, json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert_eq!(response_json(response).await["count"], json!(1));
    }

    let forbidden = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/cohorts",
            PROJECT_TOKEN,
            json!({"experiment_id": experiment.id, "name": "Forbidden"}),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    for uri in [
        format!("/api/v1/participations/{participation_id}/complete"),
        format!("/api/v1/experiments/{}/complete", experiment.id),
    ] {
        let forbidden = fixture
            .app
            .clone()
            .oneshot(fixture.request(
                Method::POST,
                &uri,
                PROJECT_TOKEN,
                json!({"expected_revision": 1}),
            ))
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN, "{uri}");
    }

    let completed_participation = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/participations/{participation_id}/complete"),
            HUMAN_TOKEN,
            json!({"expected_revision": participation_revision}),
        ))
        .await
        .unwrap();
    assert_eq!(completed_participation.status(), StatusCode::OK);
    let completed_participation = response_json(completed_participation).await;
    assert_eq!(completed_participation["data"]["status"], "completed");
    assert_eq!(
        completed_participation["data"]["meta"]["revision"],
        json!(participation_revision + 1)
    );
    assert_eq!(
        fixture
            .store
            .get_animal(animal.id)
            .await
            .unwrap()
            .current_status,
        muriarc_core::AnimalStatus::Alive
    );

    let stale_participation = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/participations/{participation_id}/withdraw"),
            HUMAN_TOKEN,
            json!({"expected_revision": participation_revision}),
        ))
        .await
        .unwrap();
    assert_eq!(stale_participation.status(), StatusCode::CONFLICT);

    let completed_experiment = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/experiments/{}/complete", experiment.id),
            HUMAN_TOKEN,
            json!({"expected_revision": experiment.meta.revision}),
        ))
        .await
        .unwrap();
    assert_eq!(completed_experiment.status(), StatusCode::OK);
    assert_eq!(
        response_json(completed_experiment).await["data"]["status"],
        "completed"
    );

    let stale_experiment = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/experiments/{}/cancel", experiment.id),
            HUMAN_TOKEN,
            json!({"expected_revision": experiment.meta.revision}),
        ))
        .await
        .unwrap();
    assert_eq!(stale_experiment.status(), StatusCode::CONFLICT);

    let audit_entries = fixture
        .store
        .list_audit_entries(&muriarc_core::AuditFilter {
            lab_id: fixture.lab_id,
            project_id: Some(fixture.project_id),
            entity_id: Some(cohort_id),
        })
        .await
        .unwrap();
    assert_eq!(audit_entries.len(), 1);
    assert_eq!(audit_entries[0].actor.user_id, Some(fixture.user_id));
    assert_eq!(audit_entries[0].source, WriteSource::Web);
}

#[tokio::test]
async fn genetics_and_attachment_binary_routes_validate_content_and_audit_writes() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let parent = Animal::new_mouse(fixture.lab_id, "P-REST", Sex::Male, now).unwrap();
    let child = Animal::new_mouse(fixture.lab_id, "C-REST", Sex::Female, now).unwrap();
    fixture.store.create_animal(&parent, &audit).await.unwrap();
    fixture.store.create_animal(&child, &audit).await.unwrap();
    let experiment = Experiment::new(
        fixture.lab_id,
        fixture.project_id,
        "Genetics scope experiment",
        now,
    )
    .unwrap();
    fixture
        .store
        .create_experiment(&experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_participation(&Participation::enroll(experiment.id, child.id, now), &audit)
        .await
        .unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/gene-loci",
            HUMAN_TOKEN,
            json!({
                "project_id": fixture.project_id,
                "symbol": "GeneA",
                "description": "Mechanosensor"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let locus_id: Uuid =
        serde_json::from_value(response_json(response).await["data"]["id"].clone()).unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/alleles",
            HUMAN_TOKEN,
            json!({
                "project_id": fixture.project_id,
                "locus_id": locus_id,
                "symbol": "flox",
                "is_wild_type": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let allele_id: Uuid =
        serde_json::from_value(response_json(response).await["data"]["id"].clone()).unwrap();

    let genotype_payload = json!({
        "project_id": fixture.project_id,
        "animal_id": child.id,
        "locus_id": locus_id,
        "allele_1_id": allele_id,
        "allele_2_id": allele_id,
        "assessed_at": now
    });
    for token in [PROJECT_TOKEN, PROJECT_EDITOR_TOKEN] {
        let denied = fixture
            .app
            .clone()
            .oneshot(fixture.request(
                Method::POST,
                "/api/v1/genotypes",
                token,
                genotype_payload.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    }

    let hidden_project = Project::new(fixture.lab_id, "Hidden genetics project", now).unwrap();
    fixture
        .store
        .create_project(&hidden_project, &audit)
        .await
        .unwrap();
    let wrong_scope = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/genotypes",
            HUMAN_TOKEN,
            json!({
                "project_id": hidden_project.id,
                "animal_id": child.id,
                "locus_id": locus_id,
                "allele_1_id": allele_id,
                "allele_2_id": allele_id,
                "assessed_at": now
            }),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_scope.status(), StatusCode::NOT_FOUND);

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/genotypes",
            ANIMAL_MANAGER_TOKEN,
            genotype_payload,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let genotype_id: Uuid =
        serde_json::from_value(response_json(response).await["data"]["id"].clone()).unwrap();
    let genotype_events = fixture.store.list_animal_events(child.id).await.unwrap();
    assert!(genotype_events.iter().any(|event| {
        event.project_id == Some(fixture.project_id)
            && matches!(
                &event.kind,
                AnimalEventKind::Genotyped { genotype_ids }
                    if genotype_ids == &vec![genotype_id]
            )
    }));
    for (entity_type, entity_id, project_id) in [
        (EntityType::GeneLocus, locus_id, None),
        (EntityType::Allele, allele_id, None),
        (EntityType::Genotype, genotype_id, Some(fixture.project_id)),
    ] {
        let provenance = fixture
            .store
            .list_provenance(&ProvenanceFilter {
                lab_id: fixture.lab_id,
                project_id,
                entity_type: Some(entity_type),
                entity_id: Some(entity_id),
                source: None,
            })
            .await
            .unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(provenance[0].source, ProvenanceSource::Human);
    }

    let viewer_list = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/genotypes?animal_id={}&project_id={}",
                child.id, fixture.project_id
            ),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(viewer_list.status(), StatusCode::OK);
    assert_eq!(response_json(viewer_list).await["count"], json!(1));

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/pedigrees",
            HUMAN_TOKEN,
            json!({
                "animal_id": child.id,
                "parent_id": parent.id,
                "parent_type": "father"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    for uri in [
        format!("/api/v1/alleles?locus_id={locus_id}"),
        format!("/api/v1/genotypes?animal_id={}", child.id),
        format!("/api/v1/pedigrees?animal_id={}", child.id),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(Method::GET, &uri, HUMAN_TOKEN, json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert_eq!(response_json(response).await["count"], json!(1));
    }

    let content = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01";
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/attachments/upload?entity_type=animal&entity_id={}&file_name=photo.png&media_type=image%2Fpng",
                    child.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(content.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let attachment = response_json(response).await;
    assert!(attachment["data"].get("relative_path").is_none());
    assert_eq!(attachment["data"]["size_bytes"], json!(content.len()));
    assert_eq!(
        attachment["data"]["sha256"],
        json!(format!("{:x}", Sha256::digest(content)))
    );
    assert_eq!(attachment["data"]["version"], 1);
    let attachment_id: Uuid = serde_json::from_value(attachment["data"]["id"].clone()).unwrap();
    assert_eq!(
        attachment["data"]["content_href"],
        json!(format!("/api/v1/attachments/{attachment_id}/content"))
    );

    let stored = fixture
        .store
        .list_attachments(fixture.lab_id, "animal", child.id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].relative_path.starts_with("objects/"));
    assert!(
        fixture
            .attachment_root
            .join(&stored[0].relative_path)
            .is_file()
    );

    let download = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/attachments/{attachment_id}/content"),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        download.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        download.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(
        download.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("filename*=UTF-8''photo.png")
    );
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        content.as_slice()
    );

    // Version is server-derived; sending the same logical file creates a new,
    // no-overwrite object and a monotonically increasing metadata revision.
    let second_content = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x01";
    let second = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/attachments/upload?entity_type=animal&entity_id={}&file_name=photo.png&media_type=image%2Fpng",
                    child.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .body(Body::from(second_content.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(response_json(second).await["data"]["version"], 2);
    let versions = fixture
        .store
        .list_attachments(fixture.lab_id, "animal", child.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_ne!(versions[0].relative_path, versions[1].relative_path);

    let provenance = fixture
        .store
        .list_provenance(&ProvenanceFilter {
            lab_id: fixture.lab_id,
            entity_type: Some(EntityType::Attachment),
            entity_id: Some(attachment_id),
            ..ProvenanceFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0].source, ProvenanceSource::Human);

    // Paths, hash and size are not part of the upload contract. The old JSON
    // metadata-only endpoint is deliberately unavailable.
    let rejected = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/attachments/upload?entity_type=animal&entity_id={}&file_name=..%2Fescape.png",
                    child.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .body(Body::from("escape"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let obsolete = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/attachments",
            HUMAN_TOKEN,
            json!({
                "entity_type": "animal",
                "entity_id": child.id,
                "file_name": "forged.bin",
                "size_bytes": 1,
                "sha256": "b".repeat(64),
                "relative_path": "../../outside"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(obsolete.status(), StatusCode::METHOD_NOT_ALLOWED);

    let unknown = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/attachments/upload?entity_type=animal&entity_id={}&file_name=unknown.bin",
                    Uuid::new_v4()
                ))
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .body(Body::from("unknown"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let viewer_write = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/attachments/upload?entity_type=project&entity_id={}&project_id={}&file_name=forbidden.bin",
                    fixture.project_id, fixture.project_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {PROJECT_TOKEN}"))
                .body(Body::from("forbidden"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(viewer_write.status(), StatusCode::FORBIDDEN);

    fs::write(
        fixture.attachment_root.join(&stored[0].relative_path),
        b"tampered",
    )
    .unwrap();
    let polluted = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/attachments/{attachment_id}/content"),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(polluted.status(), StatusCode::CONFLICT);
    assert_eq!(response_json(polluted).await["error"]["code"], "conflict");

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            "/api/v1/audit?limit=500",
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let entries = response_json(response).await;
    assert!(
        entries["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["actor"]["user_id"] == json!(fixture.user_id))
    );

    let forbidden = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/audit?project_id={}", fixture.project_id),
            PROJECT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn research_handlers_hide_resources_from_other_labs() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let other_lab = Lab::new("Other lab", now).unwrap();
    fixture.store.create_lab(&other_lab, &audit).await.unwrap();
    let other_project = Project::new(other_lab.id, "Other project", now).unwrap();
    fixture
        .store
        .create_project(&other_project, &audit)
        .await
        .unwrap();
    let other_experiment = Experiment::new(other_lab.id, other_project.id, "Hidden", now).unwrap();
    fixture
        .store
        .create_experiment(&other_experiment, &audit)
        .await
        .unwrap();
    let other_animal = Animal::new_mouse(other_lab.id, "HIDDEN", Sex::Female, now).unwrap();
    fixture
        .store
        .create_animal(&other_animal, &audit)
        .await
        .unwrap();
    let other_template =
        muriarc_core::ExperimentTemplateVersion::draft(other_lab.id, "hidden", 1, "Hidden", now)
            .unwrap();
    fixture
        .store
        .create_template_version(&other_template, &audit)
        .await
        .unwrap();

    for uri in [
        format!("/api/v1/experiment-template-versions/{}", other_template.id),
        format!("/api/v1/cohorts?experiment_id={}", other_experiment.id),
        format!(
            "/api/v1/attachments?entity_type=animal&entity_id={}",
            other_animal.id
        ),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(Method::GET, &uri, HUMAN_TOKEN, json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }

    for uri in [
        format!("/api/v1/experiments?project_id={}", other_project.id),
        format!("/api/v1/measurements?project_id={}", other_project.id),
        format!("/api/v1/samples?project_id={}", other_project.id),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(Method::GET, &uri, HUMAN_TOKEN, json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "not_found", "{uri}");
        assert_eq!(body["error"]["message"], "resource was not found", "{uri}");
        let body = body.to_string();
        assert!(!body.contains(&other_lab.name), "{uri}");
        assert!(!body.contains(&other_project.name), "{uri}");
        assert!(!body.contains(&other_experiment.name), "{uri}");
    }

    let rejected = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/animals",
            HUMAN_TOKEN,
            json!({
                "display_id": "FOREIGN-PROJECT-ANIMAL",
                "sex": "female",
                "project_id": other_project.id
            }),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
    let rejected = response_json(rejected).await;
    assert_eq!(rejected["error"]["message"], "resource was not found");
    assert!(!rejected.to_string().contains(&other_project.name));

    let animals = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            "/api/v1/animals?q=FOREIGN-PROJECT-ANIMAL",
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(animals.status(), StatusCode::OK);
    assert_eq!(response_json(animals).await["count"], 0);
}

#[tokio::test]
async fn animal_import_template_transport_supports_blank_example_and_legacy_default() {
    let fixture = Fixture::new(None).await;
    let download = |query: &'static str, token: &'static str| {
        let app = fixture.app.clone();
        let request = fixture.request(
            Method::GET,
            &format!("/api/v1/data/animal-import/template?{query}"),
            token,
            json!({}),
        );
        async move { app.oneshot(request).await.unwrap() }
    };

    let legacy = download("format=csv", HUMAN_TOKEN).await;
    assert_eq!(legacy.status(), StatusCode::OK);
    assert_eq!(
        legacy.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"muriarc-animal-import.csv\""
    );
    let legacy_bytes = legacy.into_body().collect().await.unwrap().to_bytes();

    let example = download("format=csv&variant=example", HUMAN_TOKEN).await;
    assert_eq!(example.status(), StatusCode::OK);
    assert_eq!(
        example.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"muriarc-animal-import.csv\""
    );
    let example_bytes = example.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(legacy_bytes, example_bytes);
    assert_eq!(String::from_utf8_lossy(&example_bytes).lines().count(), 5);

    let blank = download("format=csv&variant=blank", HUMAN_TOKEN).await;
    assert_eq!(blank.status(), StatusCode::OK);
    assert_eq!(
        blank.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"muriarc-animal-import-blank.csv\""
    );
    let blank_bytes = blank.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&blank_bytes).lines().count(), 1);

    let legacy_xlsx = download("format=xlsx", HUMAN_TOKEN).await;
    assert_eq!(legacy_xlsx.status(), StatusCode::OK);
    assert_eq!(
        legacy_xlsx.headers()[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        legacy_xlsx.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"muriarc-animal-import.xlsx\""
    );
    let legacy_xlsx = legacy_xlsx.into_body().collect().await.unwrap().to_bytes();
    let legacy_table = read_xlsx(Cursor::new(legacy_xlsx)).unwrap();
    assert_eq!(legacy_table.rows.len(), 4);

    let blank_xlsx = download("format=xlsx&variant=blank", HUMAN_TOKEN).await;
    assert_eq!(blank_xlsx.status(), StatusCode::OK);
    assert_eq!(
        blank_xlsx.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"muriarc-animal-import-blank.xlsx\""
    );
    let blank_xlsx = blank_xlsx.into_body().collect().await.unwrap().to_bytes();
    let blank_table = read_xlsx(Cursor::new(blank_xlsx)).unwrap();
    assert!(blank_table.rows.is_empty());

    let unsupported = download("format=csv&variant=unsupported", HUMAN_TOKEN).await;
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
    let unknown_query = download("format=csv&unexpected=true", HUMAN_TOKEN).await;
    assert_eq!(unknown_query.status(), StatusCode::BAD_REQUEST);

    let animal_manager = download("format=csv&variant=blank", ANIMAL_MANAGER_TOKEN).await;
    assert_eq!(animal_manager.status(), StatusCode::OK);
    let viewer = download("format=csv&variant=blank", PROJECT_TOKEN).await;
    assert_eq!(viewer.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_editor_export_is_scoped_while_viewer_and_lab_snapshot_stay_forbidden() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);

    let visible = Animal::new_mouse(fixture.lab_id, "EXPORT-VISIBLE", Sex::Female, now).unwrap();
    let hidden = Animal::new_mouse(fixture.lab_id, "EXPORT-HIDDEN", Sex::Male, now).unwrap();
    fixture.store.create_animal(&visible, &audit).await.unwrap();
    fixture.store.create_animal(&hidden, &audit).await.unwrap();

    let visible_experiment = Experiment::new(
        fixture.lab_id,
        fixture.project_id,
        "Visible export experiment",
        now,
    )
    .unwrap();
    fixture
        .store
        .create_experiment(&visible_experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_participation(
            &Participation::enroll(visible_experiment.id, visible.id, now),
            &audit,
        )
        .await
        .unwrap();

    let hidden_project = Project::new(fixture.lab_id, "Hidden export project", now).unwrap();
    fixture
        .store
        .create_project(&hidden_project, &audit)
        .await
        .unwrap();
    let hidden_experiment = Experiment::new(
        fixture.lab_id,
        hidden_project.id,
        "Hidden export experiment",
        now,
    )
    .unwrap();
    fixture
        .store
        .create_experiment(&hidden_experiment, &audit)
        .await
        .unwrap();
    fixture
        .store
        .create_participation(
            &Participation::enroll(hidden_experiment.id, hidden.id, now),
            &audit,
        )
        .await
        .unwrap();

    let viewer_export = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/data/exports",
            PROJECT_TOKEN,
            json!({
                "format": "csv",
                "idempotency_key": "project-viewer-export",
                "project_id": fixture.project_id
            }),
        ))
        .await
        .unwrap();
    assert_eq!(viewer_export.status(), StatusCode::FORBIDDEN);

    let export = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/data/exports",
            PROJECT_EDITOR_TOKEN,
            json!({
                "format": "csv",
                "idempotency_key": "project-editor-export",
                "project_id": fixture.project_id
            }),
        ))
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::CREATED);
    let artifact = response_json(export).await["data"].clone();

    let download = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/data/artifacts/{}",
                artifact["jobId"].as_str().unwrap()
            ),
            PROJECT_EDITOR_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    let body = download.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("EXPORT-VISIBLE"));
    assert!(!body.contains("EXPORT-HIDDEN"));

    for (path, payload) in [
        (
            "/api/v1/data/exports",
            json!({ "format": "csv", "idempotency_key": "viewer-lab-export" }),
        ),
        (
            "/api/v1/data/snapshots",
            json!({ "idempotency_key": "viewer-lab-snapshot" }),
        ),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(fixture.request(Method::POST, path, PROJECT_TOKEN, payload))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn artifact_download_rejects_same_length_tampering_without_streaming_file_bytes() {
    let fixture = Fixture::new(None).await;
    let export = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/data/exports",
            HUMAN_TOKEN,
            json!({ "format": "csv", "idempotency_key": "tampered-download" }),
        ))
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::CREATED);
    let artifact = response_json(export).await["data"].clone();
    let job_id = artifact["jobId"].as_str().unwrap();
    let path = fixture
        ._data_dir
        .path()
        .join("data")
        .join("artifacts")
        .join(format!("{job_id}.bin"));
    let mut tampered = fs::read(&path).unwrap();
    assert!(!tampered.is_empty());
    tampered[0] ^= 0xff;
    fs::write(&path, &tampered).unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/data/artifacts/{job_id}"),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_ne!(body.as_ref(), tampered.as_slice());
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "internal_error");
}

#[tokio::test]
async fn server_data_transport_streams_imports_and_persists_downloadable_artifacts() {
    let fixture = Fixture::new(None).await;
    let upload = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/data/imports?file_name=animals.csv&idempotency_key=http-import-1&import_kind=animal")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("display_id,sex\nHTTP-001,female\n"))
                .unwrap(),
        )
        .await
        .unwrap();
    let upload_status = upload.status();
    let upload_body = response_json(upload).await;
    assert_eq!(upload_status, StatusCode::CREATED, "{upload_body}");
    let preview = upload_body["data"].clone();
    assert_eq!(preview["importKind"], json!("animal"));
    assert_eq!(preview["canConfirm"], json!(true));

    let retry = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/data/imports?file_name=animals.csv&idempotency_key=http-import-1&import_kind=animal")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("display_id,sex\nHTTP-001,female\n"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let changed_retry = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/data/imports?file_name=animals.csv&idempotency_key=http-import-1&import_kind=animal")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("display_id,sex\nHTTP-CHANGED,male\n"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changed_retry.status(), StatusCode::CONFLICT);

    let confirmation = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!(
                "/api/v1/data/imports/{}/confirm",
                preview["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({ "preview_hash": preview["previewHash"] }),
        ))
        .await
        .unwrap();
    assert_eq!(confirmation.status(), StatusCode::OK);
    let receipt = response_json(confirmation).await;
    assert_eq!(receipt["data"]["counts"]["animals"], json!(1));

    let export = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/data/exports",
            HUMAN_TOKEN,
            json!({ "format": "csv", "idempotency_key": "http-export-1" }),
        ))
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::CREATED);
    let artifact = response_json(export).await["data"].clone();
    assert_eq!(artifact["kind"], json!("export"));
    assert_eq!(
        artifact["downloadUrl"],
        json!(format!(
            "/data/artifacts/{}",
            artifact["jobId"].as_str().unwrap()
        ))
    );

    let download = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/data/artifacts/{}",
                artifact["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.headers()[header::CONTENT_TYPE],
        "text/csv; charset=utf-8"
    );
    let bytes = download.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("HTTP-001"));

    let snapshot = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            "/api/v1/data/snapshots",
            HUMAN_TOKEN,
            json!({ "idempotency_key": "http-snapshot-1" }),
        ))
        .await
        .unwrap();
    assert_eq!(snapshot.status(), StatusCode::CREATED);
    let snapshot = response_json(snapshot).await["data"].clone();
    assert_eq!(snapshot["kind"], "snapshot");
    let snapshot_download = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/data/artifacts/{}",
                snapshot["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(snapshot_download.status(), StatusCode::OK);
    assert_eq!(
        snapshot_download.headers()[header::CONTENT_TYPE],
        "application/vnd.muriarc.snapshot+zip"
    );

    let hidden = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!(
                "/api/v1/data/artifacts/{}",
                artifact["jobId"].as_str().unwrap()
            ),
            OTHER_ADMIN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let cancel_preview = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/data/imports?file_name=cancel.csv&idempotency_key=http-cancel-import&import_kind=animal")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("display_id\nHTTP-CANCEL\n"))
                .unwrap(),
        )
        .await
        .unwrap();
    let cancel_preview = response_json(cancel_preview).await["data"].clone();
    let cancel = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!(
                "/api/v1/data/imports/{}/cancel",
                cancel_preview["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancelled_confirmation = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!(
                "/api/v1/data/imports/{}/confirm",
                cancel_preview["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({ "preview_hash": cancel_preview["previewHash"] }),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled_confirmation.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn server_import_stream_enforces_its_independent_32_mib_limit() {
    let fixture = Fixture::new(None).await;
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/data/imports?file_name=animals.csv&idempotency_key=oversized-import&import_kind=animal")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(vec![b'x'; 32 * 1024 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["error"]["code"], "upload_too_large");
}

#[tokio::test]
async fn server_measurement_import_uses_experiment_project_and_creates_drafts() {
    let fixture = Fixture::new(None).await;
    let now = chrono::Utc::now();
    let audit = AuditContext {
        actor: Actor::human(fixture.user_id, "Animal manager"),
        source: WriteSource::Web,
        request_id: Some("measurement-http-setup".to_owned()),
        reason: Some("measurement HTTP integration setup".to_owned()),
    };
    let mut template = ExperimentTemplateVersion::draft(
        fixture.lab_id,
        "http-measurements",
        1,
        "HTTP measurements",
        now,
    )
    .unwrap();
    template
        .replace_fields(
            vec![TemplateField {
                key: "body_weight".to_owned(),
                label: "Body weight".to_owned(),
                value_type: FieldValueType::Number,
                unit: Some("g".to_owned()),
                required: true,
                categories: Vec::new(),
                minimum: Some(0.0),
                maximum: None,
                display_order: 0,
                ai_writable: true,
            }],
            now,
        )
        .unwrap();
    fixture
        .store
        .create_template_version(&template, &audit)
        .await
        .unwrap();
    let published = fixture
        .store
        .publish_template_version(
            template.id,
            template.meta.revision,
            fixture.user_id,
            now,
            &audit,
        )
        .await
        .unwrap();
    let mut experiment =
        Experiment::new(fixture.lab_id, fixture.project_id, "HTTP-DEMO-001", now).unwrap();
    experiment.template_version_id = Some(published.id);
    fixture
        .store
        .create_experiment(&experiment, &audit)
        .await
        .unwrap();
    let animal = Animal::new_mouse(fixture.lab_id, "HTTP-M-01", Sex::Female, now).unwrap();
    fixture.store.create_animal(&animal, &audit).await.unwrap();
    fixture
        .store
        .create_participation(
            &Participation::enroll(experiment.id, animal.id, now),
            &audit,
        )
        .await
        .unwrap();

    let upload = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/v1/data/imports?file_name=measurements.csv&idempotency_key=http-measurement-import&import_kind=measurement&experiment_id={}",
                    experiment.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(
                    "display_id,measurement_key,value_type,value,unit,measured_at\nHTTP-M-01,body_weight,number,23.4,g,2026-07-19T08:00:00Z\n",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = upload.status();
    let preview = response_json(upload).await["data"].clone();
    assert_eq!(status, StatusCode::CREATED, "{preview}");
    assert_eq!(preview["importKind"], "measurement");
    assert_eq!(preview["experimentId"], json!(experiment.id));
    assert_eq!(preview["canConfirm"], true);

    let confirmed = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!(
                "/api/v1/data/imports/{}/confirm",
                preview["jobId"].as_str().unwrap()
            ),
            HUMAN_TOKEN,
            json!({ "preview_hash": preview["previewHash"] }),
        ))
        .await
        .unwrap();
    assert_eq!(confirmed.status(), StatusCode::OK);
    let receipt = response_json(confirmed).await;
    assert_eq!(receipt["data"]["counts"]["measurements"], 1);
    let measurements = fixture
        .store
        .list_measurements(&MeasurementFilter {
            project_id: fixture.project_id,
            experiment_id: Some(experiment.id),
            animal_id: Some(animal.id),
        })
        .await
        .unwrap();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].status, RecordStatus::Draft);
}

#[tokio::test]
async fn private_ai_images_are_owner_scoped_and_admin_bearer_requires_a_view_session() {
    let fixture = Fixture::new(None).await;
    let content = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01";
    let upload = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/ai/images/upload?file_name=private.png&media_type=image%2Fpng")
                .header(header::AUTHORIZATION, format!("Bearer {HUMAN_TOKEN}"))
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(content.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::CREATED);
    let uploaded = response_json(upload).await;
    let image_id: Uuid = serde_json::from_value(uploaded["data"]["image"]["id"].clone()).unwrap();

    let own_list = fixture
        .app
        .clone()
        .oneshot(fixture.request(Method::GET, "/api/v1/ai/images", HUMAN_TOKEN, json!({})))
        .await
        .unwrap();
    assert_eq!(own_list.status(), StatusCode::OK);
    assert_eq!(response_json(own_list).await["count"], 1);

    let other_list = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            "/api/v1/ai/images",
            PROJECT_EDITOR_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(other_list.status(), StatusCode::OK);
    assert_eq!(response_json(other_list).await["count"], 0);

    let owner_content = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/ai/images/{image_id}/content"),
            HUMAN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(owner_content.status(), StatusCode::OK);
    assert_eq!(
        owner_content
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        content.as_slice()
    );

    let admin_without_session = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            &format!("/api/v1/ai/images/{image_id}/content"),
            OTHER_ADMIN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(admin_without_session.status(), StatusCode::FORBIDDEN);

    let enter_without_session = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::POST,
            &format!("/api/v1/admin/ai/images/users/{}/enter", fixture.user_id),
            OTHER_ADMIN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(enter_without_session.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(enter_without_session).await["error"]["code"],
        "session_required"
    );

    let stats = fixture
        .app
        .clone()
        .oneshot(fixture.request(
            Method::GET,
            "/api/v1/admin/ai/images/stats",
            OTHER_ADMIN_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(stats.status(), StatusCode::OK);
    assert_eq!(response_json(stats).await["count"], 1);
}
