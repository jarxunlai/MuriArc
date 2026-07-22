use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use super::*;

fn response(content: &str) -> CompletionResponse {
    CompletionResponse {
        id: Some("mock-response".to_owned()),
        model: Some("mock-model".to_owned()),
        content: Some(content.to_owned()),
        tool_calls: Vec::new(),
        finish_reason: Some("stop".to_owned()),
        usage: None,
    }
}

fn spawn_http_server(
    status: &str,
    body: String,
    delay: Duration,
) -> (String, Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request_complete(&request) {
                break;
            }
        }
        sender
            .send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        thread::sleep(delay);
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Request-ID: safe-request-id\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(headers.as_bytes());
        let _ = stream.write_all(body.as_bytes());
    });
    (format!("http://{address}/v1"), receiver, handle)
}

fn request_complete(request: &[u8]) -> bool {
    let Some(header_end) = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    else {
        return false;
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    request.len() >= header_end + content_length
}

#[test]
fn credentials_are_never_serialized_or_debugged() {
    let credentials = ProviderCredentials::bearer("call-only-secret").unwrap();
    let debug = format!("{credentials:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("call-only-secret"));

    let config = ProviderConfig::local_http("local", "model", "http://127.0.0.1:11434/v1");
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("call-only-secret"));
    assert!(
        serde_json::from_value::<ProviderConfig>(serde_json::json!({
            "provider_id": "local",
            "kind": "local_http",
            "model": "model",
            "base_url": "http://127.0.0.1:11434/v1",
            "api_key": "must-not-be-stored"
        }))
        .is_err()
    );
}

#[test]
fn cloud_provider_requires_https_and_url_has_no_credentials() {
    let insecure =
        ProviderConfig::openai_compatible("cloud", "model", "http://api.example.test/v1");
    assert!(matches!(
        OpenAiCompatibleProvider::new(insecure),
        Err(ProviderConfigError::HttpsRequired)
    ));
    let credential_url =
        ProviderConfig::local_http("local", "model", "http://user:password@127.0.0.1:11434/v1");
    assert!(matches!(
        LocalHttpProvider::new(credential_url),
        Err(ProviderConfigError::InvalidBaseUrl)
    ));
}

#[tokio::test]
async fn mock_provider_records_requests_but_not_credentials() {
    let mock = MockProvider::new("mock", "mock-model", [Ok(response("hello"))]);
    let request = CompletionRequest::new(vec![ChatMessage::user("status")]);
    let result = mock
        .complete(
            request.clone(),
            ProviderCredentials::bearer("never-record-this").unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(result.content.as_deref(), Some("hello"));
    assert_eq!(mock.requests().unwrap(), vec![request]);
    assert!(matches!(
        mock.complete(
            CompletionRequest::new(vec![ChatMessage::user("again")]),
            ProviderCredentials::none()
        )
        .await,
        Err(ProviderError::MockExhausted)
    ));
}

#[tokio::test]
async fn local_http_provider_uses_openai_wire_format_and_call_credentials() {
    let body = serde_json::json!({
        "id": "response-1",
        "model": "local-model",
        "choices": [{
            "message": {"content": "3 animals", "tool_calls": []},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let mut config = ProviderConfig::local_http("local", "local-model", base_url);
    config.timeout_ms = 2_000;
    config.max_response_bytes = 4096;
    let provider = LocalHttpProvider::new(config).unwrap();
    let result = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user("count animals")]),
            ProviderCredentials::bearer("call-only-secret").unwrap(),
        )
        .await
        .unwrap();
    let request = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();

    assert_eq!(result.content.as_deref(), Some("3 animals"));
    assert_eq!(result.usage.unwrap().total_tokens, 12);
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer call-only-secret")
    );
    assert!(request.contains("\"model\":\"local-model\""));
}

#[tokio::test]
async fn deepseek_glm_and_kimi_compatible_requests_keep_credentials_and_models_isolated() {
    for (provider_id, model, api_key) in [
        ("deepseek", "deepseek-chat", "deepseek-key-a"),
        ("zhipu-glm", "glm-5.2", "glm-key-b"),
        ("moonshot-kimi", "kimi-k3", "kimi-key-c"),
    ] {
        let body = serde_json::json!({
            "id": format!("response-{provider_id}"),
            "model": model,
            "choices": [{
                "message": {"content": "OK", "tool_calls": []},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 11, "completion_tokens": 1, "total_tokens": 12}
        })
        .to_string();
        let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
        let provider =
            LocalHttpProvider::new(ProviderConfig::local_http(provider_id, model, base_url))
                .unwrap();
        let mut request = CompletionRequest::new(vec![ChatMessage::user("ping")]);
        request.max_output_tokens = Some(777);
        request.temperature = Some(0.4);

        let response = provider
            .complete(request, ProviderCredentials::bearer(api_key).unwrap())
            .await
            .unwrap();
        let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
        handle.join().unwrap();

        assert_eq!(response.model.as_deref(), Some(model));
        let authorization = captured
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then_some(value.trim())
            })
            .unwrap();
        assert_eq!(authorization, format!("Bearer {api_key}"));
        let payload: serde_json::Value = serde_json::from_str(
            captured
                .split_once("\r\n\r\n")
                .map(|(_, body)| body)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["model"], model);
        assert_eq!(payload["max_tokens"], 777);
        assert_eq!(payload["temperature"], 0.4);
    }
}

#[tokio::test]
async fn local_http_provider_serializes_assistant_tool_call_history() {
    let body = serde_json::json!({
        "id": "response-2",
        "model": "local-model",
        "choices": [{
            "message": {"content": "M001 is active", "tool_calls": []},
            "finish_reason": "stop"
        }]
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider =
        LocalHttpProvider::new(ProviderConfig::local_http("local", "local-model", base_url))
            .unwrap();
    let tool_call = ProviderToolCall {
        id: "call_1".to_owned(),
        name: "animal_search".to_owned(),
        arguments: serde_json::json!({"query": "M001"}),
    };
    let request = CompletionRequest::new(vec![
        ChatMessage::user("Find M001"),
        ChatMessage::assistant_tool_calls(None, vec![tool_call]),
        ChatMessage::tool("call_1", r#"{"kind":"read_result","data":{}}"#),
    ]);

    provider
        .complete(request, ProviderCredentials::none())
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();

    let payload: serde_json::Value = serde_json::from_str(
        captured
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        payload["messages"][1]["tool_calls"][0]["function"]["arguments"],
        serde_json::json!(r#"{"query":"M001"}"#)
    );
    assert_eq!(payload["messages"][2]["tool_call_id"], "call_1");
}

#[tokio::test]
async fn response_limit_is_enforced_before_reading_body() {
    let (base_url, captured, handle) = spawn_http_server(
        "200 OK",
        "x".repeat(MIN_MAX_RESPONSE_BYTES + 1),
        Duration::ZERO,
    );
    let mut config = ProviderConfig::local_http("local", "model", base_url);
    config.timeout_ms = 2_000;
    config.max_response_bytes = MIN_MAX_RESPONSE_BYTES;
    let provider = LocalHttpProvider::new(config).unwrap();
    let error = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user("hello")]),
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();
    let _ = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    assert_eq!(
        error,
        ProviderError::ResponseTooLarge {
            limit: MIN_MAX_RESPONSE_BYTES
        }
    );
}

#[tokio::test]
async fn timeout_and_http_errors_do_not_expose_provider_body_or_key() {
    let (base_url, captured, handle) = spawn_http_server(
        "200 OK",
        serde_json::json!({"choices": []}).to_string(),
        Duration::from_millis(250),
    );
    let mut config = ProviderConfig::local_http("local", "model", base_url);
    config.timeout_ms = MIN_TIMEOUT_MS;
    let provider = LocalHttpProvider::new(config).unwrap();
    let timeout = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user("hello")]),
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();
    let _ = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    assert_eq!(
        timeout,
        ProviderError::Transport {
            kind: TransportFailure::Timeout
        }
    );

    let secret_body = "server-secret-must-not-leak";
    let (base_url, captured, handle) =
        spawn_http_server("401 Unauthorized", secret_body.to_owned(), Duration::ZERO);
    let provider =
        LocalHttpProvider::new(ProviderConfig::local_http("local", "model", base_url)).unwrap();
    let error = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user("hello")]),
            ProviderCredentials::bearer("key-must-not-leak").unwrap(),
        )
        .await
        .unwrap_err();
    let _ = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    let displayed = error.to_string();
    assert!(!displayed.contains(secret_body));
    assert!(!displayed.contains("key-must-not-leak"));
    assert!(matches!(
        error,
        ProviderError::HttpStatus {
            status: 401,
            request_id: Some(_)
        }
    ));
}

#[tokio::test]
async fn raw_sql_tool_shapes_are_rejected_before_provider_call() {
    let mock = MockProvider::new("mock", "model", [Ok(response("unused"))]);
    let mut request = CompletionRequest::new(vec![ChatMessage::user("query")]);
    request.tools.push(ToolDefinition {
        name: "unsafe_tool".to_owned(),
        description: "Unsafe".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"raw_sql": {"type": "string"}}
        }),
    });
    assert!(matches!(
        mock.complete(request, ProviderCredentials::none()).await,
        Err(ProviderError::InvalidRequest(
            "invalid or unsafe tool definition"
        ))
    ));
}
