#[cfg(feature = "postgres")]
mod admin_users;
mod ai_api;
mod ai_images;
mod ai_sources;
mod animal_details;
mod animals;
mod attachment_files;
mod attachments;
mod audit;
mod auth_api;
mod breeding;
mod cages;
mod data_api;
#[cfg(test)]
mod data_remap_tests;
mod experiments;
mod genetics;
mod genotyping_batches;
mod jobs_api;
mod library;
mod measurements;
mod observations;
mod operations;
mod projects;
mod research;
mod samples;
mod scope;
#[cfg(feature = "postgres")]
mod technical_logs;
#[cfg(test)]
mod tests;
mod validation;

use std::path::PathBuf;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, FromRequest, FromRequestParts, Path, Query, Request, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{StatusCode, request::Parts},
    middleware,
    response::IntoResponse,
    routing::get,
};
use muriarc_application::ApplicationResult;
use muriarc_core::{CompatibilityReport, Permission, StoreResult};
use serde::{Serialize, de::DeserializeOwned};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::{
    ApiError, AppState, AuthPrincipal, RequestMetadata,
    auth::{authenticate_request, require_password_ready},
    jobs::JobRepositoryError,
    mcp,
};

const MAX_API_JSON_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
pub(super) struct ItemResponse<T> {
    data: T,
    request_id: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CollectionResponse<T> {
    data: Vec<T>,
    count: usize,
    request_id: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    service: &'static str,
    version: &'static str,
    status: &'static str,
    database: &'static str,
    compatibility: &'static str,
    storage: &'static str,
    ui: &'static str,
}

#[derive(Debug, Serialize)]
struct CompatibilityHealthResponse {
    status: &'static str,
    report: Option<CompatibilityReport>,
    error: Option<String>,
}

pub fn api_router(state: AppState) -> Router {
    base_router(state).fallback(fallback)
}

/// Full shared-server router. API and MCP paths retain their JSON semantics;
/// all other paths may fall back to the Vue entry point for client-side routes.
pub fn application_router(state: AppState, ui_dir: Option<PathBuf>) -> Router {
    let router = base_router(state);
    match ui_dir {
        Some(ui_dir) => {
            let index = ServeFile::new(ui_dir.join("index.html"));
            router.fallback_service(ServeDir::new(ui_dir).fallback(index))
        }
        None => router.fallback(fallback),
    }
}

fn base_router(state: AppState) -> Router {
    let public = auth_api::public_router().layer(DefaultBodyLimit::max(MAX_API_JSON_BYTES));
    let ready_json = Router::new()
        .merge(ai_api::router())
        .merge(ai_images::router())
        .merge(ai_sources::router())
        .merge(auth_api::protected_router())
        .merge(audit::router())
        .merge(animals::router())
        .merge(animal_details::router())
        .merge(breeding::router())
        .merge(cages::router())
        .merge(projects::router())
        .merge(experiments::router())
        .merge(genetics::router())
        .merge(genotyping_batches::router())
        .merge(research::router())
        .merge(measurements::router())
        .merge(observations::router())
        .merge(operations::router())
        .merge(samples::router())
        .merge(jobs_api::router())
        .merge(library::router());
    #[cfg(feature = "postgres")]
    let ready_json = ready_json
        .merge(admin_users::router())
        .merge(technical_logs::router());
    let ready = Router::new()
        .merge(ready_json.layer(DefaultBodyLimit::max(MAX_API_JSON_BYTES)))
        .merge(attachments::router())
        .merge(data_api::router())
        .route_layer(middleware::from_fn(require_password_ready));
    let protected = Router::new()
        .merge(auth_api::credential_router().layer(DefaultBodyLimit::max(MAX_API_JSON_BYTES)))
        .merge(ready)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_request,
        ));

    let api_v1 = Router::new()
        .merge(public)
        .merge(protected)
        // The nested fallback prevents an unknown API path from being served
        // index.html by the SPA fallback.
        .fallback(fallback);
    let api = Router::new()
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/api/v1/health", get(health))
        .route("/api/v1/livez", get(livez))
        .route("/api/v1/readyz", get(readyz))
        .route("/api/v1/compatibility", get(compatibility_health))
        .nest("/api/v1", api_v1);

    api.merge(mcp::router(state.clone())).with_state(state)
}

async fn fallback() -> ApiError {
    ApiError::not_found("API route was not found")
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match readiness(&state).await {
        Ok(_) => (
            StatusCode::OK,
            Json(HealthResponse {
                service: "muriarc-server",
                version: env!("CARGO_PKG_VERSION"),
                status: "ok",
                database: "ok",
                compatibility: "ok",
                storage: "ok",
                ui: "ok",
            }),
        ),
        Err(error) => {
            tracing::error!(error = %error, "health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    service: "muriarc-server",
                    version: env!("CARGO_PKG_VERSION"),
                    status: "degraded",
                    database: "unavailable",
                    compatibility: "unavailable",
                    storage: "unavailable",
                    ui: "unavailable",
                }),
            )
        }
    }
}

