#![forbid(unsafe_code)]

//! Policy types used at the boundary between an AI model and MuriArc.
//!
//! This crate intentionally has no API that accepts or emits raw SQL. Models
//! submit a validated QueryRequest, select a fixed ToolName, and create a
//! human-reviewable WriteDraft for every mutation.

pub mod approval;
pub mod assistant;
pub mod autonomy;
pub mod data_tools;
pub mod provider;
pub mod query;
pub mod scopes;
pub mod store_executor;
pub mod transport;
pub mod workflow;

pub use approval::{
    ApprovalDecision, ApprovalError, ApprovalRecord, ApprovalRequirement, DraftKind, DraftStatus,
    FieldChange, HumanApprover, ProposalActor, WriteDraft,
};
pub use assistant::{
    AssistantConfigError, AssistantError, AssistantLimits, AssistantRequest, AssistantResponse,
    AssistantService, AssistantUsage, Citation, DomainToolExecutor, DomainToolOutput,
    DomainToolRequest, ToolExecutionError, ToolRunOutcome, ToolRunTrace, fixed_tool_definitions,
};
pub use autonomy::AiActionPolicy;
pub use data_tools::{
    AiDataAccessContext, AiDataApplyResult, AiDataToolBackend, AiExportFormat, AiExportResource,
    ExportCreateArguments, ImportCommitDraftArguments, ImportCommitDraftPayload,
    ImportPreviewArguments, valid_sha256,
};
#[cfg(any(test, feature = "test-support"))]
pub use provider::MockProvider;
pub use provider::{
    AiProvider, BuiltinProvider, ChatMessage, ChatRole, CompletionRequest, CompletionResponse,
    CredentialError, LocalHttpProvider, MAX_VISION_IMAGE_BASE64_BYTES, MAX_VISION_IMAGES,
    MAX_VISION_TOTAL_BASE64_BYTES, OpenAiCompatibleProvider, ProviderConfig, ProviderConfigError,
    ProviderCredentials, ProviderError, ProviderKind, ProviderToolCall, TokenUsage, ToolDefinition,
    TransportFailure, VisionImageInput,
};
pub use query::{
    FilterClause, FilterOperator, PageSpec, QueryField, QueryRequest, QueryResource, QueryValue,
    SafeQuery, SortDirection, SortSpec, ValidationError,
};
pub use scopes::{AccessGrant, ScopeSet, ToolAuthorizationError, ToolName, ToolScope};
pub use store_executor::{StoreDomainToolExecutor, StoreToolAccessContext};
pub use transport::{
    AiAutonomyUpdateRequest, AiAutonomyView, AssistantConversationDetail,
    AssistantConversationMessage, AssistantConversationSummary, AssistantTrace,
    AssistantTurnRequest, AssistantTurnResponse, DraftDecisionRequest, WriteDraftSummary,
};

pub use workflow::{AiExecutionContext, AiWorkflowError, AiWorkflowService, DraftDecisionResponse};
