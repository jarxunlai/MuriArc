use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use chrono::{DateTime, Utc};
use muriarc_core::{LabRole, Membership, Permission, ProjectRole, RecordMeta, User, UserStatus};
use muriarc_store_postgres::PostgresStore;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AuthPrincipal, AuthenticationMethod, RequestMetadata, hash_password,
    persistent_auth::verify_password,
};

const GOVERNANCE_LOCK_ID: i64 = 5_568_604_466_432_177_474;
const MAX_INITIAL_PROJECTS: usize = 500;
const MAX_PASSWORD_BYTES: usize = 1024;

/// Password material accepted by one governance call.
///
/// It is deliberately neither `Clone` nor `Serialize`; debug output is always
/// redacted and the allocation is zeroized when dropped.
pub struct SensitivePassword(Zeroizing<String>);

impl SensitivePassword {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SensitivePassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitivePassword([REDACTED])")
    }
}

pub struct AdminMutationContext<'a> {
    pub actor: &'a AuthPrincipal,
    pub authentication: AuthenticationMethod,
    pub metadata: &'a RequestMetadata,
    pub current_password: &'a SensitivePassword,
}

impl fmt::Debug for AdminMutationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminMutationContext")
            .field("actor_user_id", &self.actor.user_id)
            .field("authentication", &self.authentication)
            .field("request_id", &self.metadata.request_id)
            .field("current_password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialProjectRole {
    pub project_id: Uuid,
    pub role: ProjectRole,
}

pub struct CreateManagedUserCommand {
    pub email: String,
    pub display_name: String,
    pub lab_role: Option<LabRole>,
    pub project_roles: Vec<InitialProjectRole>,
    temporary_password: SensitivePassword,
}

impl CreateManagedUserCommand {
    pub fn new(
        email: impl Into<String>,
        display_name: impl Into<String>,
        temporary_password: SensitivePassword,
        lab_role: Option<LabRole>,
        project_roles: Vec<InitialProjectRole>,
    ) -> Self {
        Self {
            email: email.into(),
            display_name: display_name.into(),
            lab_role,
            project_roles,
            temporary_password,
        }
    }
}

impl fmt::Debug for CreateManagedUserCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateManagedUserCommand")
            .field("email", &self.email)
            .field("display_name", &self.display_name)
            .field("lab_role", &self.lab_role)
            .field("project_roles", &self.project_roles)
            .field("temporary_password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProjectMembership {
    pub membership_id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub role: ProjectRole,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: UserStatus,
    pub revision: i64,
    pub credential_revision: i64,
    pub must_change_password: bool,
    pub is_environment_root: bool,
    pub lab_membership_id: Option<Uuid>,
    pub lab_role: Option<LabRole>,
    pub lab_membership_revision: Option<i64>,
    pub project_memberships: Vec<ManagedProjectMembership>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UserGovernanceError {
    #[error("a live browser session is required for account governance")]
    SessionRequired,
    #[error("the current user is not an active LabAdmin")]
    Forbidden,
    #[error("current password verification failed")]
    StepUpFailed,
    #[error("the requested user or membership was not found")]
    NotFound,
    #[error("the operation would remove the final active LabAdmin")]
    LastActiveLabAdmin,
    #[error("the operation would remove the final active ProjectAdmin")]
    LastActiveProjectAdmin,
    #[error("an administrator cannot remove their own active administrator access")]
    SelfLockout,
    #[error("the environment root account is managed by deployment configuration")]
    EnvironmentRootManaged,
    #[error("only the environment root can govern a LabAdmin account")]
    LabAdminManagedByRoot,
    #[error("administrators must change their own password through account security")]
    SelfCredentialReset,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("the user governance service is unavailable")]
    Unavailable,
}

struct VerifiedCredential(Zeroizing<String>);

#[derive(Debug, Clone)]
pub struct PostgresUserGovernance {
    postgres: PostgresStore,
    lab_id: Uuid,
    environment_root_user_id: Uuid,
}

impl PostgresUserGovernance {
    pub fn new(postgres: PostgresStore, lab_id: Uuid, environment_root_user_id: Uuid) -> Self {
        Self {
            postgres,
            lab_id,
            environment_root_user_id,
        }
    }

    pub fn lab_id(&self) -> Uuid {
        self.lab_id
    }

    pub async fn list_users(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
        project_id: Option<Uuid>,
    ) -> Result<Vec<ManagedUser>, UserGovernanceError> {
        if let Some(project_id) = project_id {
            self.ensure_project_admin_claim(actor, authentication, project_id)?;
            self.ensure_live_project_admin_session(actor, authentication, project_id)
                .await?;
            let mut users = self.load_managed_users(None).await?;
            users.retain(|user| !user.is_environment_root);
            for user in &mut users {
                user.lab_membership_id = None;
                user.lab_role = None;
                user.lab_membership_revision = None;
                user.credential_revision = 0;
                user.must_change_password = false;
                user.project_memberships
                    .retain(|membership| membership.project_id == project_id);
            }
            Ok(users)
        } else {
            self.ensure_admin_claim(actor, authentication)?;
            self.ensure_live_admin_session(actor, authentication)
                .await?;
            self.load_managed_users(None).await
        }
    }

    pub async fn create_user(
        &self,
        context: &AdminMutationContext<'_>,
        command: CreateManagedUserCommand,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        validate_project_assignments(command.lab_role, &command.project_roles)?;
        if command.lab_role == Some(LabRole::LabAdmin)
            && !self.actor_is_environment_root(context.actor)
        {
            return Err(UserGovernanceError::LabAdminManagedByRoot);
        }
        let now = Utc::now();
        let user = User::new(self.lab_id, command.email, command.display_name, now)
            .map_err(|error| UserGovernanceError::Validation(error.to_string()))?;
        validate_email(&user.email)?;
        let password_hash = hash_password(command.temporary_password.expose()).map_err(|_| {
            UserGovernanceError::Validation(
                "temporary password must contain at least 8 non-control characters and at most 1024 bytes"
                    .to_owned(),
            )
        })?;

        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor(&mut transaction, context, &verified)
            .await?;

        let project_names = self
            .validate_projects(&mut transaction, &command.project_roles)
            .await?;

        sqlx::query(
            "INSERT INTO users (id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, 'active', $5, $5, NULL, 1)",
        )
        .bind(user.id)
        .bind(user.lab_id)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(conflict_or_database)?;
        write_audit(
            &mut transaction,
            context,
            None,
            "user",
            user.id,
            "create",
            "auth.user.created",
            None,
            Some(to_json(&user)?),
            now,
        )
        .await?;

        sqlx::query(
            "INSERT INTO user_credentials (user_id, password_hash, created_at, password_changed_at, must_change_password, revision) VALUES ($1, $2, $3, $3, TRUE, 1)",
        )
        .bind(user.id)
        .bind(&password_hash)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(conflict_or_database)?;
        write_audit(
            &mut transaction,
            context,
            None,
            "user_credential",
            user.id,
            "create",
            "auth.credential.temporary.created",
            None,
            Some(json!({
                "algorithm": "argon2id",
                "created_at": now,
                "must_change_password": true,
                "revision": 1
            })),
            now,
        )
        .await?;

        let lab_membership = command
            .lab_role
            .map(|role| Membership::lab(self.lab_id, user.id, role, now));
        if let Some(membership) = &lab_membership {
            insert_membership(&mut transaction, membership).await?;
            write_membership_audit(&mut transaction, context, "create", None, membership, now)
                .await?;
        }

        let mut project_memberships = Vec::with_capacity(command.project_roles.len());
        for assignment in command.project_roles {
            let membership = Membership::project(
                self.lab_id,
                assignment.project_id,
                user.id,
                assignment.role,
                now,
            );
            insert_membership(&mut transaction, &membership).await?;
            write_membership_audit(&mut transaction, context, "create", None, &membership, now)
                .await?;
            project_memberships.push(ManagedProjectMembership {
                membership_id: membership.id,
                project_id: assignment.project_id,
                project_name: project_names
                    .get(&assignment.project_id)
                    .cloned()
                    .ok_or(UserGovernanceError::Unavailable)?,
                role: assignment.role,
                revision: membership.meta.revision,
            });
        }

        transaction.commit().await.map_err(database)?;
        project_memberships.sort_by(|left, right| {
            left.project_name
                .cmp(&right.project_name)
                .then(left.project_id.cmp(&right.project_id))
        });
        Ok(ManagedUser {
            id: user.id,
            email: user.email,
            display_name: user.display_name,
            status: user.status,
            revision: user.meta.revision,
            credential_revision: 1,
            must_change_password: true,
            is_environment_root: false,
            lab_membership_id: lab_membership.as_ref().map(|membership| membership.id),
            lab_role: lab_membership
                .as_ref()
                .and_then(|membership| membership.lab_role),
            lab_membership_revision: lab_membership
                .as_ref()
                .map(|membership| membership.meta.revision),
            project_memberships,
            created_at: user.meta.created_at,
            updated_at: user.meta.updated_at,
        })
    }

