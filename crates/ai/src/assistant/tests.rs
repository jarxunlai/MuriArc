use std::sync::{Arc, Mutex};

use chrono::{Duration, TimeZone, Utc};
use serde_json::json;

use super::*;
use crate::{DraftKind, FieldChange, MockProvider, ScopeSet, ToolScope};

type Handler =
    dyn Fn(&DomainToolRequest) -> Result<DomainToolOutput, ToolExecutionError> + Send + Sync;

#[derive(Clone)]
struct TestExecutor {
    requests: Arc<Mutex<Vec<DomainToolRequest>>>,
    handler: Arc<Handler>,
}

impl TestExecutor {
    fn new(
        handler: impl Fn(&DomainToolRequest) -> Result<DomainToolOutput, ToolExecutionError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(handler),
        }
    }

    fn requests(&self) -> Vec<DomainToolRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl DomainToolExecutor for TestExecutor {
    async fn execute(
        &self,
        request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        self.requests.lock().unwrap().push(request.clone());
        (self.handler)(&request)
    }
}

fn completion(content: Option<&str>, tool_calls: Vec<ProviderToolCall>) -> CompletionResponse {
    CompletionResponse {
        id: Some(Uuid::new_v4().to_string()),
        model: Some("mock-model".to_owned()),
        content: content.map(str::to_owned),
        tool_calls,
        finish_reason: None,
        usage: Some(TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
        }),
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ProviderToolCall {
    ProviderToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    }
}

fn read_access() -> AccessGrant {
    AccessGrant::local_user(ScopeSet::new([ToolScope::Read]))
}

fn empty_read_executor() -> TestExecutor {
    TestExecutor::new(|_| Ok(DomainToolOutput::read(json!({"items": []}), vec![])))
}

#[tokio::test]
async fn bounded_user_assistant_history_is_sent_before_the_new_turn() {
    let provider = MockProvider::new(
        "mock",
        "model",
        [Ok(completion(Some("third answer"), vec![]))],
    );
    let probe = provider.clone();
    let service = AssistantService::new(provider, empty_read_executor());
    let request = AssistantRequest::new(Uuid::new_v4(), "third question").with_history(vec![
        ChatMessage::user("first question"),
        ChatMessage::assistant("first answer"),
        ChatMessage::user("second question"),
        ChatMessage::assistant("second answer"),
    ]);

    let response = service
        .run(request, &read_access(), ProviderCredentials::none())
        .await
        .unwrap();

    assert_eq!(response.content, "third answer");
    let requests = probe.requests().unwrap();
    let messages = &requests[0].messages;
    assert_eq!(messages.len(), 6);
    assert_eq!(messages[0].role, ChatRole::System);
    assert_eq!(messages[1].content, "first question");
    assert_eq!(messages[2].content, "first answer");
    assert_eq!(messages[3].content, "second question");
    assert_eq!(messages[4].content, "second answer");
    assert_eq!(messages[5].content, "third question");
}

#[tokio::test]
async fn malformed_or_oversized_history_is_rejected_before_provider_access() {
    let provider = MockProvider::new("mock", "model", []);
    let probe = provider.clone();
    let service = AssistantService::new(provider, empty_read_executor());
    let malformed = AssistantRequest::new(Uuid::new_v4(), "next").with_history(vec![
        ChatMessage::assistant("not a user message"),
        ChatMessage::assistant("answer"),
    ]);

    let error = service
        .run(malformed, &read_access(), ProviderCredentials::none())
        .await
        .unwrap_err();

    assert!(matches!(error, AssistantError::InvalidConversationHistory));
    assert!(probe.requests().unwrap().is_empty());
}

struct AnimalOnlyExecutor;

#[async_trait]
impl DomainToolExecutor for AnimalOnlyExecutor {
    fn supported_tools(&self) -> Vec<ToolName> {
        vec![ToolName::AnimalSearch]
    }

