use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    super::{
        config::ProviderError,
        types::{CompletionRequest, CompletionResponse, ProviderToolCall, TokenUsage, valid_token},
    },
    serialize_json,
};

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireRequestMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize)]
struct WireRequestMessage<'a> {
    role: super::super::types::ChatRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<WireContent<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireRequestToolCall<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireContent<'a> {
    Text(&'a str),
    Parts(Vec<WireContentPart<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentPart<'a> {
    Text { text: &'a str },
    ImageUrl { image_url: WireImageUrl },
}

#[derive(Serialize)]
struct WireImageUrl {
    url: String,
    detail: &'static str,
}

#[derive(Serialize)]
struct WireRequestToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireRequestToolFunction<'a>,
}

#[derive(Serialize)]
struct WireRequestToolFunction<'a> {
    name: &'a str,
    arguments: String,
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction<'a>,
}

#[derive(Serialize)]
struct WireFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

#[derive(Deserialize)]
struct WireResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<WireChoice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireResponseMessage {
    content: Option<String>,
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCall>,
}

#[derive(Deserialize)]
struct WireToolCall {
    id: String,
    function: WireToolFunction,
}

#[derive(Deserialize)]
struct WireToolFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

pub(super) fn serialize_request(
    model: &str,
    request: &CompletionRequest,
) -> Result<Vec<u8>, ProviderError> {
    let messages = request
        .messages
        .iter()
        .map(|message| WireRequestMessage {
            role: message.role,
            content: if message.images.is_empty() {
                (!message.content.is_empty()).then_some(WireContent::Text(message.content.as_str()))
            } else {
                let mut parts = Vec::with_capacity(message.images.len() + 1);
                if !message.content.is_empty() {
                    parts.push(WireContentPart::Text {
                        text: message.content.as_str(),
                    });
                }
                parts.extend(
                    message
                        .images
                        .iter()
                        .map(|image| WireContentPart::ImageUrl {
                            image_url: WireImageUrl {
                                url: format!(
                                    "data:{};base64,{}",
                                    image.media_type, image.data_base64
                                ),
                                detail: "high",
                            },
                        }),
                );
                Some(WireContent::Parts(parts))
            },
            tool_call_id: message.tool_call_id.as_deref(),
            tool_calls: message
                .tool_calls
                .iter()
                .map(|tool_call| WireRequestToolCall {
                    id: tool_call.id.as_str(),
                    kind: "function",
                    function: WireRequestToolFunction {
                        name: tool_call.name.as_str(),
                        arguments: tool_call.arguments.to_string(),
                    },
                })
                .collect(),
        })
        .collect();
    let tools = request
        .tools
        .iter()
        .map(|tool| WireTool {
            kind: "function",
            function: WireFunction {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.parameters,
            },
        })
        .collect();
    serialize_json(&WireRequest {
        model,
        messages,
        tools,
        temperature: request.temperature,
        max_tokens: request.max_output_tokens,
        stream: false,
    })
}

pub(super) fn parse_response(bytes: &[u8]) -> Result<CompletionResponse, ProviderError> {
    let wire: WireResponse =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::MalformedResponse)?;
    let choice = wire
        .choices
        .into_iter()
        .next()
        .ok_or(ProviderError::MalformedResponse)?;
    let finish_reason = choice.finish_reason;
    let content = choice
        .message
        .content
        .or(choice.message.refusal)
        .filter(|content| !content.trim().is_empty());
    let tool_calls = choice
        .message
        .tool_calls
        .into_iter()
        .map(|tool_call| {
            if !valid_token(&tool_call.id, 128) || !valid_token(&tool_call.function.name, 64) {
                return Err(ProviderError::MalformedResponse);
            }
            let arguments: Value = serde_json::from_str(&tool_call.function.arguments)
                .map_err(|_| ProviderError::MalformedResponse)?;
            if !arguments.is_object() {
                return Err(ProviderError::MalformedResponse);
            }
            Ok(ProviderToolCall {
                id: tool_call.id,
                name: tool_call.function.name,
                arguments,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if content.is_none() && tool_calls.is_empty() {
        return if finish_reason.as_deref() == Some("length") {
            Err(ProviderError::OutputBudgetExhausted)
        } else {
            Err(ProviderError::EmptyResponse)
        };
    }
    let usage = wire.usage.map(|usage| TokenUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
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
