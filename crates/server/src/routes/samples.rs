use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use muriarc_application::{CreateSampleCommand, create_sample};
use muriarc_core::{Permission, Sample, SampleFilter};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, scope, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/samples", get(list).post(create))
        .route("/samples/{id}", get(get_one))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    animal_id: Option<Uuid>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<Sample>>, ApiError> {
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadSample,
    )
    .await?;
    let samples = store(
        state.store.list_samples(&SampleFilter {
            project_id: query.project_id,
            experiment_id: query.experiment_id,
            animal_id: query.animal_id,
        }),
        &metadata,
    )
    .await?;
    Ok(collection(samples, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Sample>>, ApiError> {
    let sample = store(state.store.get_sample(id), &metadata).await?;
    ensure_lab(sample.lab_id, &principal, &metadata)?;
    authorize(
        &principal,
        Permission::ReadSample,
        Some(sample.project_id),
        &metadata,
    )?;
    Ok(item(sample, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    animal_id: Uuid,
    sample_type: String,
    quantity: Option<f64>,
    unit: Option<String>,
    location: Option<String>,
    collected_at: Option<DateTime<Utc>>,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Sample>>), ApiError> {
    authorize(
        &principal,
        Permission::WriteSample,
        Some(payload.project_id),
        &metadata,
    )?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let sample = application(
        create_sample(
            state.store.as_ref(),
            CreateSampleCommand {
                lab_id: principal.lab_id,
                project_id: payload.project_id,
                experiment_id: payload.experiment_id,
                animal_id: payload.animal_id,
                sample_type: payload.sample_type,
                quantity: payload.quantity,
                unit: payload.unit,
                location: payload.location,
                collected_at: payload.collected_at.unwrap_or(now),
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(sample, &metadata)))
}
