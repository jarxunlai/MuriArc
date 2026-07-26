#![forbid(unsafe_code)]

mod ai_source_resolver;

mod ai_data_tools;
mod ai_secrets;
mod ai_step_up;
mod auth;
#[cfg(all(feature = "postgres", test))]
mod bootstrap;
mod deployment_security;
#[cfg(feature = "postgres")]
mod environment_root;
mod error;
mod jobs;
mod mcp;
#[cfg(feature = "postgres")]
mod persistent_auth;
mod routes;
mod technical_logs;
#[cfg(feature = "postgres")]
mod user_governance;

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use ai_step_up::AiStepUpRateLimiter;
use muriarc_core::{AiModelProfileStore, AiOperationStore, MuriArcStore};
use muriarc_data::DataFiles;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeAccessMode {
    #[default]
    ReadWrite,
    ReadOnlyActivation,
}

pub use ai_secrets::{
    AiLabSettingsView, AiModelDefaultsView, AiModelProfileView, AiModelValidationView,
    AiProviderDiagnosticsView, AiProviderEndpointView, AiProviderModelPresetView,
    AiProviderPresetView, AiProviderSettingsView, AiProviderStoreError, ArchiveAiModelProfileInput,
    DisabledAiProviderStore, ResolvedAiProvider, SaveAiLabSettingsInput, SaveAiModelDefaultsInput,
    SaveAiModelProfileInput, SaveAiProviderEndpointInput, SaveAiProviderSettingsInput,
    SensitiveSecret, UserAiProviderStore, ValidateAiModelProfileInput,
};
#[cfg(feature = "postgres")]
pub use ai_secrets::{AiMasterKey, PostgresAiProviderStore};
pub use auth::{
    AuthError, AuthPrincipal, AuthenticatedSession, AuthenticationMethod, Authenticator,
    ChainedAuthenticator, DisabledSessionBackend, ExternalTokenSummary, NewExternalToken,
    NewSession, RequestMetadata, SESSION_COOKIE_NAME, SessionBackend, SessionCookieConfig,
    StaticTokenAuthenticator, derive_csrf_token, generate_secret_token, token_hash,
};
#[cfg(all(feature = "postgres", test))]
pub use bootstrap::{
    BootstrapSeedConfig, BootstrapSeedError, BootstrapSeedOutcome, seed_postgres_bootstrap,
};
pub use deployment_security::{
    CLOUDFLARE_ATTACHMENT_MAX_BYTES, CredentialPolicy, DeploymentProfile, DeploymentSecurityPolicy,
    ExternalApiPolicy, RuntimeCapabilities,
};
#[cfg(feature = "postgres")]
pub use environment_root::{
    EnvironmentRootConfig, EnvironmentRootError, EnvironmentRootOutcome,
    sync_postgres_environment_root,
};
pub use error::{ApiError, ErrorEnvelope, ErrorPayload};
pub use jobs::{
    InMemoryJobRepository, JobCreateOutcome, JobRepository, JobRepositoryError, StoreJobRepository,
};
#[cfg(feature = "postgres")]
pub use persistent_auth::{LiveBootstrapAuthenticator, PostgresAuthBackend, hash_password};
pub use routes::{api_router, application_router};
#[cfg(feature = "postgres")]
pub use technical_logs::PostgresTechnicalLogService;
pub use technical_logs::{
    DisabledTechnicalLogService, SaveTechnicalLogPolicyInput, TechnicalLogCleanupPreview,
    TechnicalLogError, TechnicalLogEvent, TechnicalLogPolicyView, TechnicalLogService,
};
#[cfg(feature = "postgres")]
pub use user_governance::{
    AdminMutationContext, CreateManagedUserCommand, InitialProjectRole, ManagedProjectMembership,
    ManagedUser, PostgresUserGovernance, SensitivePassword, UserGovernanceError,
};