async fn livez() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match readiness(&state).await {
        Ok(_) => (StatusCode::OK, "ready\n"),
        Err(error) => {
            tracing::warn!(error = %error, "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready\n")
        }
    }
}

async fn compatibility_health(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.compatibility_report().await {
        Ok(report) if report.is_compatible() => (
            StatusCode::OK,
            Json(CompatibilityHealthResponse {
                status: "compatible",
                report: Some(report),
                error: None,
            }),
        ),
        Ok(report) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(CompatibilityHealthResponse {
                status: "incompatible",
                report: Some(report),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(CompatibilityHealthResponse {
                status: "unavailable",
                report: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn readiness(state: &AppState) -> Result<CompatibilityReport, String> {
    if !state.runtime_compatibility_verified {
        return Err("runtime compatibility preflight was not completed".to_owned());
    }
    state
        .store
        .health_check()
        .await
        .map_err(|error| error.to_string())?;
    let report = state
        .store
        .compatibility_report()
        .await
        .map_err(|error| error.to_string())?;
    report.require_compatible()?;
    if state.data_files.is_none() {
        return Err("data storage is not configured".to_owned());
    }
    let attachment_root = state
        .attachment_root
        .as_ref()
        .ok_or_else(|| "attachment root is not configured".to_owned())?;
    if !tokio::fs::metadata(attachment_root.as_ref())
        .await
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err("attachment root is not a directory".to_owned());
    }
    let ui_root = state
        .ui_root
        .as_ref()
        .ok_or_else(|| "UI asset root is not configured".to_owned())?;
    if !tokio::fs::metadata(ui_root.join("index.html"))
        .await
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("UI index asset is missing".to_owned());
    }
    Ok(report)
}

pub(super) fn authorize(
    principal: &AuthPrincipal,
    permission: Permission,
    project_id: Option<Uuid>,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    principal
        .ensure(permission, project_id)
        .map_err(|error| error.with_request_id(metadata.request_id.clone()))
}

pub(super) fn ensure_lab(
    entity_lab_id: Uuid,
    principal: &AuthPrincipal,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if entity_lab_id == principal.lab_id {
        Ok(())
    } else {
        Err(ApiError::not_found("resource was not found")
            .with_request_id(metadata.request_id.clone()))
    }
}

pub(super) async fn store<T>(
    result: impl std::future::Future<Output = StoreResult<T>>,
    metadata: &RequestMetadata,
) -> Result<T, ApiError> {
    result
        .await
        .map_err(ApiError::from_store)
        .map_err(|error| error.with_request_id(metadata.request_id.clone()))
}

pub(super) async fn application<T>(
    result: impl std::future::Future<Output = ApplicationResult<T>>,
    metadata: &RequestMetadata,
) -> Result<T, ApiError> {
    result
        .await
        .map_err(ApiError::from)
        .map_err(|error| error.with_request_id(metadata.request_id.clone()))
}

pub(super) fn job_error(error: JobRepositoryError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        JobRepositoryError::IdempotencyConflict => ApiError::conflict(error.to_string()),
        JobRepositoryError::NotFound(id) => ApiError::not_found(format!("job {id} was not found")),
        JobRepositoryError::Unavailable => ApiError::internal(),
    };
    error.with_request_id(metadata.request_id.clone())
}

pub(super) fn item<T>(data: T, metadata: &RequestMetadata) -> Json<ItemResponse<T>> {
    Json(ItemResponse {
        data,
        request_id: metadata.request_id.clone(),
    })
}

pub(super) fn collection<T>(
    data: Vec<T>,
    metadata: &RequestMetadata,
) -> Json<CollectionResponse<T>> {
    let count = data.len();
    Json(CollectionResponse {
        data,
        count,
        request_id: metadata.request_id.clone(),
    })
}

pub(super) struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request
            .extensions()
            .get::<RequestMetadata>()
            .map(|metadata| metadata.request_id.clone());
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                let status = error.status();
                extraction_error(status, "invalid_json", error, request_id)
            })
    }
}

pub(super) struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
    Query<T>: FromRequestParts<S, Rejection = QueryRejection>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .extensions
            .get::<RequestMetadata>()
            .map(|metadata| metadata.request_id.clone());
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|error| {
                extraction_error(StatusCode::BAD_REQUEST, "invalid_query", error, request_id)
            })
    }
}

pub(super) struct ApiPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
    Path<T>: FromRequestParts<S, Rejection = PathRejection>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .extensions
            .get::<RequestMetadata>()
            .map(|metadata| metadata.request_id.clone());
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|error| {
                extraction_error(StatusCode::BAD_REQUEST, "invalid_path", error, request_id)
            })
    }
}

fn extraction_error(
    status: StatusCode,
    code: &'static str,
    error: impl std::fmt::Display,
    request_id: Option<String>,
) -> ApiError {
    let mut api_error = ApiError::new(status, code, error.to_string());
    if let Some(request_id) = request_id {
        api_error = api_error.with_request_id(request_id);
    }
    api_error
}
