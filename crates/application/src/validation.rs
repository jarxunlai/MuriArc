use crate::{ApplicationError, ApplicationResult};

pub(crate) fn normalized_required(
    field: &'static str,
    value: String,
    max: usize,
) -> ApplicationResult<String> {
    let value = value.trim().to_owned();
    ensure_max_chars(field, &value, max)?;
    Ok(value)
}

pub(crate) fn normalized_optional(
    field: &'static str,
    value: Option<String>,
    max: usize,
) -> ApplicationResult<Option<String>> {
    value
        .map(|value| {
            let value = value.trim().to_owned();
            ensure_max_chars(field, &value, max)?;
            Ok((!value.is_empty()).then_some(value))
        })
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn ensure_max_chars(
    field: &'static str,
    value: &str,
    max: usize,
) -> ApplicationResult<()> {
    if value.chars().count() > max {
        Err(ApplicationError::TooLong { field, max })
    } else {
        Ok(())
    }
}

pub(crate) fn normalized_required_bytes(
    field: &'static str,
    value: String,
    max: usize,
) -> ApplicationResult<String> {
    let value = value.trim().to_owned();
    ensure_max_bytes(field, &value, max)?;
    Ok(value)
}

pub(crate) fn normalized_optional_bytes(
    field: &'static str,
    value: Option<String>,
    max: usize,
) -> ApplicationResult<Option<String>> {
    value
        .map(|value| {
            let value = value.trim().to_owned();
            ensure_max_bytes(field, &value, max)?;
            Ok((!value.is_empty()).then_some(value))
        })
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn ensure_max_bytes(
    field: &'static str,
    value: &str,
    max: usize,
) -> ApplicationResult<()> {
    if value.len() > max {
        Err(ApplicationError::TooManyBytes { field, max })
    } else {
        Ok(())
    }
}
