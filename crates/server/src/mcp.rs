use std::env;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header::ORIGIN},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use muriarc_core::{
    AnimalFilter, AnimalStatus, ExperimentFilter, ExperimentStatus, MeasurementFilter, Permission,
    SampleFilter, StoreError,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata, auth::authenticate_request};

const JSON_RPC_VERSION: &str = "2.0";
const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[LATEST_PROTOCOL_VERSION, "2025-03-26"];
const MAX_MCP_BODY_BYTES: usize = 128 * 1024;
const MAX_TOOL_ITEMS: usize = 100;
const DEFAULT_TOOL_ITEMS: usize = 50;
const MAX_CLIENT_TEXT_BYTES: usize = 256;

pub(crate) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/mcp", post(handle))
        .layer(DefaultBodyLimit::max(MAX_MCP_BODY_BYTES))
        .route_layer(middleware::from_fn_with_state(state, authenticate_request))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcErrorBody {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl RpcErrorBody {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    fn internal() -> Self {
        Self::new(-32603, "internal error")
    }
}

async fn handle(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    headers: HeaderMap,
    payload: Result<Json<RpcRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    enforce_origin(&headers, &metadata)?;
    if !principal.is_external_ai() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "external_ai_token_required",
            "MCP requires an external token narrowed by AI scopes",
        )
        .with_request_id(metadata.request_id));
    }

    let request = match payload {
        Ok(Json(request)) => request,
        Err(error) => {
            return Ok(rpc_failure(
                Value::Null,
                RpcErrorBody::new(-32700, format!("parse error: {error}")),
            ));
        }
    };

    let id = request.id.clone().unwrap_or(Value::Null);
    if request.jsonrpc != JSON_RPC_VERSION
        || request.method.is_empty()
        || !valid_request_id(request.id.as_ref())
    {
        return Ok(rpc_failure(
            id,
            RpcErrorBody::new(-32600, "invalid JSON-RPC request"),
        ));
    }

    let result = dispatch(&state, &principal, &request).await;
    if request.id.is_none() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    Ok(match result {
        Ok(value) => rpc_success(id, value),
        Err(error) => rpc_failure(id, error),
    })
}

