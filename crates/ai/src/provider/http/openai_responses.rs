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

#[derive(Deserialize)]
struct WireResponse {
    id: Option<String>,
    model: Option<String>,
    status: Option<String>,
    #[serde(default)]
    output: Vec<WireOutputItem>,
    usage: Option<WireUsage>,
    incomplete_details: Option<WireIncompleteDetails>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireOutputItem {
    #[serde(rename = "message")]
    Message {
        #[serde(default)]
        content: Vec<WireOutputContent>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: Option<String>,
        call_id: Option<String>,
        name: String,
        arguments: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireOutputContent {
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    total_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct WireIncompleteDetails {
    reason: Option<String>,
}

pub(super) fn serialize_request(
    model: &str,
    request: &CompletionRequest,
) -> Result<Vec<u8>, ProviderError> {
    let mut input = Vec::new();
    for message in &request.messages {
        match message.role {
            ChatRole::System | ChatRole::User => {
                let role = match message.role {
                    ChatRole::System => "system",
                    ChatRole::User => "user",
                    _ => unreachable!(),
                };
                let content = if message.images.is_empty() {
                    Value::String(message.content.clone())
                } else {
                    let mut parts = Vec::with_capacity(message.images.len() + 1);
                    if !message.content.is_empty() {
                        parts.push(json!({
                            "type": "input_text",
                            "text": message.content
                        }));
                    }
                    parts.extend(message.images.iter().map(|image| {
                        json!({
                            "type": "input_image",
                            "image_url": format!(
                                "data:{};base64,{}",
                                image.media_type, image.data_base64
                            ),
                            "detail": "high"
                        })
                    }));
                    Value::Array(parts)
                };
                input.push(json!({
                    "role": role,
                    "content": content
                }));
            }
            ChatRole::Assistant => {
                if !message.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": message.content
                    }));
                }
                input.extend(message.tool_calls.iter().map(|tool_call| {
                    json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.arguments.to_string()
                    })
                }));
            }
            ChatRole::Tool => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": message.tool_call_id,
                    "output": message.content
                }));
            }
        }
    }

    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": false
            })
        })
        .collect::<Vec<_>>();
    let mut wire = Map::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        ("input".to_owned(), Value::Array(input)),
        ("tools".to_owned(), Value::Array(tools)),
        ("stream".to_owned(), Value::Bool(false)),
        ("store".to_owned(), Value::Bool(false)),
    ]);
    if let Some(temperature) = request.temperature {
        wire.insert("temperature".to_owned(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        wire.insert("max_output_tokens".to_owned(), json!(max_output_tokens));
    }
    serialize_json(&wire)
}

pub(super) fn parse_response(bytes: &[u8]) -> Result<CompletionResponse, ProviderError> {
    let wire: WireResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::MalformedResponse)?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for item in wire.output {
        match item {
            WireOutputItem::Message {
                content: content_items,
            } => {
                for item in content_items {
                    match item {
                        WireOutputContent::OutputText { text } => content.push_str(&text),
                        WireOutputContent::Refusal { refusal } => content.push_str(&refusal),
                        WireOutputContent::Other => {}
                    }
                }
            }
            WireOutputItem::FunctionCall {
                id,
                call_id,
                name,
                arguments,
            } => {
                let id = call_id.or(id).ok_or(ProviderError::MalformedResponse)?;
                if !valid_token(&id, 128) || !valid_token(&name, 64) {
                    return Err(ProviderError::MalformedResponse);
                }
                let arguments: Value = serde_json::from_str(&arguments)
                    .map_err(|_| ProviderError::MalformedResponse)?;
                if !arguments.is_object() {
                    return Err(ProviderError::MalformedResponse);
                }
                tool_calls.push(ProviderToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            WireOutputItem::Other => {}
        }
    }
    let content = (!content.trim().is_empty()).then_some(content);
    let incomplete_reason = wire.incomplete_details.and_then(|details| details.reason);
    if content.is_none() && tool_calls.is_empty() {
        return if incomplete_reason.as_deref() == Some("max_output_tokens") {
            Err(ProviderError::OutputBudgetExhausted)
        } else {
            Err(ProviderError::EmptyResponse)
        };
    }
    let finish_reason = incomplete_reason.or_else(|| {
        if !tool_calls.is_empty() {
            Some("tool_calls".to_owned())
        } else if wire.status.as_deref() == Some("completed") {
            Some("stop".to_owned())
        } else {
            wire.status
        }
    });
    let usage = wire.usage.map(|usage| {
        let computed_total = usage.input_tokens.saturating_add(usage.output_tokens);
        TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens.unwrap_or(computed_total),
        }
    });
    Ok(CompletionResponse {
        id: wire.id,
        model: wire.model,
        content,
        tool_calls,
        finish_reason,
        usage,
    })
}
