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

fn captured_payload(request: &str) -> serde_json::Value {
    serde_json::from_str(
        request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap(),
    )
    .unwrap()
}

fn captured_header<'a>(request: &'a str, expected_name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(expected_name)
            .then_some(value.trim())
    })
}

#[test]
fn credentials_are_never_serialized_or_debugged() {
    let credentials = ProviderCredentials::bearer("call-only-secret").unwrap();
    let debug = format!("{credentials:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("call-only-secret"));

    let config = ProviderConfig::local_http("local", "model", "http://127.0.0.1:11434/v1");
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains(r#""protocol":"openai_chat_completions""#));
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
fn provider_protocol_is_persisted_and_legacy_config_defaults_to_chat_completions() {
    let legacy = serde_json::from_value::<ProviderConfig>(serde_json::json!({
        "provider_id": "local",
        "kind": "local_http",
        "model": "legacy-model",
        "base_url": "http://127.0.0.1:11434/v1"
    }))
    .unwrap();
    assert_eq!(legacy.protocol, AiProviderProtocol::OpenaiChatCompletions);

    let responses = ProviderConfig::local_http_with_protocol(
        "local",
        AiProviderProtocol::OpenaiResponses,
        "responses-model",
        "http://127.0.0.1:11434/v1",
    );
    assert_eq!(responses.protocol, AiProviderProtocol::OpenaiResponses);
    let serialized = serde_json::to_value(&responses).unwrap();
    assert_eq!(serialized["protocol"], "openai_responses");

    let anthropic = ProviderConfig::openai_compatible(
        "anthropic",
        "claude-model",
        "https://api.anthropic.com/v1",
    )
    .with_protocol(AiProviderProtocol::AnthropicMessages);
    assert_eq!(anthropic.protocol, AiProviderProtocol::AnthropicMessages);
}

#[test]
fn unicode_model_ids_use_the_store_compatible_character_limit() {
    let valid = ProviderConfig::local_http("local", "鼠".repeat(256), "http://127.0.0.1:11434/v1");
    assert!(LocalHttpProvider::new(valid).is_ok());

    let invalid =
        ProviderConfig::local_http("local", "鼠".repeat(257), "http://127.0.0.1:11434/v1");
    assert!(matches!(
        LocalHttpProvider::new(invalid),
        Err(ProviderConfigError::InvalidModel)
    ));
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
async fn chat_completions_serializes_verified_images_as_data_urls() {
    let body = serde_json::json!({
        "id": "vision-chat-response",
        "model": "vision-chat-model",
        "choices": [{
            "message": {"content": "visible", "tool_calls": []},
            "finish_reason": "stop"
        }]
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider = LocalHttpProvider::new(ProviderConfig::local_http(
        "vision-chat",
        "vision-chat-model",
        base_url,
    ))
    .unwrap();
    provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user_with_images(
                "Inspect",
                vec![
                    VisionImageInput {
                        media_type: "image/jpeg".to_owned(),
                        data_base64: "aGVsbG8=".to_owned(),
                    },
                    VisionImageInput {
                        media_type: "image/png".to_owned(),
                        data_base64: "d29ybGQ=".to_owned(),
                    },
                ],
            )]),
            ProviderCredentials::none(),
        )
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    let payload = captured_payload(&captured);
    let parts = payload["messages"][0]["content"].as_array().unwrap();
    assert_eq!(
        parts[0],
        serde_json::json!({"type": "text", "text": "Inspect"})
    );
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(
        parts[1]["image_url"]["url"],
        "data:image/jpeg;base64,aGVsbG8="
    );
    assert_eq!(parts[1]["image_url"]["detail"], "high");
    assert_eq!(
        parts[2]["image_url"]["url"],
        "data:image/png;base64,d29ybGQ="
    );
}

#[tokio::test]
async fn responses_protocol_serializes_verified_images_as_input_parts() {
    let body = serde_json::json!({
        "id": "vision-responses-response",
        "model": "vision-responses-model",
        "status": "completed",
        "output": [{
            "type": "message",
            "content": [{"type": "output_text", "text": "visible"}]
        }]
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider = LocalHttpProvider::new(ProviderConfig::local_http_with_protocol(
        "vision-responses",
        AiProviderProtocol::OpenaiResponses,
        "vision-responses-model",
        base_url,
    ))
    .unwrap();
    provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user_with_images(
                "Inspect",
                vec![VisionImageInput {
                    media_type: "image/webp".to_owned(),
                    data_base64: "aGVsbG8=".to_owned(),
                }],
            )]),
            ProviderCredentials::none(),
        )
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    let payload = captured_payload(&captured);
    let parts = payload["input"][0]["content"].as_array().unwrap();
    assert_eq!(
        parts[0],
        serde_json::json!({"type": "input_text", "text": "Inspect"})
    );
    assert_eq!(parts[1]["type"], "input_image");
    assert_eq!(parts[1]["image_url"], "data:image/webp;base64,aGVsbG8=");
    assert_eq!(parts[1]["detail"], "high");
}

#[tokio::test]
async fn every_protocol_rejects_invalid_image_payloads_before_network_io() {
    for protocol in [
        AiProviderProtocol::OpenaiChatCompletions,
        AiProviderProtocol::OpenaiResponses,
        AiProviderProtocol::AnthropicMessages,
    ] {
        let provider = LocalHttpProvider::new(ProviderConfig::local_http_with_protocol(
            "invalid-image",
            protocol,
            "vision-model",
            "http://127.0.0.1:9/v1",
        ))
        .unwrap();
        let result = provider
            .complete(
                CompletionRequest::new(vec![ChatMessage::user_with_images(
                    "Inspect",
                    vec![VisionImageInput {
                        media_type: "image/png".to_owned(),
                        data_base64: "not base64!".to_owned(),
                    }],
                )]),
                ProviderCredentials::none(),
            )
            .await;
        assert!(
            matches!(
                result,
                Err(ProviderError::InvalidRequest("invalid vision image"))
            ),
            "{protocol:?} must reject before attempting its configured endpoint"
        );
    }
}

#[tokio::test]
async fn anthropic_rejects_nonportable_image_media_before_network_io() {
    let provider = LocalHttpProvider::new(ProviderConfig::local_http_with_protocol(
        "anthropic-image",
        AiProviderProtocol::AnthropicMessages,
        "claude-model",
        "http://127.0.0.1:9/v1",
    ))
    .unwrap();
    let result = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user_with_images(
                "Inspect",
                vec![VisionImageInput {
                    media_type: "image/bmp".to_owned(),
                    data_base64: "aGVsbG8=".to_owned(),
                }],
            )]),
            ProviderCredentials::none(),
        )
        .await;
    assert!(matches!(
        result,
        Err(ProviderError::InvalidRequest(
            "image media type is unsupported by Anthropic Messages"
        ))
    ));
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
async fn chat_completions_maps_tool_calls_from_response() {
    let body = serde_json::json!({
        "id": "chat-response",
        "model": "chat-model",
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_chat_1",
                    "type": "function",
                    "function": {
                        "name": "animal_search",
                        "arguments": "{\"query\":\"M001\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider =
        LocalHttpProvider::new(ProviderConfig::local_http("local", "chat-model", base_url))
            .unwrap();
    let response = provider
        .complete(
            CompletionRequest::new(vec![ChatMessage::user("Find M001")]),
            ProviderCredentials::none(),
        )
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();

    assert!(captured.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_chat_1".to_owned(),
            name: "animal_search".to_owned(),
            arguments: serde_json::json!({"query": "M001"}),
        }]
    );
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(response.usage.unwrap().total_tokens, 25);
}

#[tokio::test]
async fn responses_protocol_maps_requests_responses_tools_and_usage() {
    let body = serde_json::json!({
        "id": "resp_123",
        "model": "responses-model",
        "status": "completed",
        "output": [
            {
                "type": "message",
                "id": "msg_123",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "I found M001.",
                    "annotations": []
                }]
            },
            {
                "type": "function_call",
                "id": "fc_123",
                "call_id": "call_responses_1",
                "name": "animal_search",
                "arguments": "{\"query\":\"M001\"}",
                "status": "completed"
            }
        ],
        "usage": {"input_tokens": 30, "output_tokens": 7, "total_tokens": 37}
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider = LocalHttpProvider::new(ProviderConfig::local_http_with_protocol(
        "local",
        AiProviderProtocol::OpenaiResponses,
        "responses-model",
        base_url,
    ))
    .unwrap();
    let prior_call = ProviderToolCall {
        id: "call_prior_1".to_owned(),
        name: "animal_search".to_owned(),
        arguments: serde_json::json!({"query": "M000"}),
    };
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system("Use project-scoped tools only."),
        ChatMessage::user("Find M001"),
        ChatMessage::assistant_tool_calls(Some("I will check.".to_owned()), vec![prior_call]),
        ChatMessage::tool("call_prior_1", r#"{"data":[]}"#),
    ]);
    request.tools.push(ToolDefinition {
        name: "animal_search".to_owned(),
        description: "Search project animals".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}}
        }),
    });
    request.temperature = Some(0.3);
    request.max_output_tokens = Some(800);

    let response = provider
        .complete(
            request,
            ProviderCredentials::bearer("responses-key").unwrap(),
        )
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    let payload = captured_payload(&captured);

    assert!(captured.starts_with("POST /v1/responses HTTP/1.1"));
    assert_eq!(
        captured_header(&captured, "authorization"),
        Some("Bearer responses-key")
    );
    assert_eq!(payload["model"], "responses-model");
    assert_eq!(payload["store"], false);
    assert_eq!(payload["max_output_tokens"], 800);
    assert_eq!(payload["input"][0]["role"], "system");
    assert_eq!(payload["input"][2]["role"], "assistant");
    assert_eq!(payload["input"][3]["type"], "function_call");
    assert_eq!(payload["input"][3]["arguments"], r#"{"query":"M000"}"#);
    assert_eq!(payload["input"][4]["type"], "function_call_output");
    assert_eq!(payload["tools"][0]["name"], "animal_search");
    assert_eq!(payload["tools"][0]["strict"], false);

    assert_eq!(response.content.as_deref(), Some("I found M001."));
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_responses_1".to_owned(),
            name: "animal_search".to_owned(),
            arguments: serde_json::json!({"query": "M001"}),
        }]
    );
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(response.usage.unwrap().total_tokens, 37);
}