    pub async fn set_user_status(
        &self,
        context: &AdminMutationContext<'_>,
        user_id: Uuid,
        expected_revision: i64,
        status: UserStatus,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor(&mut transaction, context, &verified)
            .await?;
        let mut user = load_user_for_update(&mut transaction, self.lab_id, user_id).await?;
        self.ensure_target_governable(&mut transaction, context, user.id)
            .await?;
        if user.id == context.actor.user_id && status == UserStatus::Suspended {
            return Err(UserGovernanceError::SelfLockout);
        }
        ensure_revision(user.meta.revision, expected_revision, "user")?;
        if user.status == status {
            return Err(UserGovernanceError::Conflict(
                "user already has the requested status".to_owned(),
            ));
        }
        if status == UserStatus::Suspended {
            self.ensure_admin_removal_safe(&mut transaction, user.id)
                .await?;
        }
        let before = user.clone();
        match status {
            UserStatus::Active => user.reactivate(now),
            UserStatus::Suspended => user.suspend(now),
        }
        update_user_row(&mut transaction, &user, expected_revision).await?;
        write_audit(
            &mut transaction,
            context,
            None,
            "user",
            user.id,
            "update",
            "auth.user.status.updated",
            Some(to_json(&before)?),
            Some(to_json(&user)?),
            now,
        )
        .await?;
        if status == UserStatus::Suspended {
            revoke_user_authentication(&mut transaction, context, user.id, "user_suspended", now)
                .await?;
        }
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(user.id).await
    }

    pub async fn update_user_profile(
        &self,
        context: &AdminMutationContext<'_>,
        user_id: Uuid,
        expected_revision: i64,
        email: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let email = email.into().trim().to_ascii_lowercase();
        validate_email(&email)?;
        let display_name = display_name.into();
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor(&mut transaction, context, &verified)
            .await?;
        let mut user = load_user_for_update(&mut transaction, self.lab_id, user_id).await?;
        self.ensure_target_governable(&mut transaction, context, user.id)
            .await?;
        if user.id == context.actor.user_id {
            return Err(UserGovernanceError::Conflict(
                "use the account security page to update your own profile".to_owned(),
            ));
        }
        ensure_revision(user.meta.revision, expected_revision, "user")?;
        let normalized_display_name = display_name.trim();
        if user.email == email && user.display_name == normalized_display_name {
            return Err(UserGovernanceError::Conflict(
                "user profile already has the requested values".to_owned(),
            ));
        }
        let before = user.clone();
        user.rename(display_name, now)
            .map_err(|error| UserGovernanceError::Validation(error.to_string()))?;
        user.email = email;
        update_user_row(&mut transaction, &user, expected_revision).await?;
        write_audit(
            &mut transaction,
            context,
            None,
            "user",
            user.id,
            "update",
            "auth.profile.admin.updated",
            Some(json!({
                "email": before.email,
                "display_name": before.display_name,
                "revision": before.meta.revision
            })),
            Some(json!({
                "email": user.email,
                "display_name": user.display_name,
                "revision": user.meta.revision
            })),
            now,
        )
        .await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(user.id).await
    }