async fn dispatch(
    state: &AppState,
    principal: &AuthPrincipal,
    request: &RpcRequest,
) -> Result<Value, RpcErrorBody> {
    match request.method.as_str() {
        "initialize" => initialize(request.params.clone()),
        "notifications/initialized" => {
            parse_params::<EmptyParams>(request.params.clone())?;
            Ok(json!({}))
        }
        "ping" => {
            parse_params::<EmptyParams>(request.params.clone())?;
            Ok(json!({}))
        }
        "tools/list" => list_tools(request.params.clone()),
        "tools/call" => call_tool(state, principal, request.params.clone()).await,
        _ => Err(RpcErrorBody::new(-32601, "method not found")),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializeParams {
    protocol_version: String,
    capabilities: Value,
    client_info: ClientInfo,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientInfo {
    name: String,
    version: String,
    #[serde(default)]
    title: Option<String>,
}

fn initialize(params: Option<Value>) -> Result<Value, RpcErrorBody> {
    let params = parse_params::<InitializeParams>(params)?;
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&params.protocol_version.as_str()) {
        return Err(RpcErrorBody::invalid_params(format!(
            "unsupported protocol version; supported versions: {}",
            SUPPORTED_PROTOCOL_VERSIONS.join(", ")
        )));
    }
    validate_client_text("clientInfo.name", &params.client_info.name)?;
    validate_client_text("clientInfo.version", &params.client_info.version)?;
    if let Some(title) = &params.client_info.title {
        validate_client_text("clientInfo.title", title)?;
    }
    if !params.capabilities.is_object() {
        return Err(RpcErrorBody::invalid_params(
            "capabilities must be a JSON object",
        ));
    }

    Ok(json!({
        "protocolVersion": params.protocol_version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {
            "name": "MuriArc",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Read-only animal research tools. Raw SQL and direct writes are unavailable."
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListToolsParams {
    #[serde(default)]
    cursor: Option<String>,
}

fn list_tools(params: Option<Value>) -> Result<Value, RpcErrorBody> {
    let params = parse_params::<ListToolsParams>(params)?;
    if let Some(cursor) = params.cursor {
        if cursor.is_empty() || cursor.len() > MAX_CLIENT_TEXT_BYTES {
            return Err(RpcErrorBody::invalid_params("cursor is malformed"));
        }
        return Err(RpcErrorBody::invalid_params(
            "tool pagination cursors are not used by this server",
        ));
    }
    Ok(json!({"tools": tool_definitions()}))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallToolParams {
    name: String,
    #[serde(default = "empty_arguments")]
    arguments: Value,
}

fn empty_arguments() -> Value {
    json!({})
}

async fn call_tool(
    state: &AppState,
    principal: &AuthPrincipal,
    params: Option<Value>,
) -> Result<Value, RpcErrorBody> {
    let params = parse_params::<CallToolParams>(params)?;
    if !params.arguments.is_object() {
        return Err(RpcErrorBody::invalid_params(
            "tool arguments must be a JSON object",
        ));
    }

    let value = match params.name.as_str() {
        "animal.search" => {
            let args = parse_value::<AnimalSearchArgs>(params.arguments)?;
            require_optional_project_permission(
                state,
                principal,
                Permission::ReadAnimal,
                args.project_id,
            )
            .await?;
            let limit = validated_limit(args.limit)?;
            let query = args
                .query
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            if query
                .as_ref()
                .is_some_and(|value| value.len() > MAX_CLIENT_TEXT_BYTES)
            {
                return Err(RpcErrorBody::invalid_params(
                    "query must not exceed 256 bytes",
                ));
            }
            let animals = state
                .store
                .list_animals(&AnimalFilter {
                    lab_id: principal.lab_id,
                    project_id: args.project_id,
                    cage_id: args.cage_id,
                    status: args.status,
                    query,
                })
                .await
                .map_err(map_store_error)?;
            limited_collection(animals, limit)?
        }
        "animal.timeline" => {
            let args = parse_value::<AnimalTimelineArgs>(params.arguments)?;
            let limit = validated_limit(args.limit)?;
            require_optional_project_permission(
                state,
                principal,
                Permission::ReadAnimal,
                args.project_id,
            )
            .await?;
            let animal = state
                .store
                .get_animal(args.animal_id)
                .await
                .map_err(map_store_error)?;
            if animal.lab_id != principal.lab_id {
                return Err(RpcErrorBody::new(-32004, "resource was not found"));
            }
            if let Some(project_id) = args.project_id {
                let visible = state
                    .store
                    .list_animals(&AnimalFilter {
                        lab_id: principal.lab_id,
                        project_id: Some(project_id),
                        cage_id: None,
                        status: None,
                        query: None,
                    })
                    .await
                    .map_err(map_store_error)?
                    .into_iter()
                    .any(|candidate| candidate.id == animal.id);
                if !visible {
                    return Err(RpcErrorBody::new(-32004, "resource was not found"));
                }
            }
            let mut events = state
                .store
                .list_animal_events(args.animal_id)
                .await
                .map_err(map_store_error)?;
            if let Some(project_id) = args.project_id {
                let can_read_unscoped = principal.is_lab_operator();
                events.retain(|event| {
                    event.project_id == Some(project_id)
                        || (can_read_unscoped && event.project_id.is_none())
                });
            }
            latest_collection(events, limit)?
        }
        "experiment.status" => {
            let args = parse_value::<ExperimentStatusArgs>(params.arguments)?;
            require_project_permission(
                state,
                principal,
                Permission::ReadExperiment,
                args.project_id,
            )
            .await?;
            let limit = validated_limit(args.limit)?;
            let experiments = state
                .store
                .list_experiments(&ExperimentFilter {
                    project_id: args.project_id,
                    status: args.status,
                })
                .await
                .map_err(map_store_error)?;
            limited_collection(experiments, limit)?
        }
        "measurement.query" => {
            let args = parse_value::<MeasurementQueryArgs>(params.arguments)?;
            require_project_permission(
                state,
                principal,
                Permission::ReadMeasurement,
                args.project_id,
            )
            .await?;
            let limit = validated_limit(args.limit)?;
            let measurements = state
                .store
                .list_measurements(&MeasurementFilter {
                    project_id: args.project_id,
                    experiment_id: args.experiment_id,
                    animal_id: args.animal_id,
                })
                .await
                .map_err(map_store_error)?;
            limited_collection(measurements, limit)?
        }
        "sample.inventory" => {
            let args = parse_value::<SampleInventoryArgs>(params.arguments)?;
            require_project_permission(state, principal, Permission::ReadSample, args.project_id)
                .await?;
            let limit = validated_limit(args.limit)?;
            let samples = state
                .store
                .list_samples(&SampleFilter {
                    project_id: args.project_id,
                    experiment_id: args.experiment_id,
                    animal_id: args.animal_id,
                })
                .await
                .map_err(map_store_error)?;
            limited_collection(samples, limit)?
        }
        _ => return Err(RpcErrorBody::invalid_params("unknown tool name")),
    };

    tool_result(value)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimalSearchArgs {
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default)]
    cage_id: Option<Uuid>,
    #[serde(default)]
    status: Option<AnimalStatus>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimalTimelineArgs {
    animal_id: Uuid,
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentStatusArgs {
    project_id: Uuid,
    #[serde(default)]
    status: Option<ExperimentStatus>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementQueryArgs {
    project_id: Uuid,
    #[serde(default)]
    experiment_id: Option<Uuid>,
    #[serde(default)]
    animal_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleInventoryArgs {
    project_id: Uuid,
    #[serde(default)]
    experiment_id: Option<Uuid>,
    #[serde(default)]
    animal_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_TOOL_ITEMS
}

fn validated_limit(limit: usize) -> Result<usize, RpcErrorBody> {
    if (1..=MAX_TOOL_ITEMS).contains(&limit) {
        Ok(limit)
    } else {
        Err(RpcErrorBody::invalid_params(format!(
            "limit must be between 1 and {MAX_TOOL_ITEMS}"
        )))
    }
}

fn require_permission(
    principal: &AuthPrincipal,
    permission: Permission,
    project_id: Option<Uuid>,
) -> Result<(), RpcErrorBody> {
    if principal.can(permission, project_id) {
        Ok(())
    } else {
        Err(RpcErrorBody::new(-32003, "tool permission denied"))
    }
}

async fn require_optional_project_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    permission: Permission,
    project_id: Option<Uuid>,
) -> Result<(), RpcErrorBody> {
    match project_id {
        Some(project_id) => {
            require_project_permission(state, principal, permission, project_id).await
        }
        None => require_permission(principal, permission, None),
    }
}

async fn require_project_permission(
    state: &AppState,
    principal: &AuthPrincipal,
    permission: Permission,
    project_id: Uuid,
) -> Result<(), RpcErrorBody> {
    require_permission(principal, permission, Some(project_id))?;
    let project = state
        .store
        .get_project(project_id)
        .await
        .map_err(map_store_error)?;
    if project.lab_id != principal.lab_id {
        return Err(RpcErrorBody::new(-32004, "resource was not found"));
    }
    Ok(())
}

fn limited_collection<T: Serialize>(
    mut values: Vec<T>,
    limit: usize,
) -> Result<Value, RpcErrorBody> {
    let truncated = values.len() > limit;
    values.truncate(limit);
    let count = values.len();
    serde_json::to_value(json!({
        "items": values,
        "count": count,
        "truncated": truncated
    }))
    .map_err(|_| RpcErrorBody::internal())
}

fn latest_collection<T: Serialize>(
    mut values: Vec<T>,
    limit: usize,
) -> Result<Value, RpcErrorBody> {
    let truncated = values.len() > limit;
    if truncated {
        values.drain(0..values.len() - limit);
    }
    let count = values.len();
    serde_json::to_value(json!({
        "items": values,
        "count": count,
        "truncated": truncated
    }))
    .map_err(|_| RpcErrorBody::internal())
}

fn tool_result(structured_content: Value) -> Result<Value, RpcErrorBody> {
    let text = serde_json::to_string(&structured_content).map_err(|_| RpcErrorBody::internal())?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured_content,
        "isError": false
    }))
}

fn parse_params<T: DeserializeOwned>(params: Option<Value>) -> Result<T, RpcErrorBody> {
    parse_value(params.unwrap_or_else(|| json!({})))
}

fn parse_value<T: DeserializeOwned>(value: Value) -> Result<T, RpcErrorBody> {
    serde_json::from_value(value)
        .map_err(|error| RpcErrorBody::invalid_params(format!("invalid parameters: {error}")))
}

fn validate_client_text(field: &str, value: &str) -> Result<(), RpcErrorBody> {
    if value.is_empty()
        || value.len() > MAX_CLIENT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        Err(RpcErrorBody::invalid_params(format!(
            "{field} is malformed"
        )))
    } else {
        Ok(())
    }
}

fn map_store_error(error: StoreError) -> RpcErrorBody {
    match error {
        StoreError::NotFound { .. } => RpcErrorBody::new(-32004, "resource was not found"),
        StoreError::Conflict(message) => RpcErrorBody::new(-32009, message),
        StoreError::Validation(message) => RpcErrorBody::invalid_params(message),
        StoreError::Database(message) | StoreError::Serialization(message) => {
            tracing::error!(error = %message, "MCP domain tool failed");
            RpcErrorBody::internal()
        }
    }
}

fn valid_request_id(id: Option<&Value>) -> bool {
    id.is_none_or(|id| id.is_null() || id.is_string() || id.is_number())
}

fn enforce_origin(headers: &HeaderMap, metadata: &RequestMetadata) -> Result<(), ApiError> {
    let origins = headers.get_all(ORIGIN);
    let mut values = origins.iter();
    let Some(origin) = values.next() else {
        return Ok(());
    };
    if values.next().is_some() {
        return Err(origin_error(metadata));
    }
    let origin = origin.to_str().map_err(|_| origin_error(metadata))?;
    let allowed = env::var("MURIARC_MCP_ALLOWED_ORIGINS").ok();
    if origin_matches(origin, allowed.as_deref()) {
        Ok(())
    } else {
        Err(origin_error(metadata))
    }
}

fn origin_matches(origin: &str, configured: Option<&str>) -> bool {
    configured.is_some_and(|configured| {
        configured
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .any(|allowed| allowed == origin)
    })
}

fn origin_error(metadata: &RequestMetadata) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "mcp_origin_forbidden",
        "browser origin is not allowed for MCP",
    )
    .with_request_id(metadata.request_id.clone())
}

fn rpc_success(id: Value, result: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "result": result
        })),
    )
        .into_response()
}