#[tokio::test]
async fn anthropic_protocol_maps_requests_responses_tools_usage_and_authentication() {
    let body = serde_json::json!({
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "model": "claude-model",
        "content": [
            {"type": "text", "text": "I will inspect M001."},
            {
                "type": "tool_use",
                "id": "toolu_123",
                "name": "animal_search",
                "input": {"query": "M001"}
            }
        ],
        "stop_reason": "tool_use",
        "usage": {
            "input_tokens": 20,
            "cache_creation_input_tokens": 3,
            "cache_read_input_tokens": 2,
            "output_tokens": 6
        }
    })
    .to_string();
    let (base_url, captured, handle) = spawn_http_server("200 OK", body, Duration::ZERO);
    let provider = LocalHttpProvider::new(ProviderConfig::local_http_with_protocol(
        "local",
        AiProviderProtocol::AnthropicMessages,
        "claude-model",
        base_url,
    ))
    .unwrap();
    let prior_call = ProviderToolCall {
        id: "toolu_prior".to_owned(),
        name: "animal_search".to_owned(),
        arguments: serde_json::json!({"query": "M000"}),
    };
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system("Stay within the active project."),
        ChatMessage::user_with_images(
            "Inspect the image",
            vec![VisionImageInput {
                media_type: "image/png".to_owned(),
                data_base64: "aGVsbG8=".to_owned(),
            }],
        ),
        ChatMessage::assistant_tool_calls(None, vec![prior_call]),
        ChatMessage::tool("toolu_prior", r#"{"data":[]}"#),
    ]);
    request.tools.push(ToolDefinition {
        name: "animal_search".to_owned(),
        description: "Search project animals".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}}
        }),
    });
    request.max_output_tokens = Some(900);

    let response = provider
        .complete(
            request,
            ProviderCredentials::bearer("anthropic-key").unwrap(),
        )
        .await
        .unwrap();
    let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    handle.join().unwrap();
    let payload = captured_payload(&captured);

    assert!(captured.starts_with("POST /v1/messages HTTP/1.1"));
    assert_eq!(
        captured_header(&captured, "x-api-key"),
        Some("anthropic-key")
    );
    assert_eq!(
        captured_header(&captured, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(captured_header(&captured, "authorization"), None);
    assert_eq!(payload["system"], "Stay within the active project.");
    assert_eq!(
        payload["messages"][0]["content"][0],
        serde_json::json!({"type": "text", "text": "Inspect the image"})
    );
    assert_eq!(payload["messages"][0]["content"][1]["type"], "image");
    assert_eq!(
        payload["messages"][0]["content"][1]["source"],
        serde_json::json!({
            "type": "base64",
            "media_type": "image/png",
            "data": "aGVsbG8="
        })
    );
    assert_eq!(payload["messages"][1]["content"][0]["type"], "tool_use");
    assert_eq!(payload["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(payload["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(payload["max_tokens"], 900);

    assert_eq!(response.id.as_deref(), Some("msg_123"));
    assert_eq!(response.model.as_deref(), Some("claude-model"));
    assert_eq!(response.content.as_deref(), Some("I will inspect M001."));
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "toolu_123".to_owned(),
            name: "animal_search".to_owned(),
            arguments: serde_json::json!({"query": "M001"}),
        }]
    );
    assert_eq!(response.finish_reason.as_deref(), Some("tool_use"));
    let usage = response.usage.unwrap();
    assert_eq!(usage.input_tokens, 25);
    assert_eq!(usage.output_tokens, 6);
    assert_eq!(usage.total_tokens, 31);
}

