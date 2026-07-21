use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_FILTERS: usize = 20;
pub const MAX_SORTS: usize = 3;
pub const MAX_SELECTED_FIELDS: usize = 24;
pub const MAX_PAGE_SIZE: u16 = 100;
pub const MAX_OFFSET: u32 = 10_000;
pub const MAX_LIST_VALUES: usize = 50;
pub const MAX_TEXT_VALUE_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryResource {
    Animal,
    Cage,
    Project,
    Experiment,
    Measurement,
    Sample,
    Job,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryField {
    Id,
    LabId,
    ProjectId,
    AnimalId,
    CageId,
    ExperimentId,
    DisplayId,
    Name,
    Species,
    Sex,
    Status,
    MeasurementType,
    SampleType,
    ValueNumber,
    ValueBoolean,
    ValueText,
    Unit,
    CreatedAt,
    UpdatedAt,
    OccurredAt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum QueryValue {
    Null,
    Text(String),
    Number(f64),
    Boolean(bool),
    Uuid(Uuid),
    Timestamp(DateTime<Utc>),
    TextList(Vec<String>),
    NumberList(Vec<f64>),
    BooleanList(Vec<bool>),
    UuidList(Vec<Uuid>),
    TimestampList(Vec<DateTime<Utc>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Eq,
    NotEq,
    Contains,
    StartsWith,
    In,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Between,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterClause {
    pub field: QueryField,
    pub operator: FilterOperator,
    pub value: QueryValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortSpec {
    pub field: QueryField,
    #[serde(default = "default_sort_direction")]
    pub direction: SortDirection,
}

const fn default_sort_direction() -> SortDirection {
    SortDirection::Asc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSpec {
    #[serde(default = "default_page_limit")]
    pub limit: u16,
    #[serde(default)]
    pub offset: u32,
}

const fn default_page_limit() -> u16 {
    50
}

impl Default for PageSpec {
    fn default() -> Self {
        Self {
            limit: default_page_limit(),
            offset: 0,
        }
    }
}

/// Untrusted query request as produced by a UI or model.
///
/// Unknown properties are rejected. In particular, a property named sql or
/// raw_sql can never be smuggled into this request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    pub resource: QueryResource,
    #[serde(default)]
    pub fields: Vec<QueryField>,
    #[serde(default)]
    pub filters: Vec<FilterClause>,
    #[serde(default)]
    pub sort: Vec<SortSpec>,
    #[serde(default)]
    pub page: PageSpec,
}

/// A validated domain query plan.
///
/// The inner request remains private so callers cannot construct a trusted plan
/// without validation. Store implementations must translate fields to their own
/// prepared, parameter-bound queries; this type never compiles or carries SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct SafeQuery(QueryRequest);

impl SafeQuery {
    pub fn resource(&self) -> QueryResource {
        self.0.resource
    }

    pub fn fields(&self) -> &[QueryField] {
        &self.0.fields
    }

    pub fn filters(&self) -> &[FilterClause] {
        &self.0.filters
    }

    pub fn sort(&self) -> &[SortSpec] {
        &self.0.sort
    }

    pub fn page(&self) -> PageSpec {
        self.0.page
    }

    pub fn into_request(self) -> QueryRequest {
        self.0
    }
}

impl TryFrom<QueryRequest> for SafeQuery {
    type Error = ValidationError;

    fn try_from(request: QueryRequest) -> Result<Self, Self::Error> {
        validate_request(&request)?;
        Ok(Self(request))
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("too many filters: maximum is {maximum}")]
    TooManyFilters { maximum: usize },
    #[error("too many sort fields: maximum is {maximum}")]
    TooManySorts { maximum: usize },
    #[error("too many selected fields: maximum is {maximum}")]
    TooManySelectedFields { maximum: usize },
    #[error("page limit must be between 1 and {maximum}")]
    InvalidPageLimit { maximum: u16 },
    #[error("page offset exceeds {maximum}")]
    OffsetTooLarge { maximum: u32 },
    #[error("field {field:?} is not available for {resource:?}")]
    FieldNotAllowed {
        resource: QueryResource,
        field: QueryField,
    },
    #[error("selected field appears more than once: {field:?}")]
    DuplicateField { field: QueryField },
    #[error("operator {operator:?} is invalid for field {field:?}")]
    OperatorNotAllowed {
        field: QueryField,
        operator: FilterOperator,
    },
    #[error("invalid value for field {field:?}: {reason}")]
    InvalidValue {
        field: QueryField,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    Keyword,
    Number,
    Boolean,
    Uuid,
    Timestamp,
}

fn validate_request(request: &QueryRequest) -> Result<(), ValidationError> {
    if request.filters.len() > MAX_FILTERS {
        return Err(ValidationError::TooManyFilters {
            maximum: MAX_FILTERS,
        });
    }
    if request.sort.len() > MAX_SORTS {
        return Err(ValidationError::TooManySorts { maximum: MAX_SORTS });
    }
    if request.fields.len() > MAX_SELECTED_FIELDS {
        return Err(ValidationError::TooManySelectedFields {
            maximum: MAX_SELECTED_FIELDS,
        });
    }
    if request.page.limit == 0 || request.page.limit > MAX_PAGE_SIZE {
        return Err(ValidationError::InvalidPageLimit {
            maximum: MAX_PAGE_SIZE,
        });
    }
    if request.page.offset > MAX_OFFSET {
        return Err(ValidationError::OffsetTooLarge {
            maximum: MAX_OFFSET,
        });
    }

    let mut fields = BTreeSet::new();
    for field in &request.fields {
        validate_field(request.resource, *field)?;
        if !fields.insert(*field) {
            return Err(ValidationError::DuplicateField { field: *field });
        }
    }

    for filter in &request.filters {
        validate_field(request.resource, filter.field)?;
        validate_filter(filter)?;
    }

    for sort in &request.sort {
        validate_field(request.resource, sort.field)?;
    }

    Ok(())
}

fn validate_field(resource: QueryResource, field: QueryField) -> Result<(), ValidationError> {
    use QueryField::*;
    use QueryResource::*;

    let allowed = match resource {
        Animal => matches!(
            field,
            Id | LabId
                | ProjectId
                | CageId
                | DisplayId
                | Name
                | Species
                | Sex
                | Status
                | CreatedAt
                | UpdatedAt
        ),
        Cage => matches!(
            field,
            Id | LabId | DisplayId | Name | Status | CreatedAt | UpdatedAt
        ),
        Project => matches!(field, Id | LabId | Name | Status | CreatedAt | UpdatedAt),
        Experiment => matches!(
            field,
            Id | ProjectId | Name | Status | CreatedAt | UpdatedAt
        ),
        Measurement => matches!(
            field,
            Id | ProjectId
                | ExperimentId
                | AnimalId
                | MeasurementType
                | ValueNumber
                | ValueBoolean
                | ValueText
                | Unit
                | Status
                | OccurredAt
                | CreatedAt
                | UpdatedAt
        ),
        Sample => matches!(
            field,
            Id | ProjectId | ExperimentId | AnimalId | SampleType | Status | CreatedAt | UpdatedAt
        ),
        Job => matches!(
            field,
            Id | ProjectId | Name | Status | CreatedAt | UpdatedAt
        ),
    };

    if allowed {
        Ok(())
    } else {
        Err(ValidationError::FieldNotAllowed { resource, field })
    }
}

fn field_kind(field: QueryField) -> FieldKind {
    use QueryField::*;

    match field {
        Id | LabId | ProjectId | AnimalId | CageId | ExperimentId => FieldKind::Uuid,
        DisplayId | Name | ValueText | Unit => FieldKind::Text,
        Species | Sex | Status | MeasurementType | SampleType => FieldKind::Keyword,
        ValueNumber => FieldKind::Number,
        ValueBoolean => FieldKind::Boolean,
        CreatedAt | UpdatedAt | OccurredAt => FieldKind::Timestamp,
    }
}

fn validate_filter(filter: &FilterClause) -> Result<(), ValidationError> {
    use FilterOperator::*;

    let kind = field_kind(filter.field);
    let operator_allowed = match filter.operator {
        Eq | NotEq | In | IsNull | IsNotNull => true,
        Contains | StartsWith => matches!(kind, FieldKind::Text),
        GreaterThan | GreaterThanOrEqual | LessThan | LessThanOrEqual | Between => {
            matches!(kind, FieldKind::Number | FieldKind::Timestamp)
        }
    };

    if !operator_allowed {
        return Err(ValidationError::OperatorNotAllowed {
            field: filter.field,
            operator: filter.operator,
        });
    }

    match filter.operator {
        IsNull | IsNotNull => {
            if !matches!(filter.value, QueryValue::Null) {
                return invalid(filter.field, "null operator requires a null value");
            }
        }
        In => validate_list_value(filter.field, kind, &filter.value)?,
        Between => validate_range_value(filter.field, kind, &filter.value)?,
        Contains | StartsWith => match &filter.value {
            QueryValue::Text(value) => validate_text(filter.field, value)?,
            _ => return invalid(filter.field, "text operator requires a text value"),
        },
        Eq | NotEq | GreaterThan | GreaterThanOrEqual | LessThan | LessThanOrEqual => {
            validate_scalar_value(filter.field, kind, &filter.value)?
        }
    }

    Ok(())
}

fn validate_scalar_value(
    field: QueryField,
    kind: FieldKind,
    value: &QueryValue,
) -> Result<(), ValidationError> {
    let valid = matches!(
        (kind, value),
        (FieldKind::Text | FieldKind::Keyword, QueryValue::Text(_))
            | (FieldKind::Number, QueryValue::Number(_))
            | (FieldKind::Boolean, QueryValue::Boolean(_))
            | (FieldKind::Uuid, QueryValue::Uuid(_))
            | (FieldKind::Timestamp, QueryValue::Timestamp(_))
    );

    if !valid {
        return invalid(field, "value type does not match the field");
    }

    match value {
        QueryValue::Text(value) => validate_text(field, value),
        QueryValue::Number(value) if !value.is_finite() => {
            invalid(field, "numeric values must be finite")
        }
        _ => Ok(()),
    }
}

fn validate_list_value(
    field: QueryField,
    kind: FieldKind,
    value: &QueryValue,
) -> Result<(), ValidationError> {
    let length = match (kind, value) {
        (FieldKind::Text | FieldKind::Keyword, QueryValue::TextList(values)) => {
            for value in values {
                validate_text(field, value)?;
            }
            values.len()
        }
        (FieldKind::Number, QueryValue::NumberList(values)) => {
            if values.iter().any(|value| !value.is_finite()) {
                return invalid(field, "numeric values must be finite");
            }
            values.len()
        }
        (FieldKind::Boolean, QueryValue::BooleanList(values)) => values.len(),
        (FieldKind::Uuid, QueryValue::UuidList(values)) => values.len(),
        (FieldKind::Timestamp, QueryValue::TimestampList(values)) => values.len(),
        _ => return invalid(field, "list type does not match the field"),
    };

    if length == 0 || length > MAX_LIST_VALUES {
        return invalid(field, "list size is outside the allowed range");
    }

    Ok(())
}

fn validate_range_value(
    field: QueryField,
    kind: FieldKind,
    value: &QueryValue,
) -> Result<(), ValidationError> {
    match (kind, value) {
        (FieldKind::Number, QueryValue::NumberList(values))
            if values.len() == 2
                && values.iter().all(|value| value.is_finite())
                && values[0] <= values[1] =>
        {
            Ok(())
        }
        (FieldKind::Timestamp, QueryValue::TimestampList(values))
            if values.len() == 2 && values[0] <= values[1] =>
        {
            Ok(())
        }
        (FieldKind::Number, _) => {
            invalid(field, "number range must contain two ordered finite values")
        }
        (FieldKind::Timestamp, _) => {
            invalid(field, "timestamp range must contain two ordered values")
        }
        _ => invalid(field, "range is not supported for this field"),
    }
}

fn validate_text(field: QueryField, value: &str) -> Result<(), ValidationError> {
    if value.len() > MAX_TEXT_VALUE_BYTES {
        invalid(field, "text value is too long")
    } else {
        Ok(())
    }
}

fn invalid<T>(field: QueryField, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidValue { field, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animal_query() -> QueryRequest {
        QueryRequest {
            resource: QueryResource::Animal,
            fields: vec![QueryField::Id, QueryField::DisplayId],
            filters: vec![FilterClause {
                field: QueryField::Status,
                operator: FilterOperator::Eq,
                value: QueryValue::Text("active".into()),
            }],
            sort: vec![SortSpec {
                field: QueryField::DisplayId,
                direction: SortDirection::Asc,
            }],
            page: PageSpec {
                limit: 25,
                offset: 0,
            },
        }
    }

    #[test]
    fn valid_request_becomes_safe_query() {
        let query = SafeQuery::try_from(animal_query()).unwrap();
        assert_eq!(query.resource(), QueryResource::Animal);
        assert_eq!(query.page().limit, 25);
    }

    #[test]
    fn rejects_fields_from_another_resource() {
        let mut request = animal_query();
        request.filters.push(FilterClause {
            field: QueryField::MeasurementType,
            operator: FilterOperator::Eq,
            value: QueryValue::Text("body_weight".into()),
        });

        assert_eq!(
            SafeQuery::try_from(request).unwrap_err(),
            ValidationError::FieldNotAllowed {
                resource: QueryResource::Animal,
                field: QueryField::MeasurementType,
            }
        );
    }

    #[test]
    fn rejects_operator_and_value_mismatch() {
        let mut request = animal_query();
        request.filters[0] = FilterClause {
            field: QueryField::Status,
            operator: FilterOperator::Contains,
            value: QueryValue::Text("act".into()),
        };

        assert!(matches!(
            SafeQuery::try_from(request),
            Err(ValidationError::OperatorNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_oversized_page() {
        let mut request = animal_query();
        request.page.limit = MAX_PAGE_SIZE + 1;

        assert!(matches!(
            SafeQuery::try_from(request),
            Err(ValidationError::InvalidPageLimit { .. })
        ));
    }

    #[test]
    fn deserialization_rejects_raw_sql_property() {
        let result = serde_json::from_value::<QueryRequest>(serde_json::json!({
            "resource": "animal",
            "sql": "select * from animals",
            "fields": [],
            "filters": []
        }));

        assert!(result.is_err());
    }

    #[test]
    fn text_is_data_not_executable_sql() {
        let mut request = animal_query();
        request.filters[0] = FilterClause {
            field: QueryField::DisplayId,
            operator: FilterOperator::Eq,
            value: QueryValue::Text("M001'; DROP TABLE animals; --".into()),
        };

        let safe = SafeQuery::try_from(request).unwrap();
        assert!(matches!(safe.filters()[0].value, QueryValue::Text(_)));
    }
}