    pub async fn reset_user_password(
        &self,
        context: &AdminMutationContext<'_>,
        user_id: Uuid,
        expected_credential_revision: i64,
        temporary_password: SensitivePassword,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let password_hash = hash_password(temporary_password.expose()).map_err(|_| {
            UserGovernanceError::Validation(
                "temporary password must contain at least 8 non-control characters and at most 1024 bytes"
                    .to_owned(),
            )
        })?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor(&mut transaction, context, &verified)
            .await?;
        let target = load_user_for_update(&mut transaction, self.lab_id, user_id).await?;
        self.ensure_target_governable(&mut transaction, context, target.id)
            .await?;
        if user_id == context.actor.user_id {
            return Err(UserGovernanceError::SelfCredentialReset);
        }
        let credential = sqlx::query(
            "SELECT password_hash, must_change_password, revision FROM user_credentials WHERE user_id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database)?;
        let (action, before, next_credential_revision) = if let Some(credential) = credential {
            let existing_hash: String = credential.try_get("password_hash").map_err(database)?;
            let must_change_password: bool = credential
                .try_get("must_change_password")
                .map_err(database)?;
            let credential_revision: i64 = credential.try_get("revision").map_err(database)?;
            ensure_revision(
                credential_revision,
                expected_credential_revision,
                "credential",
            )?;
            if verify_password(&existing_hash, temporary_password.expose().as_bytes())
                .map_err(|_| UserGovernanceError::Unavailable)?
            {
                return Err(UserGovernanceError::Conflict(
                    "temporary password must differ from the current password".to_owned(),
                ));
            }
            let changed = sqlx::query(
                "UPDATE user_credentials SET password_hash = $2, password_changed_at = $3, must_change_password = TRUE, revision = revision + 1 WHERE user_id = $1 AND revision = $4 AND password_hash = $5",
            )
            .bind(user_id)
            .bind(&password_hash)
            .bind(now)
            .bind(expected_credential_revision)
            .bind(existing_hash)
            .execute(&mut *transaction)
            .await
            .map_err(database)?
            .rows_affected();
            if changed != 1 {
                return Err(stale("credential"));
            }
            (
                "update",
                Some(json!({
                    "must_change_password": must_change_password,
                    "revision": credential_revision
                })),
                credential_revision + 1,
            )
        } else {
            ensure_revision(0, expected_credential_revision, "credential")?;
            sqlx::query(
                "INSERT INTO user_credentials (user_id, password_hash, created_at, password_changed_at, must_change_password, revision) VALUES ($1, $2, $3, $3, TRUE, 1)",
            )
            .bind(user_id)
            .bind(&password_hash)
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(conflict_or_database)?;
            ("create", None, 1)
        };
        write_audit(
            &mut transaction,
            context,
            None,
            "user_credential",
            user_id,
            action,
            "auth.password.admin_reset",
            before,
            Some(json!({
                "must_change_password": true,
                "revision": next_credential_revision,
                "sessions_revoked": true,
                "external_tokens_revoked": true
            })),
            now,
        )
        .await?;
        revoke_user_authentication(
            &mut transaction,
            context,
            user_id,
            "administrator_password_reset",
            now,
        )
        .await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(user_id).await
    }

    pub async fn grant_lab_role(
        &self,
        context: &AdminMutationContext<'_>,
        user_id: Uuid,
        expected_user_revision: i64,
        role: LabRole,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor(&mut transaction, context, &verified)
            .await?;
        let user = load_user_for_update(&mut transaction, self.lab_id, user_id).await?;
        self.ensure_target_governable(&mut transaction, context, user.id)
            .await?;
        if role == LabRole::LabAdmin && !self.actor_is_environment_root(context.actor) {
            return Err(UserGovernanceError::LabAdminManagedByRoot);
        }
        ensure_revision(user.meta.revision, expected_user_revision, "user")?;
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND deleted_at IS NULL)",
        )
        .bind(self.lab_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database)?;
        if existing {
            return Err(UserGovernanceError::Conflict(
                "user already has an active lab role".to_owned(),
            ));
        }
        let membership = Membership::lab(self.lab_id, user_id, role, now);
        insert_membership(&mut transaction, &membership).await?;
        write_membership_audit(&mut transaction, context, "create", None, &membership, now).await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(user_id).await
    }

    pub async fn update_lab_role(
        &self,
        context: &AdminMutationContext<'_>,
        membership_id: Uuid,
        expected_revision: i64,
        role: LabRole,
    ) -> Result<ManagedUser, UserGovernanceError> {
        self.update_membership_role(context, membership_id, expected_revision, Some(role), None)
            .await
    }

    pub async fn grant_project_role(
        &self,
        context: &AdminMutationContext<'_>,
        user_id: Uuid,
        expected_user_revision: i64,
        project_id: Uuid,
        role: ProjectRole,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        self.lock_and_validate_actor_for_project(&mut transaction, context, &verified, project_id)
            .await?;
        let user = load_user_for_update(&mut transaction, self.lab_id, user_id).await?;
        if user.id == self.environment_root_user_id {
            return Err(UserGovernanceError::EnvironmentRootManaged);
        }
        ensure_revision(user.meta.revision, expected_user_revision, "user")?;
        validate_project(&mut transaction, self.lab_id, project_id).await?;
        let membership = Membership::project(self.lab_id, project_id, user_id, role, now);
        insert_membership(&mut transaction, &membership).await?;
        write_membership_audit(&mut transaction, context, "create", None, &membership, now).await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(user_id).await
    }

    pub async fn update_project_role(
        &self,
        context: &AdminMutationContext<'_>,
        membership_id: Uuid,
        expected_revision: i64,
        role: ProjectRole,
    ) -> Result<ManagedUser, UserGovernanceError> {
        self.update_membership_role(context, membership_id, expected_revision, None, Some(role))
            .await
    }

    pub async fn revoke_membership(
        &self,
        context: &AdminMutationContext<'_>,
        membership_id: Uuid,
        expected_revision: i64,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        let mut membership =
            load_membership_for_update(&mut transaction, self.lab_id, membership_id).await?;
        if let Some(project_id) = membership.project_id {
            self.lock_and_validate_actor_for_project(
                &mut transaction,
                context,
                &verified,
                project_id,
            )
            .await?;
            if membership.user_id == self.environment_root_user_id {
                return Err(UserGovernanceError::EnvironmentRootManaged);
            }
        } else {
            self.lock_and_validate_actor(&mut transaction, context, &verified)
                .await?;
            self.ensure_target_governable(&mut transaction, context, membership.user_id)
                .await?;
        }
        ensure_revision(membership.meta.revision, expected_revision, "membership")?;
        if let Some(project_id) = membership.project_id
            && membership.project_role == Some(ProjectRole::ProjectAdmin)
        {
            self.ensure_project_admin_removal_safe(
                &mut transaction,
                project_id,
                membership.user_id,
            )
            .await?;
        } else if membership.project_id.is_none() && membership.lab_role == Some(LabRole::LabAdmin)
        {
            self.ensure_admin_removal_safe(&mut transaction, membership.user_id)
                .await?;
            if membership.user_id == context.actor.user_id {
                return Err(UserGovernanceError::SelfLockout);
            }
        } else if membership.project_id.is_none() && membership.user_id == context.actor.user_id {
            return Err(UserGovernanceError::SelfLockout);
        }
        let before = membership.clone();
        membership.soft_delete(now);
        let changed = sqlx::query(
            "UPDATE memberships SET updated_at = $1, deleted_at = $1, revision = $2 WHERE id = $3 AND lab_id = $4 AND revision = $5 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(membership.meta.revision)
        .bind(membership.id)
        .bind(self.lab_id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await
        .map_err(database)?
        .rows_affected();
        if changed != 1 {
            return Err(stale("membership"));
        }
        write_membership_audit(
            &mut transaction,
            context,
            "soft_delete",
            Some(&before),
            &membership,
            now,
        )
        .await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(membership.user_id).await
    }

    async fn update_membership_role(
        &self,
        context: &AdminMutationContext<'_>,
        membership_id: Uuid,
        expected_revision: i64,
        lab_role: Option<LabRole>,
        project_role: Option<ProjectRole>,
    ) -> Result<ManagedUser, UserGovernanceError> {
        let verified = self.verify_step_up(context).await?;
        let now = Utc::now();
        let mut transaction = self.postgres.pool().begin().await.map_err(database)?;
        let mut membership =
            load_membership_for_update(&mut transaction, self.lab_id, membership_id).await?;
        if let Some(project_id) = membership.project_id {
            self.lock_and_validate_actor_for_project(
                &mut transaction,
                context,
                &verified,
                project_id,
            )
            .await?;
            if membership.user_id == self.environment_root_user_id {
                return Err(UserGovernanceError::EnvironmentRootManaged);
            }
        } else {
            self.lock_and_validate_actor(&mut transaction, context, &verified)
                .await?;
            self.ensure_target_governable(&mut transaction, context, membership.user_id)
                .await?;
        }
        ensure_revision(membership.meta.revision, expected_revision, "membership")?;
        let before = membership.clone();
        match (lab_role, project_role) {
            (Some(role), None) => {
                if role == LabRole::LabAdmin && !self.actor_is_environment_root(context.actor) {
                    return Err(UserGovernanceError::LabAdminManagedByRoot);
                }
                if membership.project_id.is_some() {
                    return Err(UserGovernanceError::Validation(
                        "project memberships cannot receive a lab role".to_owned(),
                    ));
                }
                if membership.lab_role == Some(role) {
                    return Err(UserGovernanceError::Conflict(
                        "membership already has the requested role".to_owned(),
                    ));
                }
                if membership.lab_role == Some(LabRole::LabAdmin) {
                    self.ensure_admin_removal_safe(&mut transaction, membership.user_id)
                        .await?;
                    if membership.user_id == context.actor.user_id {
                        return Err(UserGovernanceError::SelfLockout);
                    }
                }
                membership
                    .change_lab_role(role, now)
                    .map_err(|error| UserGovernanceError::Validation(error.to_string()))?;
            }
            (None, Some(role)) => {
                if membership.project_id.is_none() {
                    return Err(UserGovernanceError::Validation(
                        "lab memberships cannot receive a project role".to_owned(),
                    ));
                }
                if membership.project_role == Some(role) {
                    return Err(UserGovernanceError::Conflict(
                        "membership already has the requested role".to_owned(),
                    ));
                }
                if membership.project_role == Some(ProjectRole::ProjectAdmin)
                    && role != ProjectRole::ProjectAdmin
                {
                    self.ensure_project_admin_removal_safe(
                        &mut transaction,
                        membership
                            .project_id
                            .expect("project role has project scope"),
                        membership.user_id,
                    )
                    .await?;
                }
                membership
                    .change_project_role(role, now)
                    .map_err(|error| UserGovernanceError::Validation(error.to_string()))?;
            }
            _ => {
                return Err(UserGovernanceError::Validation(
                    "exactly one role scope must be updated".to_owned(),
                ));
            }
        }
        let changed = sqlx::query(
            "UPDATE memberships SET lab_role = $1, project_role = $2, updated_at = $3, revision = $4 WHERE id = $5 AND lab_id = $6 AND revision = $7 AND deleted_at IS NULL",
        )
        .bind(membership.lab_role.map(lab_role_name))
        .bind(membership.project_role.map(project_role_name))
        .bind(now)
        .bind(membership.meta.revision)
        .bind(membership.id)
        .bind(self.lab_id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await
        .map_err(conflict_or_database)?
        .rows_affected();
        if changed != 1 {
            return Err(stale("membership"));
        }
        write_membership_audit(
            &mut transaction,
            context,
            "update",
            Some(&before),
            &membership,
            now,
        )
        .await?;
        transaction.commit().await.map_err(database)?;
        self.load_managed_user(membership.user_id).await
    }

    async fn verify_step_up(
        &self,
        context: &AdminMutationContext<'_>,
    ) -> Result<VerifiedCredential, UserGovernanceError> {
        self.ensure_governance_claim(context.actor, context.authentication)?;
        let supplied = context.current_password.expose();
        if supplied.is_empty()
            || supplied.len() > MAX_PASSWORD_BYTES
            || supplied.chars().any(char::is_control)
        {
            return Err(UserGovernanceError::StepUpFailed);
        }
        let password_hash: Option<String> = sqlx::query_scalar(
            "SELECT c.password_hash FROM user_credentials c JOIN users u ON u.id = c.user_id WHERE u.id = $1 AND u.lab_id = $2 AND u.status = 'active' AND u.deleted_at IS NULL",
        )
        .bind(context.actor.user_id)
        .bind(self.lab_id)
        .fetch_optional(self.postgres.pool())
        .await
        .map_err(database)?;
        let password_hash = password_hash.ok_or(UserGovernanceError::StepUpFailed)?;
        let verification_hash = password_hash.clone();
        let supplied = Zeroizing::new(supplied.to_owned());
        let valid = tokio::task::spawn_blocking(move || {
            verify_password(&verification_hash, supplied.as_bytes())
        })
        .await
        .map_err(|_| UserGovernanceError::Unavailable)?
        .map_err(|_| UserGovernanceError::Unavailable)?;
        if !valid {
            return Err(UserGovernanceError::StepUpFailed);
        }
        Ok(VerifiedCredential(Zeroizing::new(password_hash)))
    }

    fn ensure_admin_claim(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
    ) -> Result<(), UserGovernanceError> {
        if !matches!(authentication, AuthenticationMethod::Session { .. }) {
            return Err(UserGovernanceError::SessionRequired);
        }
        if actor.lab_id != self.lab_id
            || actor.is_external_ai()
            || actor.must_change_password()
            || !actor.can(Permission::ManageUsers, None)
        {
            return Err(UserGovernanceError::Forbidden);
        }
        Ok(())
    }

    fn ensure_governance_claim(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
    ) -> Result<(), UserGovernanceError> {
        if !matches!(authentication, AuthenticationMethod::Session { .. }) {
            return Err(UserGovernanceError::SessionRequired);
        }
        let project_admin = actor
            .project_roles()
            .any(|(_, role)| role == ProjectRole::ProjectAdmin);
        if actor.lab_id != self.lab_id
            || actor.is_external_ai()
            || actor.must_change_password()
            || (!actor.can(Permission::ManageUsers, None) && !project_admin)
        {
            return Err(UserGovernanceError::Forbidden);
        }
        Ok(())
    }

    fn ensure_project_admin_claim(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
        project_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        self.ensure_governance_claim(actor, authentication)?;
        if actor.can(Permission::ManageUsers, None)
            || actor.can(Permission::ManageProject, Some(project_id))
        {
            Ok(())
        } else {
            Err(UserGovernanceError::Forbidden)
        }
    }

    async fn ensure_live_project_admin_session(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
        project_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        let AuthenticationMethod::Session { session_id } = authentication else {
            return Err(UserGovernanceError::SessionRequired);
        };
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM auth_sessions s JOIN users u ON u.id = s.user_id JOIN memberships m ON m.user_id = u.id AND m.lab_id = u.lab_id AND ((m.project_id IS NULL AND m.lab_role = 'lab_admin') OR (m.project_id = $4 AND m.project_role = 'project_admin')) AND m.deleted_at IS NULL WHERE s.id = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.lab_id = $3 AND u.status = 'active' AND u.deleted_at IS NULL)",
        )
        .bind(session_id)
        .bind(actor.user_id)
        .bind(self.lab_id)
        .bind(project_id)
        .fetch_one(self.postgres.pool())
        .await
        .map_err(database)?;
        if valid {
            Ok(())
        } else {
            Err(UserGovernanceError::Forbidden)
        }
    }

    async fn ensure_live_admin_session(
        &self,
        actor: &AuthPrincipal,
        authentication: AuthenticationMethod,
    ) -> Result<(), UserGovernanceError> {
        let AuthenticationMethod::Session { session_id } = authentication else {
            return Err(UserGovernanceError::SessionRequired);
        };
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM auth_sessions s JOIN users u ON u.id = s.user_id JOIN memberships m ON m.user_id = u.id AND m.lab_id = u.lab_id AND m.project_id IS NULL AND m.lab_role = 'lab_admin' AND m.deleted_at IS NULL WHERE s.id = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.lab_id = $3 AND u.status = 'active' AND u.deleted_at IS NULL)",
        )
        .bind(session_id)
        .bind(actor.user_id)
        .bind(self.lab_id)
        .fetch_one(self.postgres.pool())
        .await
        .map_err(database)?;
        if valid {
            Ok(())
        } else {
            Err(UserGovernanceError::Forbidden)
        }
    }

    async fn lock_and_validate_actor(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        context: &AdminMutationContext<'_>,
        verified: &VerifiedCredential,
    ) -> Result<(), UserGovernanceError> {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(GOVERNANCE_LOCK_ID)
            .execute(&mut **transaction)
            .await
            .map_err(database)?;
        let AuthenticationMethod::Session { session_id } = context.authentication else {
            return Err(UserGovernanceError::SessionRequired);
        };
        let session_exists = sqlx::query_scalar::<_, Uuid>(
            "SELECT s.id FROM auth_sessions s WHERE s.id = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now() FOR UPDATE",
        )
        .bind(session_id)
        .bind(context.actor.user_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?
        .is_some();
        if !session_exists {
            return Err(UserGovernanceError::SessionRequired);
        }
        let current_hash: Option<String> = sqlx::query_scalar(
            "SELECT c.password_hash FROM users u JOIN user_credentials c ON c.user_id = u.id WHERE u.id = $1 AND u.lab_id = $2 AND u.status = 'active' AND u.deleted_at IS NULL FOR UPDATE OF u, c",
        )
        .bind(context.actor.user_id)
        .bind(self.lab_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?;
        if current_hash.as_deref() != Some(verified.0.as_str()) {
            return Err(UserGovernanceError::StepUpFailed);
        }
        let admin_membership = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND lab_role = 'lab_admin' AND deleted_at IS NULL LIMIT 1 FOR UPDATE",
        )
        .bind(self.lab_id)
        .bind(context.actor.user_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?;
        if admin_membership.is_none() {
            return Err(UserGovernanceError::Forbidden);
        }
        Ok(())
    }

    async fn lock_and_validate_actor_for_project(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        context: &AdminMutationContext<'_>,
        verified: &VerifiedCredential,
        project_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(GOVERNANCE_LOCK_ID)
            .execute(&mut **transaction)
            .await
            .map_err(database)?;
        let AuthenticationMethod::Session { session_id } = context.authentication else {
            return Err(UserGovernanceError::SessionRequired);
        };
        let session_exists = sqlx::query_scalar::<_, Uuid>(
            "SELECT s.id FROM auth_sessions s WHERE s.id = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now() FOR UPDATE",
        )
        .bind(session_id)
        .bind(context.actor.user_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?
        .is_some();
        if !session_exists {
            return Err(UserGovernanceError::SessionRequired);
        }
        let current_hash: Option<String> = sqlx::query_scalar(
            "SELECT c.password_hash FROM users u JOIN user_credentials c ON c.user_id = u.id WHERE u.id = $1 AND u.lab_id = $2 AND u.status = 'active' AND u.deleted_at IS NULL FOR UPDATE OF u, c",
        )
        .bind(context.actor.user_id)
        .bind(self.lab_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?;
        if current_hash.as_deref() != Some(verified.0.as_str()) {
            return Err(UserGovernanceError::StepUpFailed);
        }
        let governing_membership = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM memberships WHERE lab_id = $1 AND user_id = $2 AND ((project_id IS NULL AND lab_role = 'lab_admin') OR (project_id = $3 AND project_role = 'project_admin')) AND deleted_at IS NULL ORDER BY project_id NULLS FIRST LIMIT 1 FOR UPDATE",
        )
        .bind(self.lab_id)
        .bind(context.actor.user_id)
        .bind(project_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database)?;
        if governing_membership.is_none() {
            return Err(UserGovernanceError::Forbidden);
        }
        Ok(())
    }

    fn actor_is_environment_root(&self, actor: &AuthPrincipal) -> bool {
        actor.user_id == self.environment_root_user_id && actor.is_environment_root()
    }

    async fn ensure_target_governable(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        context: &AdminMutationContext<'_>,
        target_user_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        if target_user_id == self.environment_root_user_id {
            return Err(UserGovernanceError::EnvironmentRootManaged);
        }
        if self.actor_is_environment_root(context.actor) {
            return Ok(());
        }
        let target_is_lab_admin: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE lab_id = $1 AND user_id = $2 AND project_id IS NULL AND lab_role = 'lab_admin' AND deleted_at IS NULL)",
        )
        .bind(self.lab_id)
        .bind(target_user_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(database)?;
        if target_is_lab_admin {
            Err(UserGovernanceError::LabAdminManagedByRoot)
        } else {
            Ok(())
        }
    }

    async fn ensure_admin_removal_safe(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        let active_admins: Vec<Uuid> = sqlx::query_scalar(
            "SELECT m.user_id FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.lab_id = $1 AND m.project_id IS NULL AND m.lab_role = 'lab_admin' AND m.deleted_at IS NULL AND u.status = 'active' AND u.deleted_at IS NULL ORDER BY m.user_id FOR UPDATE OF m, u",
        )
        .bind(self.lab_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(database)?;
        if active_admins.len() == 1 && active_admins[0] == user_id {
            Err(UserGovernanceError::LastActiveLabAdmin)
        } else {
            Ok(())
        }
    }

    async fn ensure_project_admin_removal_safe(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        project_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), UserGovernanceError> {
        let active_admins: Vec<Uuid> = sqlx::query_scalar(
            "SELECT m.user_id FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.lab_id = $1 AND m.project_id = $2 AND m.project_role = 'project_admin' AND m.deleted_at IS NULL AND u.status = 'active' AND u.deleted_at IS NULL ORDER BY m.user_id FOR UPDATE OF m, u",
        )
        .bind(self.lab_id)
        .bind(project_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(database)?;
        if active_admins.len() == 1 && active_admins[0] == user_id {
            Err(UserGovernanceError::LastActiveProjectAdmin)
        } else {
            Ok(())
        }
    }

    async fn validate_projects(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        assignments: &[InitialProjectRole],
    ) -> Result<BTreeMap<Uuid, String>, UserGovernanceError> {
        let mut names = BTreeMap::new();
        for assignment in assignments {
            let name = validate_project(transaction, self.lab_id, assignment.project_id).await?;
            names.insert(assignment.project_id, name);
        }
        Ok(names)
    }

    async fn load_managed_user(&self, user_id: Uuid) -> Result<ManagedUser, UserGovernanceError> {
        self.load_managed_users(Some(user_id))
            .await?
            .into_iter()
            .next()
            .ok_or(UserGovernanceError::NotFound)
    }

    async fn load_managed_users(
        &self,
        user_id: Option<Uuid>,
    ) -> Result<Vec<ManagedUser>, UserGovernanceError> {
        let rows = sqlx::query(
            "SELECT u.id, u.email, u.display_name, u.status, u.revision, u.created_at, u.updated_at, COALESCE(c.revision, 0) AS credential_revision, COALESCE(c.must_change_password, TRUE) AS must_change_password, m.id AS lab_membership_id, m.lab_role, m.revision AS lab_membership_revision FROM users u LEFT JOIN user_credentials c ON c.user_id = u.id LEFT JOIN memberships m ON m.lab_id = u.lab_id AND m.user_id = u.id AND m.project_id IS NULL AND m.deleted_at IS NULL WHERE u.lab_id = $1 AND u.deleted_at IS NULL AND ($2::uuid IS NULL OR u.id = $2) ORDER BY lower(u.email), u.id",
        )
        .bind(self.lab_id)
        .bind(user_id)
        .fetch_all(self.postgres.pool())
        .await
        .map_err(database)?;
        let project_rows = sqlx::query(
            "SELECT m.id, m.user_id, m.project_id, m.project_role, m.revision, p.name FROM memberships m JOIN projects p ON p.id = m.project_id AND p.lab_id = m.lab_id AND p.deleted_at IS NULL WHERE m.lab_id = $1 AND m.project_id IS NOT NULL AND m.deleted_at IS NULL AND ($2::uuid IS NULL OR m.user_id = $2) ORDER BY p.name, p.id, m.id",
        )
        .bind(self.lab_id)
        .bind(user_id)
        .fetch_all(self.postgres.pool())
        .await
        .map_err(database)?;
        let mut projects: BTreeMap<Uuid, Vec<ManagedProjectMembership>> = BTreeMap::new();
        for row in project_rows {
            let owner: Uuid = row.try_get("user_id").map_err(database)?;
            projects
                .entry(owner)
                .or_default()
                .push(ManagedProjectMembership {
                    membership_id: row.try_get("id").map_err(database)?,
                    project_id: row.try_get("project_id").map_err(database)?,
                    project_name: row.try_get("name").map_err(database)?,
                    role: parse_project_role(row.try_get("project_role").map_err(database)?)?,
                    revision: row.try_get("revision").map_err(database)?,
                });
        }
        rows.into_iter()
            .map(|row| {
                let id: Uuid = row.try_get("id").map_err(database)?;
                let lab_role = row
                    .try_get::<Option<String>, _>("lab_role")
                    .map_err(database)?
                    .map(|role| parse_lab_role(&role))
                    .transpose()?;
                Ok(ManagedUser {
                    id,
                    email: row.try_get("email").map_err(database)?,
                    display_name: row.try_get("display_name").map_err(database)?,
                    status: parse_user_status(row.try_get("status").map_err(database)?)?,
                    revision: row.try_get("revision").map_err(database)?,
                    credential_revision: row.try_get("credential_revision").map_err(database)?,
                    must_change_password: row.try_get("must_change_password").map_err(database)?,
                    is_environment_root: id == self.environment_root_user_id,
                    lab_membership_id: row.try_get("lab_membership_id").map_err(database)?,
                    lab_role,
                    lab_membership_revision: row
                        .try_get("lab_membership_revision")
                        .map_err(database)?,
                    project_memberships: projects.remove(&id).unwrap_or_default(),
                    created_at: row.try_get("created_at").map_err(database)?,
                    updated_at: row.try_get("updated_at").map_err(database)?,
                })
            })
            .collect()
    }
}

fn validate_project_assignments(
    lab_role: Option<LabRole>,
    assignments: &[InitialProjectRole],
) -> Result<(), UserGovernanceError> {
    if lab_role.is_none() && assignments.is_empty() {
        return Err(UserGovernanceError::Validation(
            "a user must receive either a lab role or at least one project role".to_owned(),
        ));
    }
    if assignments.len() > MAX_INITIAL_PROJECTS {
        return Err(UserGovernanceError::Validation(format!(
            "no more than {MAX_INITIAL_PROJECTS} initial project memberships are allowed"
        )));
    }
    let mut projects = BTreeSet::new();
    if assignments
        .iter()
        .any(|assignment| !projects.insert(assignment.project_id))
    {
        return Err(UserGovernanceError::Validation(
            "initial project memberships contain duplicate projects".to_owned(),
        ));
    }
    Ok(())
}

fn validate_email(email: &str) -> Result<(), UserGovernanceError> {
    if email.len() > 320
        || !email.contains('@')
        || email.chars().any(char::is_control)
        || email.chars().any(char::is_whitespace)
    {
        Err(UserGovernanceError::Validation(
            "email must be a valid non-whitespace address of at most 320 bytes".to_owned(),
        ))
    } else {
        Ok(())
    }
}

async fn validate_project(
    transaction: &mut Transaction<'_, Postgres>,
    lab_id: Uuid,
    project_id: Uuid,
) -> Result<String, UserGovernanceError> {
    sqlx::query_scalar(
        "SELECT name FROM projects WHERE id = $1 AND lab_id = $2 AND deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(lab_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database)?
    .ok_or(UserGovernanceError::NotFound)
}

async fn load_user_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    lab_id: Uuid,
    user_id: Uuid,
) -> Result<User, UserGovernanceError> {
    let row = sqlx::query(
        "SELECT id, lab_id, email, display_name, status, created_at, updated_at, deleted_at, revision FROM users WHERE id = $1 AND lab_id = $2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(user_id)
    .bind(lab_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database)?
    .ok_or(UserGovernanceError::NotFound)?;
    Ok(User {
        id: row.try_get("id").map_err(database)?,
        lab_id: row.try_get("lab_id").map_err(database)?,
        email: row.try_get("email").map_err(database)?,
        display_name: row.try_get("display_name").map_err(database)?,
        status: parse_user_status(row.try_get("status").map_err(database)?)?,
        meta: RecordMeta {
            created_at: row.try_get("created_at").map_err(database)?,
            updated_at: row.try_get("updated_at").map_err(database)?,
            deleted_at: row.try_get("deleted_at").map_err(database)?,
            revision: row.try_get("revision").map_err(database)?,
        },
    })
}

async fn update_user_row(
    transaction: &mut Transaction<'_, Postgres>,
    user: &User,
    expected_revision: i64,
) -> Result<(), UserGovernanceError> {
    let changed = sqlx::query(
        "UPDATE users SET status = $1, email = $2, display_name = $3, updated_at = $4, revision = $5 WHERE id = $6 AND lab_id = $7 AND revision = $8 AND deleted_at IS NULL",
    )
    .bind(user_status_name(user.status))
    .bind(&user.email)
    .bind(&user.display_name)
    .bind(user.meta.updated_at)
    .bind(user.meta.revision)
    .bind(user.id)
    .bind(user.lab_id)
    .bind(expected_revision)
    .execute(&mut **transaction)
    .await
    .map_err(conflict_or_database)?
    .rows_affected();
    if changed == 1 {
        Ok(())
    } else {
        Err(stale("user"))
    }
}

async fn load_membership_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    lab_id: Uuid,
    membership_id: Uuid,
) -> Result<Membership, UserGovernanceError> {
    let row = sqlx::query(
        "SELECT id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision FROM memberships WHERE id = $1 AND lab_id = $2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(membership_id)
    .bind(lab_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database)?
    .ok_or(UserGovernanceError::NotFound)?;
    let lab_role = row
        .try_get::<Option<String>, _>("lab_role")
        .map_err(database)?
        .map(|role| parse_lab_role(&role))
        .transpose()?;
    let project_role = row
        .try_get::<Option<String>, _>("project_role")
        .map_err(database)?
        .map(|role| parse_project_role(&role))
        .transpose()?;
    Ok(Membership {
        id: row.try_get("id").map_err(database)?,
        lab_id: row.try_get("lab_id").map_err(database)?,
        project_id: row.try_get("project_id").map_err(database)?,
        user_id: row.try_get("user_id").map_err(database)?,
        lab_role,
        project_role,
        meta: RecordMeta {
            created_at: row.try_get("created_at").map_err(database)?,
            updated_at: row.try_get("updated_at").map_err(database)?,
            deleted_at: row.try_get("deleted_at").map_err(database)?,
            revision: row.try_get("revision").map_err(database)?,
        },
    })
}

async fn insert_membership(
    transaction: &mut Transaction<'_, Postgres>,
    membership: &Membership,
) -> Result<(), UserGovernanceError> {
    membership
        .validate_scope()
        .map_err(|error| UserGovernanceError::Validation(error.to_string()))?;
    sqlx::query(
        "INSERT INTO memberships (id, lab_id, project_id, user_id, lab_role, project_role, created_at, updated_at, deleted_at, revision) VALUES ($1, $2, $3, $4, $5, $6, $7, $7, NULL, 1)",
    )
    .bind(membership.id)
    .bind(membership.lab_id)
    .bind(membership.project_id)
    .bind(membership.user_id)
    .bind(membership.lab_role.map(lab_role_name))
    .bind(membership.project_role.map(project_role_name))
    .bind(membership.meta.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(conflict_or_database)?;
    Ok(())
}

async fn revoke_user_authentication(
    transaction: &mut Transaction<'_, Postgres>,
    context: &AdminMutationContext<'_>,
    user_id: Uuid,
    reason: &'static str,
    now: DateTime<Utc>,
) -> Result<(), UserGovernanceError> {
    let (session_operation, token_operation) = match reason {
        "administrator_password_reset" => (
            "auth.session.revoked.password_reset",
            "auth.external_token.revoked.password_reset",
        ),
        "user_suspended" => (
            "auth.session.revoked.user_suspended",
            "auth.external_token.revoked.user_suspended",
        ),
        _ => return Err(UserGovernanceError::Unavailable),
    };
    let session_ids: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE auth_sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL RETURNING id",
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database)?;
    for session_id in session_ids {
        write_audit(
            transaction,
            context,
            None,
            "auth_session",
            session_id,
            "revoke",
            session_operation,
            None,
            Some(json!({"revoked_at": now, "reason": reason})),
            now,
        )
        .await?;
    }
    let token_ids: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE external_tokens SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL RETURNING id",
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database)?;
    for token_id in token_ids {
        write_audit(
            transaction,
            context,
            None,
            "external_token",
            token_id,
            "revoke",
            token_operation,
            None,
            Some(json!({"revoked_at": now, "reason": reason})),
            now,
        )
        .await?;
    }
    Ok(())
}

async fn write_membership_audit(
    transaction: &mut Transaction<'_, Postgres>,
    context: &AdminMutationContext<'_>,
    action: &'static str,
    before: Option<&Membership>,
    after: &Membership,
    occurred_at: DateTime<Utc>,
) -> Result<(), UserGovernanceError> {
    let operation_code = match action {
        "create" => "auth.membership.created",
        "update" => "auth.membership.role_updated",
        "soft_delete" => "auth.membership.revoked",
        _ => return Err(UserGovernanceError::Unavailable),
    };
    write_audit(
        transaction,
        context,
        after.project_id,
        "membership",
        after.id,
        action,
        operation_code,
        before.map(to_json).transpose()?,
        Some(to_json(after)?),
        occurred_at,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn write_audit(
    transaction: &mut Transaction<'_, Postgres>,
    context: &AdminMutationContext<'_>,
    project_id: Option<Uuid>,
    entity_type: &'static str,
    entity_id: Uuid,
    action: &'static str,
    operation_code: &str,
    before: Option<Value>,
    after: Option<Value>,
    occurred_at: DateTime<Utc>,
) -> Result<(), UserGovernanceError> {
    sqlx::query(
        "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at, operation_code, operation_version, operation_params_json) VALUES ($1, $2, $3, $4, $5, $6, 'human', $7, $8, 'web', $9, $10, $11, $12, $13, $14, 1, $15)",
    )
    .bind(Uuid::new_v4())
    .bind(context.actor.lab_id)
    .bind(project_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(action)
    .bind(context.actor.user_id)
    .bind(&context.actor.display_name)
    .bind(&context.metadata.request_id)
    .bind(
        context
            .metadata
            .reason
            .as_deref()
            .unwrap_or("administrator account governance"),
    )
    .bind(before)
    .bind(after)
    .bind(occurred_at)
    .bind(operation_code)
    .bind(json!({"credential_material": "redacted"}))
    .execute(&mut **transaction)
    .await
    .map_err(database)?;
    Ok(())
}

fn to_json(value: &impl Serialize) -> Result<Value, UserGovernanceError> {
    serde_json::to_value(value).map_err(|_| UserGovernanceError::Unavailable)
}

fn ensure_revision(
    actual: i64,
    expected: i64,
    entity: &'static str,
) -> Result<(), UserGovernanceError> {
    if actual == expected {
        Ok(())
    } else {
        Err(stale(entity))
    }
}

fn stale(entity: &'static str) -> UserGovernanceError {
    UserGovernanceError::Conflict(format!(
        "{entity} revision changed before the operation was applied"
    ))
}

fn user_status_name(status: UserStatus) -> &'static str {
    match status {
        UserStatus::Active => "active",
        UserStatus::Suspended => "suspended",
    }
}

fn parse_user_status(value: &str) -> Result<UserStatus, UserGovernanceError> {
    match value {
        "active" => Ok(UserStatus::Active),
        "suspended" => Ok(UserStatus::Suspended),
        _ => Err(UserGovernanceError::Unavailable),
    }
}

fn lab_role_name(role: LabRole) -> &'static str {
    match role {
        LabRole::LabAdmin => "lab_admin",
        LabRole::AnimalManager => "animal_manager",
    }
}

fn parse_lab_role(value: &str) -> Result<LabRole, UserGovernanceError> {
    match value {
        "lab_admin" => Ok(LabRole::LabAdmin),
        "animal_manager" => Ok(LabRole::AnimalManager),
        _ => Err(UserGovernanceError::Unavailable),
    }
}

fn project_role_name(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::ProjectAdmin => "project_admin",
        ProjectRole::Editor => "editor",
        ProjectRole::Viewer => "viewer",
    }
}

fn parse_project_role(value: &str) -> Result<ProjectRole, UserGovernanceError> {
    match value {
        "project_admin" => Ok(ProjectRole::ProjectAdmin),
        "editor" => Ok(ProjectRole::Editor),
        "viewer" => Ok(ProjectRole::Viewer),
        _ => Err(UserGovernanceError::Unavailable),
    }
}

fn conflict_or_database(error: sqlx::Error) -> UserGovernanceError {
    if let sqlx::Error::Database(database_error) = &error
        && database_error.is_unique_violation()
    {
        return UserGovernanceError::Conflict(
            "an active user or membership already uses that identity".to_owned(),
        );
    }
    database(error)
}

fn database(error: sqlx::Error) -> UserGovernanceError {
    tracing::error!(error = %error, "user governance database operation failed");
    UserGovernanceError::Unavailable
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Duration;
    use muriarc_core::{AuditContext, MuriArcStore, Permission, Project, WriteSource};

    use super::*;

    #[test]
    fn secrets_and_commands_are_debug_redacted_and_not_serializable() {
        let password = SensitivePassword::new("not-for-output-password");
        assert!(!format!("{password:?}").contains("not-for-output"));
        let command = CreateManagedUserCommand::new(
            "user@example.org",
            "User",
            SensitivePassword::new("another-private-password"),
            Some(LabRole::AnimalManager),
            vec![],
        );
        let debug = format!("{command:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("another-private-password"));
    }

    #[test]
    fn managed_user_json_has_no_credential_fields() {
        let now = Utc::now();
        let view = ManagedUser {
            id: Uuid::new_v4(),
            email: "user@example.org".to_owned(),
            display_name: "User".to_owned(),
            status: UserStatus::Active,
            revision: 1,
            credential_revision: 1,
            must_change_password: false,
            is_environment_root: false,
            lab_membership_id: None,
            lab_role: None,
            lab_membership_revision: None,
            project_memberships: vec![],
            created_at: now,
            updated_at: now,
        };
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("passwordHash"));
        assert!(!encoded.contains("temporaryPassword"));
        assert!(encoded.contains("credentialRevision"));
        assert!(!encoded.contains("argon2"));
    }

    async fn login_session(
        backend: &crate::PostgresAuthBackend,
        email: &str,
        password: &str,
    ) -> (AuthPrincipal, AuthenticationMethod, String) {
        use crate::SessionBackend as _;

        let now = Utc::now();
        let raw_session = format!("mas_{}", Uuid::new_v4().simple());
        let raw_csrf = format!("mac_{}", Uuid::new_v4().simple());
        let session = crate::NewSession {
            id: Uuid::new_v4(),
            token_hash: crate::token_hash(&raw_session),
            csrf_hash: crate::token_hash(&raw_csrf),
            created_at: now,
            expires_at: now + Duration::hours(1),
        };
        let authenticated = backend.login(email, password, &session).await.unwrap();
        (
            authenticated.principal,
            AuthenticationMethod::Session {
                session_id: session.id,
            },
            raw_session,
        )
    }

    fn context<'a>(
        actor: &'a AuthPrincipal,
        authentication: AuthenticationMethod,
        metadata: &'a RequestMetadata,
        password: &'a SensitivePassword,
    ) -> AdminMutationContext<'a> {
        AdminMutationContext {
            actor,
            authentication,
            metadata,
            current_password: password,
        }
    }

    #[tokio::test]
    async fn postgres_governance_is_scoped_step_up_protected_and_audited() {
        use crate::{Authenticator as _, SessionBackend as _};

        let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            database_url.contains("muriarc_test"),
            "MURIARC_TEST_DATABASE_URL must point to a disposable muriarc_test database"
        );
        let store = Arc::new(PostgresStore::connect(&database_url).await.unwrap());
        store.migrate().await.unwrap();

        let admin_password = format!("admin-password-{}", Uuid::new_v4());
        let bootstrap = crate::BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Governance integration lab",
            Uuid::new_v4(),
            format!("governance-admin-{}@example.org", Uuid::new_v4()),
            "Governance administrator",
        )
        .unwrap()
        .with_password_hash(crate::hash_password(&admin_password).unwrap())
        .unwrap();
        crate::seed_postgres_bootstrap(store.as_ref(), &bootstrap)
            .await
            .unwrap();
        let auth = crate::PostgresAuthBackend::new(
            store.as_ref().clone(),
            store.clone(),
            bootstrap.lab_id,
            bootstrap.user_id,
        )
        .unwrap();
        let (admin, admin_method, _) =
            login_session(&auth, &bootstrap.user_email, &admin_password).await;
        let service = PostgresUserGovernance::new(
            store.as_ref().clone(),
            bootstrap.lab_id,
            bootstrap.user_id,
        );
        let metadata = RequestMetadata {
            request_id: format!("governance-test-{}", Uuid::new_v4()),
            reason: Some("governance integration test".to_owned()),
        };
        let step_up = SensitivePassword::new(admin_password.clone());
        let admin_context = context(&admin, admin_method, &metadata, &step_up);

        let audit = AuditContext::system(WriteSource::Migration);
        let project = Project::new(bootstrap.lab_id, "Governance project", Utc::now()).unwrap();
        store.create_project(&project, &audit).await.unwrap();

        let credentialless = User::new(
            bootstrap.lab_id,
            format!("credentialless-{}@example.org", Uuid::new_v4()),
            "Credentialless legacy user",
            Utc::now(),
        )
        .unwrap();
        store.create_user(&credentialless, &audit).await.unwrap();
        store
            .create_membership(
                &Membership::lab(
                    bootstrap.lab_id,
                    credentialless.id,
                    LabRole::AnimalManager,
                    Utc::now(),
                ),
                &audit,
            )
            .await
            .unwrap();
        let credentialless_view = service
            .list_users(&admin, admin_method, None)
            .await
            .unwrap()
            .into_iter()
            .find(|user| user.id == credentialless.id)
            .unwrap();
        assert_eq!(credentialless_view.credential_revision, 0);
        assert!(credentialless_view.must_change_password);
        let credentialless_password = format!("legacy-reset-{}", Uuid::new_v4());
        let credentialless_view = service
            .reset_user_password(
                &admin_context,
                credentialless.id,
                0,
                SensitivePassword::new(credentialless_password.clone()),
            )
            .await
            .unwrap();
        assert_eq!(credentialless_view.credential_revision, 1);
        assert!(credentialless_view.must_change_password);
        let (credentialless_principal, _, _) =
            login_session(&auth, &credentialless_view.email, &credentialless_password).await;
        assert!(credentialless_principal.must_change_password());

        let wrong = SensitivePassword::new("definitely-the-wrong-password");
        let wrong_context = context(&admin, admin_method, &metadata, &wrong);
        assert_eq!(
            service
                .create_user(
                    &wrong_context,
                    CreateManagedUserCommand::new(
                        format!("wrong-step-up-{}@example.org", Uuid::new_v4()),
                        "Wrong step-up",
                        SensitivePassword::new("valid-initial-password"),
                        Some(LabRole::AnimalManager),
                        vec![],
                    ),
                )
                .await
                .unwrap_err(),
            UserGovernanceError::StepUpFailed
        );

        let managed_password = format!("managed-password-{}", Uuid::new_v4());
        let managed_email = format!("managed-{}@example.org", Uuid::new_v4());
        let mut managed = service
            .create_user(
                &admin_context,
                CreateManagedUserCommand::new(
                    managed_email.to_uppercase(),
                    "Managed researcher",
                    SensitivePassword::new(managed_password.clone()),
                    Some(LabRole::AnimalManager),
                    vec![InitialProjectRole {
                        project_id: project.id,
                        role: ProjectRole::Viewer,
                    }],
                ),
            )
            .await
            .unwrap();
        assert_eq!(managed.email, managed_email);
        assert_eq!(managed.lab_role, Some(LabRole::AnimalManager));
        assert_eq!(managed.project_memberships.len(), 1);

        let project_only_password = format!("project-only-password-{}", Uuid::new_v4());
        let project_only_email = format!("project-only-{}@example.org", Uuid::new_v4());
        let project_only = service
            .create_user(
                &admin_context,
                CreateManagedUserCommand::new(
                    &project_only_email,
                    "Project-only researcher",
                    SensitivePassword::new(project_only_password.clone()),
                    None,
                    vec![InitialProjectRole {
                        project_id: project.id,
                        role: ProjectRole::Viewer,
                    }],
                ),
            )
            .await
            .unwrap();
        assert_eq!(project_only.lab_role, None);
        let (project_only_principal, _, _) =
            login_session(&auth, &project_only_email, &project_only_password).await;
        assert!(project_only_principal.can(Permission::ReadMeasurement, Some(project.id)));
        assert!(!project_only_principal.can(Permission::ReadMeasurement, None));
        assert!(!project_only_principal.can(Permission::ReadMeasurement, Some(Uuid::new_v4())));
        assert!(!project_only_principal.can(Permission::ManageUsers, None));

        assert!(matches!(
            service
                .create_user(
                    &admin_context,
                    CreateManagedUserCommand::new(
                        managed_email.to_uppercase(),
                        "Duplicate",
                        SensitivePassword::new("duplicate-user-password"),
                        Some(LabRole::AnimalManager),
                        vec![],
                    ),
                )
                .await,
            Err(UserGovernanceError::Conflict(_))
        ));

        let (managed_principal, managed_method, managed_raw_session) =
            login_session(&auth, &managed_email, &managed_password).await;
        let managed_step_up = SensitivePassword::new(managed_password.clone());
        let managed_context = context(
            &managed_principal,
            managed_method,
            &metadata,
            &managed_step_up,
        );
        assert_eq!(
            service
                .create_user(
                    &managed_context,
                    CreateManagedUserCommand::new(
                        format!("forbidden-{}@example.org", Uuid::new_v4()),
                        "Forbidden",
                        SensitivePassword::new("forbidden-user-password"),
                        Some(LabRole::AnimalManager),
                        vec![],
                    ),
                )
                .await
                .unwrap_err(),
            UserGovernanceError::Forbidden
        );
        assert_eq!(
            service
                .list_users(&managed_principal, managed_method, None)
                .await
                .unwrap_err(),
            UserGovernanceError::Forbidden
        );
        assert!(managed.must_change_password);
        assert_eq!(managed.credential_revision, 1);
        assert!(managed_principal.must_change_password());

        let AuthenticationMethod::Session {
            session_id: managed_session_id,
        } = managed_method
        else {
            panic!("managed login must create a browser session");
        };
        let permanent_password = format!("permanent-password-{}", Uuid::new_v4());
        let ready_principal = auth
            .change_password(
                &managed_principal,
                managed_session_id,
                &managed_password,
                &permanent_password,
                &metadata.request_id,
            )
            .await
            .unwrap();
        assert!(!ready_principal.must_change_password());
        managed = service
            .list_users(&admin, admin_method, None)
            .await
            .unwrap()
            .into_iter()
            .find(|user| user.id == managed.id)
            .unwrap();
        assert_eq!(managed.credential_revision, 2);
        assert!(!managed.must_change_password);

        let updated_email = format!("updated-managed-{}@example.org", Uuid::new_v4());
        managed = service
            .update_user_profile(
                &admin_context,
                managed.id,
                managed.revision,
                &updated_email,
                "Updated managed researcher",
            )
            .await
            .unwrap();
        assert_eq!(managed.email, updated_email);
        assert_eq!(managed.display_name, "Updated managed researcher");

        let (_, _, second_managed_session) =
            login_session(&auth, &managed.email, &permanent_password).await;
        let external_raw = format!("mat_{}", Uuid::new_v4().simple());
        let external = crate::NewExternalToken {
            id: Uuid::new_v4(),
            name: "Managed integration token".to_owned(),
            token_hash: crate::token_hash(&external_raw),
            scopes: BTreeSet::from([muriarc_core::AiScope::Read]),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::days(1),
        };
        auth.create_external_token(managed.id, &external)
            .await
            .unwrap();
        assert!(auth.authenticate(&external_raw).await.is_ok());

        let reset_password = format!("reset-password-{}", Uuid::new_v4());
        managed = service
            .reset_user_password(
                &admin_context,
                managed.id,
                managed.credential_revision,
                SensitivePassword::new(reset_password.clone()),
            )
            .await
            .unwrap();
        assert_eq!(managed.credential_revision, 3);
        assert!(managed.must_change_password);
        assert_eq!(
            auth.authenticate_session(&managed_raw_session)
                .await
                .unwrap_err(),
            crate::AuthError::InvalidCredentials
        );
        assert_eq!(
            auth.authenticate_session(&second_managed_session)
                .await
                .unwrap_err(),
            crate::AuthError::InvalidCredentials
        );
        assert_eq!(
            auth.authenticate(&external_raw).await.unwrap_err(),
            crate::AuthError::InvalidCredentials
        );
        let (reset_principal, _, reset_raw_session) =
            login_session(&auth, &managed.email, &reset_password).await;
        assert!(reset_principal.must_change_password());

        let other_password = format!("other-admin-password-{}", Uuid::new_v4());
        let other = crate::BootstrapSeedConfig::new(
            Uuid::new_v4(),
            "Other governance lab",
            Uuid::new_v4(),
            format!("other-admin-{}@example.org", Uuid::new_v4()),
            "Other administrator",
        )
        .unwrap()
        .with_password_hash(crate::hash_password(&other_password).unwrap())
        .unwrap();
        crate::seed_postgres_bootstrap(store.as_ref(), &other)
            .await
            .unwrap();
        let other_auth = crate::PostgresAuthBackend::new(
            store.as_ref().clone(),
            store.clone(),
            other.lab_id,
            other.user_id,
        )
        .unwrap();
        let (other_admin, other_method, _) =
            login_session(&other_auth, &other.user_email, &other_password).await;
        assert_eq!(
            service
                .list_users(&other_admin, other_method, None)
                .await
                .unwrap_err(),
            UserGovernanceError::Forbidden
        );
        assert_eq!(
            service
                .set_user_status(&admin_context, other.user_id, 1, UserStatus::Suspended,)
                .await
                .unwrap_err(),
            UserGovernanceError::NotFound
        );

        let admin_view = service
            .list_users(&admin, admin_method, None)
            .await
            .unwrap()
            .into_iter()
            .find(|user| user.id == admin.user_id)
            .unwrap();
        assert_eq!(
            service
                .set_user_status(
                    &admin_context,
                    admin.user_id,
                    admin_view.revision,
                    UserStatus::Suspended,
                )
                .await
                .unwrap_err(),
            UserGovernanceError::EnvironmentRootManaged
        );
        assert!(admin_view.is_environment_root);
        assert_eq!(
            service
                .update_user_profile(
                    &admin_context,
                    admin.user_id,
                    admin_view.revision,
                    &bootstrap.user_email,
                    "Application-managed root",
                )
                .await
                .unwrap_err(),
            UserGovernanceError::EnvironmentRootManaged
        );
        assert_eq!(
            service
                .reset_user_password(
                    &admin_context,
                    admin.user_id,
                    admin_view.credential_revision,
                    SensitivePassword::new("application-root-reset"),
                )
                .await
                .unwrap_err(),
            UserGovernanceError::EnvironmentRootManaged
        );

        let second_admin_password = format!("second-admin-password-{}", Uuid::new_v4());
        let second_admin = service
            .create_user(
                &admin_context,
                CreateManagedUserCommand::new(
                    format!("second-admin-{}@example.org", Uuid::new_v4()),
                    "Second administrator",
                    SensitivePassword::new(second_admin_password.clone()),
                    Some(LabRole::LabAdmin),
                    vec![],
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .set_user_status(
                    &admin_context,
                    admin.user_id,
                    admin_view.revision,
                    UserStatus::Suspended,
                )
                .await
                .unwrap_err(),
            UserGovernanceError::EnvironmentRootManaged
        );

        let (forced_second_admin, second_admin_method, _) =
            login_session(&auth, &second_admin.email, &second_admin_password).await;
        assert!(forced_second_admin.must_change_password());
        let AuthenticationMethod::Session {
            session_id: second_admin_session_id,
        } = second_admin_method
        else {
            panic!("LabAdmin login must create a browser session");
        };
        let second_admin_permanent_password = format!("second-admin-permanent-{}", Uuid::new_v4());
        let ready_second_admin = auth
            .change_password(
                &forced_second_admin,
                second_admin_session_id,
                &second_admin_password,
                &second_admin_permanent_password,
                &metadata.request_id,
            )
            .await
            .unwrap();
        let second_admin_step_up = SensitivePassword::new(second_admin_permanent_password.clone());
        let second_admin_context = context(
            &ready_second_admin,
            second_admin_method,
            &metadata,
            &second_admin_step_up,
        );
        let peer_admin = service
            .create_user(
                &admin_context,
                CreateManagedUserCommand::new(
                    format!("peer-admin-{}@example.org", Uuid::new_v4()),
                    "Peer administrator",
                    SensitivePassword::new("peer-admin-temporary-password"),
                    Some(LabRole::LabAdmin),
                    vec![],
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .update_user_profile(
                    &second_admin_context,
                    peer_admin.id,
                    peer_admin.revision,
                    &peer_admin.email,
                    "Peer changed by peer",
                )
                .await
                .unwrap_err(),
            UserGovernanceError::LabAdminManagedByRoot
        );
        assert_eq!(
            service
                .reset_user_password(
                    &second_admin_context,
                    peer_admin.id,
                    peer_admin.credential_revision,
                    SensitivePassword::new("peer-reset-password"),
                )
                .await
                .unwrap_err(),
            UserGovernanceError::LabAdminManagedByRoot
        );

        let project_membership = managed.project_memberships[0].clone();
        managed = service
            .update_project_role(
                &admin_context,
                project_membership.membership_id,
                project_membership.revision,
                ProjectRole::Editor,
            )
            .await
            .unwrap();
        let updated_membership = managed.project_memberships[0].clone();
        assert_eq!(updated_membership.role, ProjectRole::Editor);
        managed = service
            .revoke_membership(
                &admin_context,
                updated_membership.membership_id,
                updated_membership.revision,
            )
            .await
            .unwrap();
        assert!(managed.project_memberships.is_empty());

        managed = service
            .set_user_status(
                &admin_context,
                managed.id,
                managed.revision,
                UserStatus::Suspended,
            )
            .await
            .unwrap();
        assert_eq!(managed.status, UserStatus::Suspended);
        assert_eq!(
            auth.authenticate_session(&managed_raw_session)
                .await
                .unwrap_err(),
            crate::AuthError::InvalidCredentials
        );
        assert_eq!(
            auth.authenticate_session(&reset_raw_session)
                .await
                .unwrap_err(),
            crate::AuthError::InvalidCredentials
        );

        let encoded = serde_json::to_string(&managed).unwrap();
        assert!(!encoded.contains(&managed_password));
        assert!(!encoded.contains(&admin_password));
        assert!(!encoded.contains("passwordHash"));
        let audit_payloads: Vec<String> = sqlx::query_scalar(
            "SELECT coalesce(before_json::text, '') || coalesce(after_json::text, '') FROM audit_entries WHERE lab_id = $1",
        )
        .bind(bootstrap.lab_id)
        .fetch_all(store.pool())
        .await
        .unwrap();
        let audit_payloads = audit_payloads.join("\n");
        assert!(!audit_payloads.contains(&credentialless_password));
        assert!(!audit_payloads.contains(&managed_password));
        assert!(!audit_payloads.contains(&permanent_password));
        assert!(!audit_payloads.contains(&reset_password));
        assert!(!audit_payloads.contains(&second_admin_permanent_password));
        assert!(!audit_payloads.contains(&admin_password));
        assert!(!audit_payloads.contains("$argon2id$"));

        let second_admin_view = service
            .list_users(&admin, admin_method, None)
            .await
            .unwrap()
            .into_iter()
            .find(|user| user.id == second_admin.id)
            .unwrap();
        assert_eq!(second_admin_view.lab_role, Some(LabRole::LabAdmin));
    }
}