#[tokio::test]
async fn protocol_endpoints_are_normalized_without_duplicate_suffixes() {
    for (protocol, configured_suffix, expected_path) in [
        (
            AiProviderProtocol::OpenaiChatCompletions,
            "/responses/",
            "/v1/chat/completions",
        ),
        (
            AiProviderProtocol::OpenaiResponses,
            "/chat/completions/",
            "/v1/responses",
        ),
        (
            AiProviderProtocol::AnthropicMessages,
            "/messages/",
            "/v1/messages",
        ),
    ] {
        let (base_url, captured, handle) =
            spawn_http_server("400 Bad Request", "not-returned".to_owned(), Duration::ZERO);
        let configured_url = format!("{base_url}{configured_suffix}");
        let provider = LocalHttpProvider::new(
            ProviderConfig::local_http("local", "model", configured_url).with_protocol(protocol),
        )
        .unwrap();
        let error = provider
            .complete(
                CompletionRequest::new(vec![ChatMessage::user("ping")]),
                ProviderCredentials::none(),
            )
            .await
            .unwrap_err();
        let captured = captured.recv_timeout(Duration::from_secs(2)).unwrap();
        handle.join().unwrap();

        assert!(captured.starts_with(&format!("POST {expected_path} HTTP/1.1")));
        assert!(matches!(
            error,
            ProviderError::HttpStatus { status: 400, .. }
        ));
    }
}