/// Dependencies shared by all Axum handlers.
///
/// Production bootstrapping supplies a PostgreSQL store and a durable
/// authenticator. Tests and local development can use explicit in-memory
/// adapters without weakening the HTTP security boundary.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn MuriArcStore>,
    pub authenticator: Arc<dyn Authenticator>,
    pub sessions: Arc<dyn SessionBackend>,
    pub session_cookie: SessionCookieConfig,
    pub deployment_security: Arc<DeploymentSecurityPolicy>,
    pub jobs: Arc<dyn JobRepository>,
    pub ai_operations: Option<Arc<dyn AiOperationStore>>,
    pub ai_model_profiles: Option<Arc<dyn AiModelProfileStore>>,
    pub ai_providers: Arc<dyn UserAiProviderStore>,
    pub(crate) ai_step_up: AiStepUpRateLimiter,
    pub data_files: Option<Arc<DataFiles>>,
    pub attachment_root: Option<Arc<PathBuf>>,
    pub ui_root: Option<Arc<PathBuf>>,
    pub runtime_compatibility_verified: bool,
    pub runtime_access_mode: RuntimeAccessMode,
    pub(crate) admin_private_views: Arc<RwLock<HashSet<(uuid::Uuid, uuid::Uuid)>>>,
    pub technical_logs: Arc<dyn TechnicalLogService>,
    #[cfg(feature = "postgres")]
    pub user_governance: Option<Arc<PostgresUserGovernance>>,
}

impl AppState {
    pub fn new(
        store: Arc<dyn MuriArcStore>,
        authenticator: Arc<dyn Authenticator>,
        jobs: Arc<dyn JobRepository>,
    ) -> Self {
        Self {
            store,
            authenticator,
            sessions: Arc::new(DisabledSessionBackend),
            session_cookie: SessionCookieConfig::default(),
            deployment_security: Arc::new(DeploymentSecurityPolicy::development_default()),
            jobs,
            ai_operations: None,
            ai_model_profiles: None,
            ai_providers: Arc::new(DisabledAiProviderStore),
            ai_step_up: AiStepUpRateLimiter::default(),
            data_files: None,
            attachment_root: None,
            ui_root: None,
            runtime_compatibility_verified: false,
            runtime_access_mode: RuntimeAccessMode::ReadWrite,
            admin_private_views: Arc::new(RwLock::new(HashSet::new())),
            technical_logs: Arc::new(DisabledTechnicalLogService),
            #[cfg(feature = "postgres")]
            user_governance: None,
        }
    }

    pub fn with_sessions(
        mut self,
        sessions: Arc<dyn SessionBackend>,
        session_cookie: SessionCookieConfig,
    ) -> Self {
        self.sessions = sessions;
        self.session_cookie = session_cookie;
        self
    }

    pub fn with_deployment_security(mut self, policy: DeploymentSecurityPolicy) -> Self {
        self.deployment_security = Arc::new(policy);
        self
    }

    pub fn with_data_storage(
        mut self,
        files: DataFiles,
        attachment_root: impl Into<PathBuf>,
    ) -> Self {
        self.data_files = Some(Arc::new(files));
        self.attachment_root = Some(Arc::new(attachment_root.into()));
        self
    }

    pub fn with_runtime_compatibility(
        mut self,
        ui_root: Option<PathBuf>,
        access_mode: RuntimeAccessMode,
    ) -> Self {
        self.ui_root = ui_root.map(Arc::new);
        self.runtime_compatibility_verified = true;
        self.runtime_access_mode = access_mode;
        self
    }

    pub fn with_ai(
        mut self,
        operations: Arc<dyn AiOperationStore>,
        model_profiles: Arc<dyn AiModelProfileStore>,
        providers: Arc<dyn UserAiProviderStore>,
    ) -> Self {
        self.ai_operations = Some(operations);
        self.ai_model_profiles = Some(model_profiles);
        self.ai_providers = providers;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_ai_step_up_limiter(mut self, limiter: AiStepUpRateLimiter) -> Self {
        self.ai_step_up = limiter;
        self
    }

    #[cfg(feature = "postgres")]
    pub fn with_user_governance(mut self, governance: PostgresUserGovernance) -> Self {
        self.user_governance = Some(Arc::new(governance));
        self
    }

    pub fn with_technical_logs(mut self, service: Arc<dyn TechnicalLogService>) -> Self {
        self.technical_logs = service;
        self
    }
}
