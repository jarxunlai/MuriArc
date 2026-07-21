use axum::{
    Json,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use muriarc_application::ApplicationError;
use muriarc_core::{DomainError, StoreError};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorPayload {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    payload: ErrorPayload,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            payload: ErrorPayload {
                code,
                message: message.into(),
                request_id: None,
                details: None,
            },
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "you do not have permission to perform this operation",
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message,
        )
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "an internal server error occurred",
        )
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.payload.request_id = Some(request_id.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.payload.details = Some(details);
        self
    }

    pub fn from_store(error: StoreError) -> Self {
        match error {
            StoreError::NotFound { entity, id } => {
                Self::not_found(format!("{entity} {id} was not found"))
            }
            StoreError::Conflict(message) => Self::conflict(message),
            StoreError::Validation(message) => Self::validation(message),
            StoreError::Database(message) => {
                tracing::error!(error = %message, "database operation failed");
                Self::internal()
            }
            StoreError::Serialization(message) => {
                tracing::error!(error = %message, "store serialization failed");
                Self::internal()
            }
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl From<DomainError> for ApiError {
    fn from(error: DomainError) -> Self {
        Self::validation(error.to_string())
    }
}

impl From<ApplicationError> for ApiError {
    fn from(error: ApplicationError) -> Self {
        match error {
            ApplicationError::Domain(error) => Self::from(error),
            ApplicationError::Store(error) => Self::from_store(error),
            ApplicationError::TooLong { field, max } => {
                Self::validation(format!("{field} must not exceed {max} characters"))
            }
            ApplicationError::TooManyBytes { field, max } => {
                Self::validation(format!("{field} must not exceed {max} bytes"))
            }
            ApplicationError::Validation(message) => Self::validation(message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = self.payload.request_id.clone();
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: self.payload,
            }),
        )
            .into_response();

        if let Some(request_id) = request_id
            && let Ok(value) = HeaderValue::from_str(&request_id)
        {
            response
                .headers_mut()
                .insert(HeaderName::from_static("x-request-id"), value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_details_are_not_returned_to_client() {
        let error =
            ApiError::from_store(StoreError::Database("password=secret host=internal".into()));
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.payload.message, "an internal server error occurred");
        assert!(!error.payload.message.contains("secret"));
    }

    #[test]
    fn validation_errors_are_safe_and_specific() {
        let error = ApiError::from(DomainError::NonFiniteMeasurement);
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.payload.code, "validation_error");
    }
}
