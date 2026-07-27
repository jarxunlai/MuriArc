use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{ProviderCredentials, ProviderError};
const MAX_MESSAGES: usize = 256;
pub const MAX_VISION_IMAGES: usize = 8;
pub const MAX_VISION_IMAGE_BASE64_BYTES: usize = 14 * 1024 * 1024;
pub const MAX_VISION_TOTAL_BASE64_BYTES: usize = 56 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_TOOLS: usize = 64;
const MAX_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_TOKENS: u32 = 131_072;
const CONNECTION_CHECK_MAX_OUTPUT_TOKENS: u32 = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionImageInput {
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<VisionImageInput>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn assistant_tool_calls(
        content: Option<String>,
        tool_calls: Vec<ProviderToolCall>,
    ) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.unwrap_or_default(),
            tool_call_id: None,
            tool_calls,
            images: Vec::new(),
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<VisionImageInput>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            images,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: Vec::new(),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

impl CompletionRequest {
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
        }
    }

    /// Builds the transport-neutral request used to verify a Provider connection.
    ///
    /// The output budget deliberately leaves room for reasoning-capable models
    /// that may spend tokens on hidden reasoning before emitting the requested
    /// short final answer. Keeping this policy in the AI boundary prevents Server
    /// and Desktop validation behavior from drifting apart.
    pub fn provider_connection_check() -> Self {
        let mut request = Self::new(vec![ChatMessage::user(
            "Connection check. Reply with the single word OK.",
        )]);
        request.max_output_tokens = Some(CONNECTION_CHECK_MAX_OUTPUT_TOKENS);
        request.temperature = Some(0.0);
        request
    }

    pub(super) fn validate(&self) -> Result<(), ProviderError> {
        if self.messages.is_empty() || self.messages.len() > MAX_MESSAGES {
            return Err(ProviderError::InvalidRequest("invalid message count"));
        }
        for message in &self.messages {
            let assistant_tool_call =
                message.role == ChatRole::Assistant && !message.tool_calls.is_empty();
            if (!assistant_tool_call
                && message.content.trim().is_empty()
                && message.images.is_empty())
                || message.content.len() > MAX_MESSAGE_BYTES
            {
                return Err(ProviderError::InvalidRequest("invalid message content"));
            }
            if !message.images.is_empty() && message.role != ChatRole::User {
                return Err(ProviderError::InvalidRequest(
                    "vision images are only allowed in user messages",
                ));
            }
            if message.images.len() > MAX_VISION_IMAGES {
                return Err(ProviderError::InvalidRequest("too many vision images"));
            }
            let mut total_image_bytes = 0_usize;
            for image in &message.images {
                if !matches!(
                    image.media_type.as_str(),
                    "image/jpeg"
                        | "image/png"
                        | "image/webp"
                        | "image/gif"
                        | "image/bmp"
                        | "image/tiff"
                        | "image/heic"
                        | "image/heif"
                ) || image.data_base64.is_empty()
                    || image.data_base64.len() > MAX_VISION_IMAGE_BASE64_BYTES
                    || !image.data_base64.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                    })
                {
                    return Err(ProviderError::InvalidRequest("invalid vision image"));
                }
                total_image_bytes = total_image_bytes
                    .checked_add(image.data_base64.len())
                    .ok_or(ProviderError::InvalidRequest("vision images are too large"))?;
            }
            if total_image_bytes > MAX_VISION_TOTAL_BASE64_BYTES {
                return Err(ProviderError::InvalidRequest("vision images are too large"));
            }
            match (
                message.role,
                message.tool_call_id.as_deref(),
                message.tool_calls.is_empty(),
            ) {
                (ChatRole::Tool, Some(id), true) if valid_token(id, 128) => {}
                (ChatRole::Tool, _, _) => {
                    return Err(ProviderError::InvalidRequest(
                        "tool messages require one valid call id and no tool calls",
                    ));
                }
                (ChatRole::Assistant, None, _) => {
                    validate_tool_calls(&message.tool_calls)?;
                }
                (_, None, true) => {}
                (_, _, _) => {
                    return Err(ProviderError::InvalidRequest(
                        "invalid tool call fields for message role",
                    ));
                }
            }
        }
        if let Some(temperature) = self.temperature
            && (!temperature.is_finite() || !(0.0..=2.0).contains(&temperature))
        {
            return Err(ProviderError::InvalidRequest("invalid temperature"));
        }
        if self
            .max_output_tokens
            .is_some_and(|value| value == 0 || value > MAX_OUTPUT_TOKENS)
        {
            return Err(ProviderError::InvalidRequest(
                "invalid maximum output token count",
            ));
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(ProviderError::InvalidRequest("too many tools"));
        }
        for (index, tool) in self.tools.iter().enumerate() {
            if !valid_token(&tool.name, 64)
                || tool.description.trim().is_empty()
                || tool.description.len() > 4096
                || !tool.parameters.is_object()
                || contains_raw_sql_key(&tool.parameters)
            {
                return Err(ProviderError::InvalidRequest(
                    "invalid or unsafe tool definition",
                ));
            }
            if self.tools[..index]
                .iter()
                .any(|existing| existing.name == tool.name)
            {
                return Err(ProviderError::InvalidRequest("duplicate tool name"));
            }
            let schema_size = serde_json::to_vec(&tool.parameters)
                .map_err(|_| ProviderError::InvalidRequest("invalid tool schema"))?
                .len();
            if schema_size > MAX_TOOL_SCHEMA_BYTES {
                return Err(ProviderError::InvalidRequest("tool schema is too large"));
            }
        }
        Ok(())
    }
}

fn validate_tool_calls(tool_calls: &[ProviderToolCall]) -> Result<(), ProviderError> {
    if tool_calls.len() > MAX_TOOLS {
        return Err(ProviderError::InvalidRequest("too many tool calls"));
    }
    for (index, tool_call) in tool_calls.iter().enumerate() {
        if !valid_token(&tool_call.id, 128)
            || !valid_token(&tool_call.name, 64)
            || !tool_call.arguments.is_object()
            || contains_raw_sql_key(&tool_call.arguments)
        {
            return Err(ProviderError::InvalidRequest("invalid or unsafe tool call"));
        }
        if tool_calls[..index]
            .iter()
            .any(|existing| existing.id == tool_call.id)
        {
            return Err(ProviderError::InvalidRequest("duplicate tool call id"));
        }
    }
    Ok(())
}

pub(super) fn valid_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn contains_raw_sql_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = key
                .chars()
                .filter(|character| !matches!(character, '_' | '-'))
                .flat_map(char::to_lowercase)
                .collect::<String>();
            matches!(normalized.as_str(), "sql" | "rawsql" | "querysql")
                || contains_raw_sql_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_raw_sql_key),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Vec<ProviderToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model(&self) -> &str;

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError>;
}
