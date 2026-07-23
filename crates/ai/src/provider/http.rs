mod anthropic_messages;
mod openai_chat_completions;
mod openai_responses;

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_TYPE},
    redirect::Policy,
};
use serde::Serialize;

use super::{
    config::{
        AiProviderProtocol, MAX_REQUEST_BYTES, ProviderConfig, ProviderConfigError,
        ProviderCredentials, ProviderError, ProviderKind, TransportFailure,
    },
    types::{AiProvider, CompletionRequest, CompletionResponse},
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

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
        let body = match self.config.protocol {
            AiProviderProtocol::OpenaiChatCompletions => {
                openai_chat_completions::serialize_request(&self.config.model, &request)?
            }
            AiProviderProtocol::OpenaiResponses => {
                openai_responses::serialize_request(&self.config.model, &request)?
            }
            AiProviderProtocol::AnthropicMessages => {
                anthropic_messages::serialize_request(&self.config.model, &request)?
            }
        };
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
        match self.config.protocol {
            AiProviderProtocol::OpenaiChatCompletions | AiProviderProtocol::OpenaiResponses => {
                if let Some(api_key) = credentials.api_key {
                    builder = builder.bearer_auth(api_key);
                }
            }
            AiProviderProtocol::AnthropicMessages => {
                builder = builder.header("anthropic-version", ANTHROPIC_VERSION);
                if let Some(api_key) = credentials.api_key {
                    builder = builder.header("x-api-key", api_key);
                }
            }
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

        match self.config.protocol {
            AiProviderProtocol::OpenaiChatCompletions => {
                openai_chat_completions::parse_response(&bytes)
            }
            AiProviderProtocol::OpenaiResponses => openai_responses::parse_response(&bytes),
            AiProviderProtocol::AnthropicMessages => anthropic_messages::parse_response(&bytes),
        }
    }
}

fn serialize_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProviderError> {
    serde_json::to_vec(value)
        .map_err(|_| ProviderError::InvalidRequest("request is not serializable"))
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
