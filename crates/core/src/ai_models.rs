use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RecordMeta;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderProtocol {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
}

impl Default for AiProviderProtocol {
    fn default() -> Self {
        Self::OpenaiChatCompletions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderTransport {
    OpenAiCompatible,
    LocalHttp,
}

impl Default for AiProviderTransport {
    fn default() -> Self {
        Self::OpenAiCompatible
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiModelPurpose {
    Conversation,
    Vision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiModelProfile {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub current_version: i64,
    pub archived_at: Option<DateTime<Utc>>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiModelProfileVersion {
    pub profile_id: Uuid,
    pub version: i64,
    pub protocol: AiProviderProtocol,
    pub transport: AiProviderTransport,
    pub base_url: String,
    pub normalized_base_url: String,
    pub model_id: String,
    pub supports_vision: bool,
    pub context_window_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub history_token_budget: u32,
    pub history_turns: u32,
    pub temperature: f32,
    pub timeout_ms: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiUserModelDefaults {
    pub user_id: Uuid,
    pub default_conversation_profile_id: Option<Uuid>,
    pub default_vision_profile_id: Option<Uuid>,
    pub meta: RecordMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiModelCredentialState {
    Present,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModelProfileSecretRef {
    pub profile_id: Uuid,
    pub profile_version: i64,
    pub keyring_account: String,
    pub credential_state: AiModelCredentialState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModelProfileBinding {
    pub profile_id: Uuid,
    pub profile_version: i64,
}
