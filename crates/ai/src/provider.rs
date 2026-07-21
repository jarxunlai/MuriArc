mod config;
mod http;
#[cfg(any(test, feature = "test-support"))]
mod mock;
mod types;

pub use config::{
    CredentialError, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS, MAX_MAX_RESPONSE_BYTES,
    MAX_REQUEST_BYTES, MAX_TIMEOUT_MS, MIN_MAX_RESPONSE_BYTES, MIN_TIMEOUT_MS, ProviderConfig,
    ProviderConfigError, ProviderCredentials, ProviderError, ProviderKind, TransportFailure,
};
pub use http::{BuiltinProvider, LocalHttpProvider, OpenAiCompatibleProvider};
#[cfg(any(test, feature = "test-support"))]
pub use mock::MockProvider;
pub use types::{
    AiProvider, ChatMessage, ChatRole, CompletionRequest, CompletionResponse,
    MAX_VISION_IMAGE_BASE64_BYTES, MAX_VISION_IMAGES, MAX_VISION_TOTAL_BASE64_BYTES,
    ProviderToolCall, TokenUsage, ToolDefinition, VisionImageInput,
};

#[cfg(test)]
mod tests;