fn rpc_failure(id: Value, error: RpcErrorBody) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "error": error
        })),
    )
        .into_response()
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool_definition(
            "animal.search",
            "Search animals visible to the token owner.",
            json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string", "format": "uuid"},
                    "cage_id": {"type": "string", "format": "uuid"},
                    "status": {"type": "string"},
                    "query": {"type": "string", "maxLength": MAX_CLIENT_TEXT_BYTES},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TOOL_ITEMS, "default": DEFAULT_TOOL_ITEMS}
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "animal.timeline",
            "Read the recent event timeline for one visible animal.",
            json!({
                "type": "object",
                "properties": {
                    "animal_id": {"type": "string", "format": "uuid"},
                    "project_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TOOL_ITEMS, "default": DEFAULT_TOOL_ITEMS}
                },
                "required": ["animal_id"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "experiment.status",
            "List experiments and their current status for one project.",
            json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string", "format": "uuid"},
                    "status": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TOOL_ITEMS, "default": DEFAULT_TOOL_ITEMS}
                },
                "required": ["project_id"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "measurement.query",
            "Query structured measurements for one project.",
            json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string", "format": "uuid"},
                    "experiment_id": {"type": "string", "format": "uuid"},
                    "animal_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TOOL_ITEMS, "default": DEFAULT_TOOL_ITEMS}
                },
                "required": ["project_id"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "sample.inventory",
            "Query the minimal traceable sample inventory for one project.",
            json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string", "format": "uuid"},
                    "experiment_id": {"type": "string", "format": "uuid"},
                    "animal_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TOOL_ITEMS, "default": DEFAULT_TOOL_ITEMS}
                },
                "required": ["project_id"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_origins_are_exact_and_default_deny() {
        assert!(!origin_matches("https://lab.example", None));
        assert!(origin_matches(
            "https://lab.example",
            Some("https://other.example, https://lab.example")
        ));
        assert!(!origin_matches(
            "https://evil-lab.example",
            Some("https://lab.example")
        ));
        assert!(!origin_matches("https://lab.example", Some("*")));
    }

    #[test]
    fn tool_arguments_reject_raw_sql_and_unknown_fields() {
        let error = parse_value::<AnimalSearchArgs>(json!({
            "query": "mouse",
            "raw_sql": "DROP TABLE animals"
        }))
        .expect_err("unknown parameters must be rejected");
        assert_eq!(error.code, -32602);
    }

    #[test]
    fn exposed_tools_are_read_only_and_fixed() {
        let names = tool_definitions()
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "animal.search",
                "animal.timeline",
                "experiment.status",
                "measurement.query",
                "sample.inventory"
            ]
        );
        assert!(names.iter().all(|name| !name.contains("sql")));
    }

    #[test]
    fn latest_collection_returns_most_recent_items_in_source_order() {
        let value = latest_collection(vec![1, 2, 3, 4], 2).unwrap();
        assert_eq!(value["items"], json!([3, 4]));
        assert_eq!(value["truncated"], json!(true));
    }

    #[test]
    fn advertised_protocol_version_is_supported() {
        assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&LATEST_PROTOCOL_VERSION));
    }
}
