use std::time::Duration;

use async_trait::async_trait;
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_TYPE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    config::{
        MAX_REQUEST_BYTES, ProviderConfig, ProviderConfigError, ProviderCredentials, ProviderError,
        ProviderKind, TransportFailure,
    },
    types::{
        AiProvider, CompletionRequest, CompletionResponse, ProviderToolCall, TokenUsage,
        valid_token,
    },
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
    role: super::types::ChatRole,
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
#[derive(Clone)]
struct HttpProviderCore {
    config: ProviderConfig,
    endpoint: Url,
    client: Client,
}

impl HttpProviderCore {
    fn new(
        config: ProviderConfig,
        expected_kind: ProviderKind,
    ) -> Result<Self, ProviderConfigError> {
        let validated = config.validate(expected_kind)?;
        let timeout = Duration::from_millis(config.timeout_ms);
        let connect_timeout = timeout.min(Duration::from_secs(15));
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .redirect(Policy::none())
            .user_agent(concat!("MuriArc/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderConfigError::ClientInitialization)?;
        Ok(Self {
            config,
            endpoint: validated.endpoint,
            client,
        })
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        request.validate()?;
        let messages =
            request
                .messages
                .iter()
                .map(|message| WireRequestMessage {
                    role: message.role,
                    content: if message.images.is_empty() {
                        (!message.content.is_empty())
                            .then_some(WireContent::Text(message.content.as_str()))
                    } else {
                        let mut parts = Vec::with_capacity(message.images.len() + 1);
                        if !message.content.is_empty() {
                            parts.push(WireContentPart::Text {
                                text: message.content.as_str(),
                            });
                        }
                        parts.extend(message.images.iter().map(|image| {
                            WireContentPart::ImageUrl {
                                image_url: WireImageUrl {
                                    url: format!(
                                        "data:{};base64,{}",
                                        image.media_type, image.data_base64
                                    ),
                                    detail: "high",
                                },
                            }
                        }));
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
        let wire_request = WireRequest {
            model: &self.config.model,
            messages,
            tools,
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            stream: false,
        };
        let body = serde_json::to_vec(&wire_request)
            .map_err(|_| ProviderError::InvalidRequest("request is not serializable"))?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(ProviderError::RequestTooLarge {
                limit: MAX_REQUEST_BYTES,
            });
        }

        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(api_key) = credentials.api_key {
            builder = builder.bearer_auth(api_key);
        }
        let mut response = builder.send().await.map_err(map_transport_error)?;
        if !response.status().is_success() {
            return Err(status_error(&response));
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.config.max_response_bytes as u64)
        {
            return Err(ProviderError::ResponseTooLarge {
                limit: self.config.max_response_bytes,
            });
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_transport_error)? {
            let next_length = bytes.len().saturating_add(chunk.len());
            if next_length > self.config.max_response_bytes {
                return Err(ProviderError::ResponseTooLarge {
                    limit: self.config.max_response_bytes,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_response(&bytes)
    }
}

fn map_transport_error(error: reqwest::Error) -> ProviderError {
    let kind = if error.is_timeout() {
        TransportFailure::Timeout
    } else if error.is_connect() {
        TransportFailure::Connection
    } else {
        TransportFailure::Request
    };
    ProviderError::Transport { kind }
}

fn status_error(response: &reqwest::Response) -> ProviderError {
    let request_id = ["x-request-id", "request-id"]
        .into_iter()
        .find_map(|name| response.headers().get(name))
        .and_then(|value| value.to_str().ok())
        .and_then(sanitize_request_id);
    ProviderError::HttpStatus {
        status: response.status().as_u16(),
        request_id,
    }
}

fn sanitize_request_id(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
    {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_response(bytes: &[u8]) -> Result<CompletionResponse, ProviderError> {
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
        .filter(|content| !content.trim().is_empty());
    let tool_calls = choice
        .message
        .tool_calls
        .into_iter()
        .map(|tool_call| {
            if !valid_token(&tool_call.id, 128) || !valid_token(&tool_call.function.name, 64) {
                return Err(ProviderError::MalformedResponse);
            }
            let arguments = serde_json::from_str(&tool_call.function.arguments)
                .map_err(|_| ProviderError::MalformedResponse)?;
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

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    inner: HttpProviderCore,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: ProviderConfig) -> Result<Self, ProviderConfigError> {
        Ok(Self {
            inner: HttpProviderCore::new(config, ProviderKind::OpenAiCompatible)?,
        })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.inner.config
    }
}

#[async_trait]
impl AiProvider for OpenAiCompatibleProvider {
    fn provider_id(&self) -> &str {
        &self.inner.config.provider_id
    }

    fn model(&self) -> &str {
        &self.inner.config.model
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        self.inner.complete(request, credentials).await
    }
}

#[derive(Clone)]
pub struct LocalHttpProvider {
    inner: HttpProviderCore,
}

impl LocalHttpProvider {
    pub fn new(config: ProviderConfig) -> Result<Self, ProviderConfigError> {
        Ok(Self {
            inner: HttpProviderCore::new(config, ProviderKind::LocalHttp)?,
        })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.inner.config
    }
}

#[async_trait]
impl AiProvider for LocalHttpProvider {
    fn provider_id(&self) -> &str {
        &self.inner.config.provider_id
    }

    fn model(&self) -> &str {
        &self.inner.config.model
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        self.inner.complete(request, credentials).await
    }
}

#[derive(Clone)]
pub enum BuiltinProvider {
    OpenAiCompatible(OpenAiCompatibleProvider),
    LocalHttp(LocalHttpProvider),
}

impl BuiltinProvider {
    pub fn from_config(config: ProviderConfig) -> Result<Self, ProviderConfigError> {
        match config.kind {
            ProviderKind::OpenAiCompatible => Ok(Self::OpenAiCompatible(
                OpenAiCompatibleProvider::new(config)?,
            )),
            ProviderKind::LocalHttp => Ok(Self::LocalHttp(LocalHttpProvider::new(config)?)),
        }
    }

    pub fn config(&self) -> &ProviderConfig {
        match self {
            Self::OpenAiCompatible(provider) => provider.config(),
            Self::LocalHttp(provider) => provider.config(),
        }
    }
}

#[async_trait]
impl AiProvider for BuiltinProvider {
    fn provider_id(&self) -> &str {
        self.config().provider_id.as_str()
    }

    fn model(&self) -> &str {
        self.config().model.as_str()
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        match self {
            Self::OpenAiCompatible(provider) => provider.complete(request, credentials).await,
            Self::LocalHttp(provider) => provider.complete(request, credentials).await,
        }
    }
}
