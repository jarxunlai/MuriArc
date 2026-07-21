use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;

use crate::{
    ApiError, AppState, AuthPrincipal, AuthenticationMethod, RequestMetadata,
    SaveTechnicalLogPolicyInput, TechnicalLogCleanupPreview, TechnicalLogError,
    TechnicalLogPolicyView,
};

use super::{ApiJson, ItemResponse, item};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/technical-logs/policy",
            get(get_policy).put(save_policy),
        )
        .route(
            "/admin/technical-logs/cleanup/preview",
            get(preview_cleanup),
        )
        .route(
            "/admin/technical-logs/cleanup",
            axum::routing::post(cleanup),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupInput {
    expected_policy_revision: i64,
    expected_eligible_rows: i64,
}

async fn get_policy(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<TechnicalLogPolicyView>>, ApiError> {
    ensure_root(&principal, authentication, &metadata)?;
    let policy = state
        .technical_logs
        .get_policy(principal.lab_id)
        .await
        .map_err(|error| log_error(error, &metadata))?;
    Ok(item(policy, &metadata))
}

async fn save_policy(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(input): ApiJson<SaveTechnicalLogPolicyInput>,
) -> Result<Json<ItemResponse<TechnicalLogPolicyView>>, ApiError> {
    ensure_root(&principal, authentication, &metadata)?;
    let policy = state
        .technical_logs
        .save_policy(principal.lab_id, input, &principal.audit_context(&metadata))
        .await
        .map_err(|error| log_error(error, &metadata))?;
    Ok(item(policy, &metadata))
}

async fn preview_cleanup(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Json<ItemResponse<TechnicalLogCleanupPreview>>, ApiError> {
    ensure_root(&principal, authentication, &metadata)?;
    let preview = state
        .technical_logs
        .preview_cleanup(principal.lab_id)
        .await
        .map_err(|error| log_error(error, &metadata))?;
    Ok(item(preview, &metadata))
}

async fn cleanup(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(input): ApiJson<CleanupInput>,
) -> Result<Json<ItemResponse<TechnicalLogCleanupPreview>>, ApiError> {
    ensure_root(&principal, authentication, &metadata)?;
    let preview = state
        .technical_logs
        .cleanup(
            principal.lab_id,
            input.expected_policy_revision,
            input.expected_eligible_rows,
            &principal.audit_context(&metadata),
        )
        .await
        .map_err(|error| log_error(error, &metadata))?;
    Ok(item(preview, &metadata))
}

fn ensure_root(
    principal: &AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: &RequestMetadata,
) -> Result<(), ApiError> {
    if principal.is_environment_root()
        && !principal.is_external_ai()
        && matches!(authentication, AuthenticationMethod::Session { .. })
    {
        Ok(())
    } else {
        Err(ApiError::forbidden().with_request_id(metadata.request_id.clone()))
    }
}

fn log_error(error: TechnicalLogError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        TechnicalLogError::Validation => ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_technical_log_policy",
            "technical log policy is invalid",
        ),
        TechnicalLogError::Conflict => ApiError::conflict(error.to_string()),
        TechnicalLogError::Unavailable => ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "technical_log_unavailable",
            "technical log storage is unavailable",
        ),
    };
    error.with_request_id(metadata.request_id.clone())
}

#[cfg(test)]
mod tests {
    use muriarc_core::LabRole;

    use super::*;

    fn metadata() -> RequestMetadata {
        RequestMetadata {
            request_id: "technical-log-route-test".to_owned(),
            reason: None,
        }
    }

    #[test]
    fn cleanup_administration_requires_root_browser_session() {
        let root = AuthPrincipal::human(
            uuid::Uuid::new_v4(),
            "Root",
            uuid::Uuid::new_v4(),
            [LabRole::LabAdmin],
        )
        .with_credential_state(false, true);
        assert!(
            ensure_root(
                &root,
                AuthenticationMethod::Session {
                    session_id: uuid::Uuid::new_v4(),
                },
                &metadata(),
            )
            .is_ok()
        );
        assert!(ensure_root(&root, AuthenticationMethod::Bearer, &metadata()).is_err());

        let lab_admin = AuthPrincipal::human(
            uuid::Uuid::new_v4(),
            "Lab admin",
            root.lab_id,
            [LabRole::LabAdmin],
        );
        assert!(
            ensure_root(
                &lab_admin,
                AuthenticationMethod::Session {
                    session_id: uuid::Uuid::new_v4(),
                },
                &metadata(),
            )
            .is_err()
        );
    }
}
