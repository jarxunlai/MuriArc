use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{
    super::{
        config::ProviderError,
        types::{
            ChatRole, CompletionRequest, CompletionResponse, ProviderToolCall, TokenUsage,
            valid_token,
        },
    },
    serialize_json,
};

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;

#[derive(Deserialize)]
struct WireResponse {
    id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    content: Vec<WireContent>,
    stop_reason: Option<String>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

pub(super) fn serialize_request(
    model: &str,
    request: &CompletionRequest,
) -> Result<Vec<u8>, ProviderError> {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        match message.role {
            ChatRole::System => system.push(message.content.as_str()),
            ChatRole::User => {
                let mut content = Vec::with_capacity(message.images.len() + 1);
                if !message.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": message.content
                    }));
                }
                for image in &message.images {
                    if !matches!(
                        image.media_type.as_str(),
                        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                    ) {
                        return Err(ProviderError::InvalidRequest(
                            "image media type is unsupported by Anthropic Messages",
                        ));
                    }
                    content.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": image.media_type,
                            "data": image.data_base64
                        }
                    }));
                }
                messages.push(json!({
                    "role": "user",
                    "content": content
                }));
            }
            ChatRole::Assistant => {
                let mut content = Vec::with_capacity(message.tool_calls.len() + 1);
                if !message.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": message.content
                    }));
                }
                content.extend(message.tool_calls.iter().map(|tool_call| {
                    json!({
                        "type": "tool_use",
                        "id": tool_call.id,
                        "name": tool_call.name,
                        "input": tool_call.arguments
                    })
                }));
                messages.push(json!({
                    "role": "assistant",
                    "content": content
                }));
            }
            ChatRole::Tool => {
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id,
                        "content": message.content
                    }]
                }));
            }
        }
    }
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters
            })
        })
        .collect::<Vec<_>>();
    let mut wire = Map::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        ("messages".to_owned(), Value::Array(messages)),
        ("tools".to_owned(), Value::Array(tools)),
        (
            "max_tokens".to_owned(),
            json!(
                request
                    .max_output_tokens
                    .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
            ),
        ),
        ("stream".to_owned(), Value::Bool(false)),
    ]);
    if !system.is_empty() {
        wire.insert("system".to_owned(), Value::String(system.join("\n\n")));
    }
    if let Some(temperature) = request.temperature {
        wire.insert("temperature".to_owned(), json!(temperature));
    }
    serialize_json(&wire)
}

pub(super) fn parse_response(bytes: &[u8]) -> Result<CompletionResponse, ProviderError> {
    let wire: WireResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::MalformedResponse)?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for item in wire.content {
        match item {
            WireContent::Text { text } => content.push_str(&text),
            WireContent::ToolUse { id, name, input } => {
                if !valid_token(&id, 128) || !valid_token(&name, 64) || !input.is_object() {
                    return Err(ProviderError::MalformedResponse);
                }
                tool_calls.push(ProviderToolCall {
                    id,
                    name,
                    arguments: input,
                });
            }
            WireContent::Other => {}
        }
    }
    let content = (!content.trim().is_empty()).then_some(content);
    if content.is_none() && tool_calls.is_empty() {
        return if wire.stop_reason.as_deref() == Some("max_tokens") {
            Err(ProviderError::OutputBudgetExhausted)
        } else {
            Err(ProviderError::EmptyResponse)
        };
    }
    let usage = wire.usage.map(|usage| {
        let input_tokens = usage
            .input_tokens
            .saturating_add(usage.cache_creation_input_tokens)
            .saturating_add(usage.cache_read_input_tokens);
        TokenUsage {
            input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: input_tokens.saturating_add(usage.output_tokens),
        }
    });
    Ok(CompletionResponse {
        id: wire.id,
        model: wire.model,
        content,
        tool_calls,
        finish_reason: wire.stop_reason,
        usage,
    })
}