    async fn execute(
        &self,
        _request: DomainToolRequest,
    ) -> Result<DomainToolOutput, ToolExecutionError> {
        Ok(DomainToolOutput::read(json!({"items": []}), vec![]))
    }
}

#[test]
fn executor_declaration_narrows_tools_visible_to_the_model() {
    let provider = MockProvider::new("mock", "model", []);
    let service = AssistantService::new(provider, AnimalOnlyExecutor);
    let access = AccessGrant::local_user(ScopeSet::new([
        ToolScope::Read,
        ToolScope::Import,
        ToolScope::Export,
        ToolScope::TemplateDraft,
        ToolScope::WriteDraft,
    ]));

    let visible = service
        .visible_tools(&access)
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();
    assert_eq!(visible, vec![ToolName::AnimalSearch.as_str()]);
}

#[tokio::test]
async fn external_scope_intersection_is_enforced_even_for_a_model_call() {
    let access = AccessGrant::external(
        ScopeSet::new([ToolScope::Read, ToolScope::WriteDraft]),
        ScopeSet::new([ToolScope::Read]),
    );
    let provider = MockProvider::new(
        "mock",
        "model",
        [Ok(completion(
            None,
            vec![call("call-1", ToolName::MutationDraft.as_str(), json!({}))],
        ))],
    );
    let provider_probe = provider.clone();
    let executor = empty_read_executor();
    let executor_probe = executor.clone();
    let service = AssistantService::new(provider, executor);

    let error = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), "change the animal"),
            &access,
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AssistantError::UnauthorizedTool {
            tool: ToolName::MutationDraft,
            ..
        }
    ));
    assert!(executor_probe.requests().is_empty());
    let provider_requests = provider_probe.requests().unwrap();
    let visible_names = provider_requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(visible_names.contains(&ToolName::AnimalSearch.as_str()));
    assert!(!visible_names.contains(&ToolName::MutationDraft.as_str()));
}

#[tokio::test]
async fn unknown_tool_never_reaches_the_domain_executor() {
    let provider = MockProvider::new(
        "mock",
        "model",
        [Ok(completion(
            None,
            vec![call("call-1", "admin_delete", json!({}))],
        ))],
    );
    let executor = empty_read_executor();
    let probe = executor.clone();
    let service = AssistantService::new(provider, executor);

    let error = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), "do something"),
            &read_access(),
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AssistantError::UnknownTool { ref name } if name == "admin_delete"
    ));
    assert!(probe.requests().is_empty());
}

#[tokio::test]
async fn raw_sql_arguments_are_rejected_before_execution() {
    let provider = MockProvider::new(
        "mock",
        "model",
        [Ok(completion(
            None,
            vec![call(
                "call-1",
                ToolName::AnimalSearch.as_str(),
                json!({"query": {"raw_sql": "drop table animals"}}),
            )],
        ))],
    );
    let executor = empty_read_executor();
    let probe = executor.clone();
    let service = AssistantService::new(provider, executor);

    let error = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), "find animals"),
            &read_access(),
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, AssistantError::RawSqlForbidden));
    assert!(probe.requests().is_empty());
}

#[tokio::test]
async fn repeated_tool_requests_stop_at_the_iteration_limit() {
    let provider = MockProvider::new(
        "mock",
        "model",
        [
            Ok(completion(
                None,
                vec![call(
                    "call-1",
                    ToolName::AnimalSearch.as_str(),
                    json!({"query": "M"}),
                )],
            )),
            Ok(completion(
                None,
                vec![call(
                    "call-2",
                    ToolName::AnimalSearch.as_str(),
                    json!({"query": "M0"}),
                )],
            )),
        ],
    );
    let provider_probe = provider.clone();
    let executor = empty_read_executor();
    let executor_probe = executor.clone();
    let limits = AssistantLimits {
        max_iterations: 2,
        ..AssistantLimits::default()
    };
    let service = AssistantService::with_limits(provider, executor, limits).unwrap();

    let error = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), "keep searching"),
            &read_access(),
            ProviderCredentials::none(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, AssistantError::IterationLimitExceeded));
    assert_eq!(executor_probe.requests().len(), 1);
    assert_eq!(provider_probe.requests().unwrap().len(), 2);
}

