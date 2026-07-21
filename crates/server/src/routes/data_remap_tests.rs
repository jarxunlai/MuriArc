use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use muriarc_core::{AuditContext, JobStatus, Lab, LabRole, MuriArcStore, User, WriteSource};
use muriarc_data::DataFiles;
use muriarc_store_sqlite::SqliteStore;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState, AuthPrincipal, StaticTokenAuthenticator, StoreJobRepository, application_router,
};

const OWNER_TOKEN: &str = "remap-owner-token-000000000000000000000000";
const OTHER_TOKEN: &str = "remap-other-token-000000000000000000000000";

struct Fixture {
    app: Router,
    store: Arc<SqliteStore>,
    files: DataFiles,
    _temp: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let store = Arc::new(SqliteStore::in_memory().await.unwrap());
        store.migrate().await.unwrap();
        let now = chrono::Utc::now();
        let audit = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Remap server lab", now).unwrap();
        store.create_lab(&lab, &audit).await.unwrap();
        let owner = User::new(lab.id, "remap-owner@example.test", "Remap owner", now).unwrap();
        let other = User::new(lab.id, "remap-other@example.test", "Other admin", now).unwrap();
        store.create_user(&owner, &audit).await.unwrap();
        store.create_user(&other, &audit).await.unwrap();
        let authenticator = StaticTokenAuthenticator::new([
            (
                OWNER_TOKEN.to_owned(),
                AuthPrincipal::human(owner.id, owner.display_name, lab.id, [LabRole::LabAdmin]),
            ),
            (
                OTHER_TOKEN.to_owned(),
                AuthPrincipal::human(other.id, other.display_name, lab.id, [LabRole::LabAdmin]),
            ),
        ])
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let files = DataFiles::new(temp.path().join("data"));
        let state = AppState::new(
            store.clone(),
            Arc::new(authenticator),
            Arc::new(StoreJobRepository::new(store.clone())),
        )
        .with_data_storage(files.clone(), temp.path().join("attachments"));
        Self {
            app: application_router(state, None),
            store,
            files,
            _temp: temp,
        }
    }

    async fn upload(&self, key: &str, source: &'static [u8]) -> (Uuid, Value) {
        let response = self
            .app
            .clone()
            .oneshot(request(
                Method::POST,
                &format!(
                    "/api/v1/data/imports?file_name=animals.csv&idempotency_key={key}&import_kind=animal"
                ),
                OWNER_TOKEN,
                Body::from(source),
                "application/octet-stream",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let value = response_json(response).await;
        let id = Uuid::parse_str(value["data"]["jobId"].as_str().unwrap()).unwrap();
        (id, value)
    }
}

#[tokio::test]
async fn remap_creates_a_new_job_cancels_the_old_and_replays_exactly() {
    let fixture = Fixture::new().await;
    let (source_id, source_preview) = fixture
        .upload(
            "server-remap-source",
            b"custom_code,gender\nM-SERVER-REMAP,F\n",
        )
        .await;
    assert_eq!(source_preview["data"]["canConfirm"], false);
    let payload = json!({
        "mapping": { "columns": {
            "display_id": "custom_code",
            "sex": "gender"
        }},
        "idempotency_key": "server-remap-replacement"
    });

    let hidden = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/remap"),
            OTHER_TOKEN,
            Body::from(payload.to_string()),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let response = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/remap"),
            OWNER_TOKEN,
            Body::from(payload.to_string()),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let remapped = response_json(response).await;
    assert_eq!(remapped["data"]["canConfirm"], true);
    let replacement_id = Uuid::parse_str(remapped["data"]["jobId"].as_str().unwrap()).unwrap();
    assert_ne!(replacement_id, source_id);
    assert_ne!(
        remapped["data"]["previewHash"],
        source_preview["data"]["previewHash"]
    );
    let old_job = fixture.store.get_job(source_id).await.unwrap();
    assert_eq!(old_job.status, JobStatus::Cancelled);
    assert!(old_job.cancellation_requested);

    let replay = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/remap"),
            OWNER_TOKEN,
            Body::from(payload.to_string()),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let replayed = response_json(replay).await;
    assert_eq!(replayed["data"]["jobId"], remapped["data"]["jobId"]);
    assert_eq!(
        replayed["data"]["previewHash"],
        remapped["data"]["previewHash"]
    );

    let mismatched = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/remap"),
            OWNER_TOKEN,
            Body::from(
                json!({
                    "mapping": { "columns": { "display_id": "gender" }},
                    "idempotency_key": "server-remap-replacement"
                })
                .to_string(),
            ),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(mismatched.status(), StatusCode::CONFLICT);

    let old_confirm = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/confirm"),
            OWNER_TOKEN,
            Body::from(
                json!({ "preview_hash": source_preview["data"]["previewHash"] }).to_string(),
            ),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(old_confirm.status(), StatusCode::CONFLICT);

    let confirmed = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{replacement_id}/confirm"),
            OWNER_TOKEN,
            Body::from(json!({ "preview_hash": remapped["data"]["previewHash"] }).to_string()),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(confirmed.status(), StatusCode::OK);
}

#[tokio::test]
async fn remap_failure_leaves_the_previous_pending_job_untouched() {
    let fixture = Fixture::new().await;
    let (source_id, _) = fixture
        .upload("server-remap-preserve", b"display_id\nM-PRESERVE\n")
        .await;
    fixture.files.clear_upload(source_id).await.unwrap();

    let response = fixture
        .app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/data/imports/{source_id}/remap"),
            OWNER_TOKEN,
            Body::from(
                json!({
                    "mapping": { "columns": { "display_id": "display_id" }},
                    "idempotency_key": "server-remap-preserve-replacement"
                })
                .to_string(),
            ),
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let preserved = fixture.store.get_job(source_id).await.unwrap();
    assert_eq!(preserved.status, JobStatus::AwaitingConfirmation);
    assert!(!preserved.cancellation_requested);
    assert!(fixture.files.read_pending_import(source_id).await.is_ok());
}

fn request(
    method: Method,
    uri: &str,
    token: &str,
    body: Body,
    content_type: &str,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
