use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use muriarc_core::{LabRole, ProjectRole, UserStatus};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    AdminMutationContext, ApiError, AppState, AuthPrincipal, AuthenticationMethod,
    CreateManagedUserCommand, InitialProjectRole, ManagedUser, PostgresUserGovernance,
    RequestMetadata, SensitivePassword, UserGovernanceError,
};

use super::{ApiJson, ApiPath, CollectionResponse, ItemResponse, collection, item};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/{user_id}/status", patch(set_user_status))
        .route("/admin/users/{user_id}/profile", patch(update_user_profile))
        .route(
            "/admin/users/{user_id}/password-reset",
            post(reset_user_password),
        )
        .route(
            "/admin/users/{user_id}/lab-membership",
            post(grant_lab_role),
        )
        .route(
            "/admin/users/{user_id}/project-memberships",
            post(grant_project_role),
        )
        .route(
            "/admin/memberships/{membership_id}/lab-role",
            patch(update_lab_role),
        )
        .route(
            "/admin/memberships/{membership_id}/project-role",
            patch(update_project_role),
        )
        .route(
            "/admin/memberships/{membership_id}",
            delete(revoke_membership),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitialProjectRoleInput {
    project_id: Uuid,
    role: ProjectRole,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateUserInput {
    email: String,
    display_name: String,
    temporary_password: String,
    current_password: String,
    lab_role: Option<LabRole>,
    #[serde(default)]
    project_roles: Vec<InitialProjectRoleInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUserProfileInput {
    expected_revision: i64,
    email: String,
    display_name: String,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetUserPasswordInput {
    expected_credential_revision: i64,
    temporary_password: String,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetUserStatusInput {
    expected_revision: i64,
    status: UserStatus,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantLabRoleInput {
    expected_user_revision: i64,
    role: LabRole,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantProjectRoleInput {
    expected_user_revision: i64,
    project_id: Uuid,
    role: ProjectRole,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateLabRoleInput {
    expected_revision: i64,
    role: LabRole,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProjectRoleInput {
    expected_revision: i64,
    role: ProjectRole,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevokeMembershipInput {
    expected_revision: i64,
    current_password: String,
}

async fn list_users(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<ManagedUser>>, ApiError> {
    let users = governance(&state, &metadata)?
        .list_users(&principal, authentication)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(collection(users, &metadata))
}

async fn create_user(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiJson(input): ApiJson<CreateUserInput>,
) -> Result<(StatusCode, Json<ItemResponse<ManagedUser>>), ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let command = CreateManagedUserCommand::new(
        input.email,
        input.display_name,
        SensitivePassword::new(input.temporary_password),
        input.lab_role,
        input
            .project_roles
            .into_iter()
            .map(|assignment| InitialProjectRole {
                project_id: assignment.project_id,
                role: assignment.role,
            })
            .collect(),
    );
    let user = governance(&state, &metadata)?
        .create_user(&context, command)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok((StatusCode::CREATED, item(user, &metadata)))
}

async fn update_user_profile(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<UpdateUserProfileInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .update_user_profile(
            &context,
            user_id,
            input.expected_revision,
            input.email,
            input.display_name,
        )
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

async fn reset_user_password(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ResetUserPasswordInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .reset_user_password(
            &context,
            user_id,
            input.expected_credential_revision,
            SensitivePassword::new(input.temporary_password),
        )
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

async fn set_user_status(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<SetUserStatusInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .set_user_status(&context, user_id, input.expected_revision, input.status)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

async fn grant_lab_role(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<GrantLabRoleInput>,
) -> Result<(StatusCode, Json<ItemResponse<ManagedUser>>), ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .grant_lab_role(&context, user_id, input.expected_user_revision, input.role)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok((StatusCode::CREATED, item(user, &metadata)))
}

async fn grant_project_role(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<GrantProjectRoleInput>,
) -> Result<(StatusCode, Json<ItemResponse<ManagedUser>>), ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .grant_project_role(
            &context,
            user_id,
            input.expected_user_revision,
            input.project_id,
            input.role,
        )
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok((StatusCode::CREATED, item(user, &metadata)))
}

async fn update_lab_role(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(membership_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<UpdateLabRoleInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .update_lab_role(&context, membership_id, input.expected_revision, input.role)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

async fn update_project_role(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(membership_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<UpdateProjectRoleInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .update_project_role(&context, membership_id, input.expected_revision, input.role)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

async fn revoke_membership(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: RequestMetadata,
    ApiPath(membership_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<RevokeMembershipInput>,
) -> Result<Json<ItemResponse<ManagedUser>>, ApiError> {
    let current_password = SensitivePassword::new(input.current_password);
    let context = mutation_context(&principal, authentication, &metadata, &current_password);
    let user = governance(&state, &metadata)?
        .revoke_membership(&context, membership_id, input.expected_revision)
        .await
        .map_err(|error| governance_error(error, &metadata))?;
    Ok(item(user, &metadata))
}

fn mutation_context<'a>(
    principal: &'a AuthPrincipal,
    authentication: AuthenticationMethod,
    metadata: &'a RequestMetadata,
    current_password: &'a SensitivePassword,
) -> AdminMutationContext<'a> {
    AdminMutationContext {
        actor: principal,
        authentication,
        metadata,
        current_password,
    }
}

fn governance<'a>(
    state: &'a AppState,
    metadata: &RequestMetadata,
) -> Result<&'a PostgresUserGovernance, ApiError> {
    state
        .user_governance
        .as_deref()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "user_governance_unavailable",
                "shared user governance is not configured",
            )
        })
        .map_err(|error| error.with_request_id(metadata.request_id.clone()))
}

fn governance_error(error: UserGovernanceError, metadata: &RequestMetadata) -> ApiError {
    let error = match error {
        UserGovernanceError::SessionRequired => ApiError::new(
            StatusCode::FORBIDDEN,
            "browser_session_required",
            "a live browser session is required for account governance",
        ),
        UserGovernanceError::Forbidden => ApiError::forbidden(),
        UserGovernanceError::StepUpFailed => ApiError::new(
            StatusCode::FORBIDDEN,
            "step_up_failed",
            "current password verification failed",
        ),
        UserGovernanceError::NotFound => ApiError::not_found("resource was not found"),
        UserGovernanceError::LastActiveLabAdmin => {
            ApiError::conflict("the final active LabAdmin cannot be suspended, demoted, or revoked")
        }
        UserGovernanceError::SelfLockout => ApiError::conflict(
            "an administrator cannot remove their own active administrator access",
        ),
        UserGovernanceError::EnvironmentRootManaged => ApiError::new(
            StatusCode::CONFLICT,
            "environment_root_managed",
            "the environment root account is managed by deployment configuration",
        ),
        UserGovernanceError::LabAdminManagedByRoot => ApiError::new(
            StatusCode::FORBIDDEN,
            "lab_admin_managed_by_environment_root",
            "only the environment root can govern a LabAdmin account",
        ),
        UserGovernanceError::SelfCredentialReset => ApiError::new(
            StatusCode::CONFLICT,
            "self_password_reset_not_allowed",
            "use account security to change your own password",
        ),
        UserGovernanceError::Validation(message) => ApiError::validation(message),
        UserGovernanceError::Conflict(message) => ApiError::conflict(message),
        UserGovernanceError::Unavailable => ApiError::internal(),
    };
    error.with_request_id(metadata.request_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_inputs_are_deserialize_only_and_reject_unknown_fields() {
        let input: Result<CreateUserInput, _> = serde_json::from_value(serde_json::json!({
            "email": "user@example.org",
            "displayName": "User",
            "temporaryPassword": "temporary-password",
            "currentPassword": "current-password",
            "labRole": "animal_manager",
            "projectRoles": [],
            "stepUpVerified": true
        }));
        assert!(input.is_err());
    }

    #[test]
    fn bearer_governance_is_mapped_to_a_stable_forbidden_error() {
        let metadata = RequestMetadata {
            request_id: "admin-test-request".to_owned(),
            reason: None,
        };
        let error = governance_error(UserGovernanceError::SessionRequired, &metadata);
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }
}
