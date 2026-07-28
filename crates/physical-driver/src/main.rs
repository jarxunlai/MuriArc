mod actions;
mod backup;
mod context;
mod model;
mod verify;

use std::{env, io::Read, process::ExitCode};

use anyhow::{Context as _, Result};
use model::{DRIVER_PROTOCOL_FORMAT, DriverRequest, DriverResponse, OperationPayload};
use serde::de::DeserializeOwned;
use serde_json::Value;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!(
                "ERROR [physical_driver_failed] physical operation failed closed; secrets and command diagnostics were suppressed"
            );
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(args.len() == 2, "driver command requires mode and action");
    let mode = args[0].as_str();
    let expected_action = args[1].as_str();
    anyhow::ensure!(
        matches!(mode, "invoke" | "hold-lock"),
        "unknown driver mode"
    );
    let request = read_request()?;
    anyhow::ensure!(
        request.format_version == DRIVER_PROTOCOL_FORMAT
            && request.action == expected_action
            && !request.action.trim().is_empty(),
        "driver request identity is invalid"
    );
    let context = context::DriverContext::load(request.profile)?;
    if mode == "hold-lock" {
        anyhow::ensure!(
            request.action == "acquire_backend_lock",
            "hold-lock supports only the backend lock"
        );
        let payload: OperationPayload = parse_payload(request.payload)?;
        anyhow::ensure!(!payload.operation_id.is_nil(), "operation id is nil");
        let _lock = context.acquire_backend_lock().await?;
        emit(
            &request.action,
            serde_json::json!({
                "operation_id": payload.operation_id,
                "lock_held": true,
            }),
        )?;
        std::future::pending::<()>().await;
        unreachable!();
    }
    let data = actions::dispatch(&context, &request.action, request.payload).await?;
    emit(&request.action, data)
}

fn read_request() -> Result<DriverRequest> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut input)
        .context("driver request could not be read")?;
    anyhow::ensure!(
        !input.is_empty() && input.len() <= 8 * 1024 * 1024,
        "driver request length is invalid"
    );
    serde_json::from_slice(&input).context("driver request schema is invalid")
}

pub(crate) fn parse_payload<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).context("driver payload schema is invalid")
}

fn emit(action: &str, data: Value) -> Result<()> {
    let response = DriverResponse {
        format_version: DRIVER_PROTOCOL_FORMAT,
        action: action.to_owned(),
        status: "pass",
        data,
    };
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_payload_reject_unknown_fields() {
        let request = serde_json::from_value::<DriverRequest>(serde_json::json!({
            "format_version": 1,
            "action": "current_generation",
            "profile": "native-system",
            "payload": {},
            "extra": true,
        }));
        assert!(request.is_err());
        let payload = parse_payload::<OperationPayload>(serde_json::json!({
            "operation_id": uuid::Uuid::new_v4(),
            "extra": true,
        }));
        assert!(payload.is_err());
    }
}