#[tokio::test]
async fn read_results_return_structured_citations_and_tool_trace() {
    let animal_id = Uuid::new_v4();
    let citation = Citation::new(EntityType::Animal, animal_id, Some(3));
    let provider = MockProvider::new(
        "mock-provider",
        "mock-model",
        [
            Ok(completion(
                None,
                vec![call(
                    "call-1",
                    ToolName::AnimalSearch.as_str(),
                    json!({"query": "M001"}),
                )],
            )),
            Ok(completion(Some("M001 is active."), vec![])),
        ],
    );
    let provider_probe = provider.clone();
    let executor = TestExecutor::new({
        let citation = citation.clone();
        move |_| {
            Ok(DomainToolOutput::read(
                json!({"items": [{"id": animal_id, "display_id": "M001"}]}),
                vec![citation.clone()],
            ))
        }
    });
    let service = AssistantService::new(provider, executor);

    let response = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), "What is M001's status?"),
            &read_access(),
            ProviderCredentials::none(),
        )
        .await
        .unwrap();

    assert_eq!(response.content, "M001 is active.");
    assert_eq!(response.citations, vec![citation.clone()]);
    assert_eq!(response.tool_runs.len(), 1);
    assert_eq!(response.tool_runs[0].outcome, ToolRunOutcome::Read);
    assert_eq!(response.tool_runs[0].citations, vec![citation]);
    assert_eq!(response.usage.provider_calls, 2);
    assert_eq!(response.usage.tool_calls, 1);
    let requests = provider_probe.requests().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(
        |message| message.role == crate::ChatRole::Assistant && !message.tool_calls.is_empty()
    ));
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.role == crate::ChatRole::Tool)
    );
}

#[tokio::test]
async fn write_tool_can_only_return_a_pending_diff_draft() {
    let user_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let animal_id = Uuid::new_v4();
    let provider = MockProvider::new(
        "mock",
        "model",
        [
            Ok(completion(
                None,
                vec![call(
                    "call-1",
                    ToolName::MutationDraft.as_str(),
                    json!({
                        "entity_type": "animal",
                        "entity_id": animal_id,
                        "expected_revision": 1,
                        "changes": [{"path": "/strain", "before": null, "after": "C57BL/6J"}]
                    }),
                )],
            )),
            Ok(completion(Some("The change is ready for review."), vec![])),
        ],
    );
    let executor = TestExecutor::new(move |request| {
        let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
        let draft = WriteDraft::new(
            DraftKind::OrdinaryWrite,
            request.tool,
            ProposalActor::Ai {
                user_id: request.user_id,
                tool_run_id: request.tool_run_id,
            },
            Some(project_id),
            vec![FieldChange {
                path: "/strain".to_owned(),
                before: Some(Value::Null),
                after: Some(json!("C57BL/6J")),
            }],
            json!({"animal_id": animal_id, "strain": "C57BL/6J"}),
            now,
            now + Duration::hours(1),
        )
        .unwrap();
        Ok(DomainToolOutput::write_draft(draft, vec![]))
    });
    let service = AssistantService::new(provider, executor);
    let access = AccessGrant::local_user(ScopeSet::new([ToolScope::WriteDraft]));

    let response = service
        .run(
            AssistantRequest::new(user_id, "Set the strain after I review it"),
            &access,
            ProviderCredentials::none(),
        )
        .await
        .unwrap();

    assert_eq!(response.drafts.len(), 1);
    assert_eq!(response.drafts[0].status(), DraftStatus::PendingApproval);
    assert_eq!(response.drafts[0].revision(), 1);
    assert_eq!(response.tool_runs[0].outcome, ToolRunOutcome::WriteDraft);
    assert_eq!(
        response.tool_runs[0].draft_id,
        Some(response.drafts[0].id())
    );
}
