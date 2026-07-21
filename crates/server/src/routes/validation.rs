use crate::{ApiError, RequestMetadata};

pub(super) const DEFAULT_COLLECTION_LIMIT: usize = 200;
pub(super) const MAX_COLLECTION_LIMIT: usize = 500;

pub(super) fn required_text(
    value: String,
    field: &'static str,
    maximum_bytes: usize,
    metadata: &RequestMetadata,
) -> Result<String, ApiError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(validation(format!("{field} must not be empty"), metadata));
    }
    if value.len() > maximum_bytes {
        return Err(validation(
            format!("{field} must not exceed {maximum_bytes} bytes"),
            metadata,
        ));
    }
    Ok(value)
}

pub(super) fn optional_text(
    value: Option<String>,
    field: &'static str,
    maximum_bytes: usize,
    metadata: &RequestMetadata,
) -> Result<Option<String>, ApiError> {
    value
        .map(|value| required_text(value, field, maximum_bytes, metadata))
        .transpose()
}

pub(super) fn collection_limit(
    value: Option<usize>,
    metadata: &RequestMetadata,
) -> Result<usize, ApiError> {
    match value.unwrap_or(DEFAULT_COLLECTION_LIMIT) {
        1..=MAX_COLLECTION_LIMIT => Ok(value.unwrap_or(DEFAULT_COLLECTION_LIMIT)),
        _ => Err(validation(
            format!("limit must be between 1 and {MAX_COLLECTION_LIMIT}"),
            metadata,
        )),
    }
}

pub(super) fn truncate<T>(values: &mut Vec<T>, limit: usize) {
    values.truncate(limit);
}

pub(super) fn validation(message: impl Into<String>, metadata: &RequestMetadata) -> ApiError {
    ApiError::validation(message).with_request_id(metadata.request_id.clone())
}
