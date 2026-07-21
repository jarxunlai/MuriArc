use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateMeasurementCommand, SignMeasurementCommand, create_measurement, sign_measurement,
};
use muriarc_core::{Measurement, MeasurementFilter, MeasurementValue, Permission};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiJson, ApiPath, ApiQuery, CollectionResponse, ItemResponse, application, authorize,
    collection, ensure_lab, item, scope, store,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/measurements", get(list).post(create))
        .route("/measurements/{id}", get(get_one))
        .route("/measurements/{id}/sign", axum::routing::post(sign))
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
) -> Result<Json<CollectionResponse<Measurement>>, ApiError> {
    scope::project_with_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadMeasurement,
    )
    .await?;
    let measurements = store(
        state.store.list_measurements(&MeasurementFilter {
            project_id: query.project_id,
            experiment_id: query.experiment_id,
            animal_id: query.animal_id,
        }),
        &metadata,
    )
    .await?;
    Ok(collection(measurements, &metadata))
}

async fn get_one(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemResponse<Measurement>>, ApiError> {
    let measurement = store(state.store.get_measurement(id), &metadata).await?;
    ensure_lab(measurement.lab_id, &principal, &metadata)?;
    authorize(
        &principal,
        Permission::ReadMeasurement,
        Some(measurement.project_id),
        &metadata,
    )?;
    Ok(item(measurement, &metadata))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    project_id: Uuid,
    experiment_id: Option<Uuid>,
    animal_id: Uuid,
    key: String,
    label: String,
    value: MeasurementValue,
    unit: Option<String>,
    measured_at: Option<DateTime<Utc>>,
}

async fn create(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiJson(payload): ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<ItemResponse<Measurement>>), ApiError> {
    authorize(
        &principal,
        Permission::WriteMeasurementDraft,
        Some(payload.project_id),
        &metadata,
    )?;
    let now = Utc::now();
    let audit = principal.audit_context(&metadata);
    let measurement = application(
        create_measurement(
            state.store.as_ref(),
            CreateMeasurementCommand {
                lab_id: principal.lab_id,
                project_id: payload.project_id,
                experiment_id: payload.experiment_id,
                animal_id: payload.animal_id,
                procedure_id: None,
                key: payload.key,
                label: payload.label,
                value: payload.value,
                unit: payload.unit,
                measured_at: payload.measured_at.unwrap_or(now),
                now,
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, item(measurement, &metadata)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignRequest {
    expected_revision: i64,
}

async fn sign(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<SignRequest>,
) -> Result<Json<ItemResponse<Measurement>>, ApiError> {
    let measurement = store(state.store.get_measurement(id), &metadata).await?;
    ensure_lab(measurement.lab_id, &principal, &metadata)?;
    authorize(
        &principal,
        Permission::SignMeasurement,
        Some(measurement.project_id),
        &metadata,
    )?;
    let audit = principal.audit_context(&metadata);
    let measurement = application(
        sign_measurement(
            state.store.as_ref(),
            SignMeasurementCommand {
                id,
                expected_revision: payload.expected_revision,
                signed_by: principal.user_id,
                signed_at: Utc::now(),
            },
            &audit,
        ),
        &metadata,
    )
    .await?;
    Ok(item(measurement, &metadata))
}
