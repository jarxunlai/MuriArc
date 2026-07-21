use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;

use super::{
    config::{ProviderCredentials, ProviderError},
    types::{AiProvider, CompletionRequest, CompletionResponse},
};
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct MockProvider {
    provider_id: String,
    model: String,
    responses: Arc<Mutex<VecDeque<Result<CompletionResponse, ProviderError>>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl MockProvider {
    pub fn new(
        provider_id: impl Into<String>,
        model: impl Into<String>,
        responses: impl IntoIterator<Item = Result<CompletionResponse, ProviderError>>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            model: model.into(),
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn requests(&self) -> Result<Vec<CompletionRequest>, ProviderError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| ProviderError::MockUnavailable)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl AiProvider for MockProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        request.validate()?;
        let _ = credentials;
        self.requests
            .lock()
            .map_err(|_| ProviderError::MockUnavailable)?
            .push(request);
        self.responses
            .lock()
            .map_err(|_| ProviderError::MockUnavailable)?
            .pop_front()
            .ok_or(ProviderError::MockExhausted)?
    }
}
