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
pub mod source_context;
pub mod store_executor;
pub mod transport;
pub mod vision;
pub mod workflow;

pub use approval::{
    ApprovalDecision, ApprovalError, ApprovalRecord, ApprovalRequirement, DraftKind, DraftStatus,
    FieldChange, HumanApprover, ProposalActor, WriteDraft,
};
pub use assistant::{
    AssistantConfigError, AssistantError, AssistantIncompleteReason, AssistantLimits,
    AssistantRequest, AssistantResponse, AssistantRuntimeConfig, AssistantService, AssistantUsage,
    Citation, ContextManagementTrace, DomainToolExecutor, DomainToolOutput, DomainToolRequest,
    ToolExecutionError, ToolRunOutcome, ToolRunTrace, estimate_completion_input_tokens,
    fixed_tool_definitions,
};
pub use autonomy::AiActionPolicy;
pub use data_tools::{
    AiDataAccessContext, AiDataApplyResult, AiDataToolBackend, AiExportArtifactView,
    AiExportFormat, AiExportResource, AiSourceImportKind, ExportCreateArguments,
    ImportCommitDraftArguments, ImportCommitDraftPayload, ImportDraftIssueSeverity,
    ImportDraftPreviewIssue, ImportDraftPreviewRow, ImportDraftPreviewSummary,
    ImportPreviewArguments, SOURCE_IMPORT_JOB_BINDING_KEY, SourceImportJobBinding,
    SourceImportPreviewArguments, valid_sha256,
};
#[cfg(any(test, feature = "test-support"))]
pub use provider::MockProvider;
pub use provider::{
    AiProvider, AiProviderProtocol, BuiltinProvider, ChatMessage, ChatRole, CompletionRequest,
    CompletionResponse, CredentialError, LocalHttpProvider, MAX_VISION_IMAGE_BASE64_BYTES,
    MAX_VISION_IMAGES, MAX_VISION_TOTAL_BASE64_BYTES, OpenAiCompatibleProvider, ProviderConfig,
    ProviderConfigError, ProviderCredentials, ProviderError, ProviderKind, ProviderToolCall,
    TokenUsage, ToolDefinition, TransportFailure, VisionImageInput,
};
pub use query::{
    FilterClause, FilterOperator, PageSpec, QueryField, QueryRequest, QueryResource, QueryValue,
    SafeQuery, SortDirection, SortSpec, ValidationError,
};
pub use scopes::{AccessGrant, ScopeSet, ToolAuthorizationError, ToolName, ToolScope};
pub use source_context::{
    AssistantSourceBundle, AssistantSourceError, AssistantSourceResolutionRequest,
    AssistantSourceResolver, MAX_ASSISTANT_SOURCE_CONTEXT_BYTES, MAX_ASSISTANT_SOURCES,
    ResolvedAssistantSource,
};
pub use store_executor::{StoreDomainToolExecutor, StoreToolAccessContext};
pub use transport::{
    AiAutonomyUpdateRequest, AiAutonomyView, AiConversationReadOnlyReason,
    AssistantConversationDetail, AssistantConversationMessage, AssistantConversationSourceRef,
    AssistantConversationStartRequest, AssistantConversationStartResponse,
    AssistantConversationSummary, AssistantImageEvidence, AssistantModelCallPurpose,
    AssistantModelCallTrace, AssistantTrace, AssistantTurnRequest, AssistantTurnResponse,
    DraftDecisionRequest, WriteDraftSummary,
};
pub use vision::{
    DataCellVisionCandidate, DataCellVisionExtraction, DataCellVisionExtractionError,
    DataCellVisionExtractionRequest, MAX_SANITIZED_VISION_INPUT_BYTES, SanitizedVisionInput,
    VisionInputSanitizationError, extract_data_cell_vision, sanitize_vision_input,
};

pub use workflow::{
    AiExecutionContext, AiWorkflowError, AiWorkflowService, AssistantTurnMedia,
    AssistantVisionObservation, DraftDecisionResponse, PreparedAssistantImage,
};