#[tokio::test]
async fn all_protocol_http_errors_are_sanitized_and_keep_request_ids() {
    for protocol in [
        AiProviderProtocol::OpenaiChatCompletions,
        AiProviderProtocol::OpenaiResponses,
        AiProviderProtocol::AnthropicMessages,
    ] {
        let secret_body = format!("provider-secret-{protocol:?}");
        let (base_url, captured, handle) =
            spawn_http_server("429 Too Many Requests", secret_body.clone(), Duration::ZERO);
        let provider = LocalHttpProvider::new(
            ProviderConfig::local_http("local", "model", base_url).with_protocol(protocol),
        )
        .unwrap();
        let error = provider
            .complete(
                CompletionRequest::new(vec![ChatMessage::user("ping")]),
                ProviderCredentials::bearer("never-display-key").unwrap(),
            )
            .await
            .unwrap_err();
        let _ = captured.recv_timeout(Duration::from_secs(2)).unwrap();
        handle.join().unwrap();

        let displayed = error.to_string();
        assert!(!displayed.contains(&secret_body));
        assert!(!displayed.contains("never-display-key"));
        assert_eq!(
            error,
            ProviderError::HttpStatus {
                status: 429,
                request_id: Some("safe-request-id".to_owned())
            }
        );
    }
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
