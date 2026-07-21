use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AnimalDirectory, AnimalResolution, CancellationCheck, ImportError, ImportIssue, IssueSeverity,
    MeasurementCatalog, MeasurementValueType, NoCancellation, TabularData, normalize_header,
    parse_date, row_issue, validate_mapping_columns,
};

const REQUIRED_COLUMNS: [&str; 5] = [
    "measurement_key",
    "value_type",
    "value",
    "unit",
    "measured_at",
];
const MEASUREMENT_MAPPING_TARGETS: [&str; 7] = [
    "animal_uuid",
    "display_id",
    "measurement_key",
    "value_type",
    "value",
    "unit",
    "measured_at",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementFieldMapping {
    /// Canonical measurement field -> source header.
    pub columns: BTreeMap<String, String>,
}

impl MeasurementFieldMapping {
    pub fn infer(headers: &[String]) -> Self {
        let aliases: BTreeMap<&str, &[&str]> = BTreeMap::from([
            (
                "animal_uuid",
                &["animal_uuid", "animal_id_uuid", "动物uuid", "小鼠uuid"] as &[_],
            ),
            (
                "display_id",
                &[
                    "display_id",
                    "animal_id",
                    "mouse_id",
                    "id",
                    "小鼠id",
                    "小鼠编号",
                    "动物编号",
                    "编号",
                ] as &[_],
            ),
            (
                "measurement_key",
                &[
                    "measurement_key",
                    "measurement",
                    "metric",
                    "指标",
                    "测量指标",
                ] as &[_],
            ),
            (
                "value_type",
                &["value_type", "type", "数据类型", "值类型"] as &[_],
            ),
            ("value", &["value", "result", "测量值", "结果"] as &[_]),
            ("unit", &["unit", "units", "单位"] as &[_]),
            (
                "measured_at",
                &[
                    "measured_at",
                    "measurement_time",
                    "datetime",
                    "timestamp",
                    "测量时间",
                    "时间",
                    "日期",
                ] as &[_],
            ),
        ]);
        let normalized: BTreeMap<String, String> = headers
            .iter()
            .map(|header| (normalize_header(header), header.clone()))
            .collect();
        let mut columns = BTreeMap::new();
        for (canonical, candidates) in aliases {
            if let Some(source) = candidates
                .iter()
                .find_map(|candidate| normalized.get(*candidate))
            {
                columns.insert(canonical.to_owned(), source.clone());
            }
        }
        Self { columns }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MeasurementImportValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Date(NaiveDate),
    Category(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementImportRow {
    pub source_row: usize,
    pub animal_id: Uuid,
    pub display_id: String,
    pub measurement_key: String,
    pub value_type: MeasurementValueType,
    pub value: MeasurementImportValue,
    pub unit: Option<String>,
    pub measured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementImportPreview {
    pub total_rows: usize,
    pub accepted_rows: Vec<MeasurementImportRow>,
    pub issues: Vec<ImportIssue>,
}

impl MeasurementImportPreview {
    pub fn can_confirm(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Error)
            .count()
    }
}

pub fn preview_measurements(
    table: &TabularData,
    mapping: &MeasurementFieldMapping,
    animals: &AnimalDirectory,
    catalog: &MeasurementCatalog,
) -> MeasurementImportPreview {
    preview_measurements_with_cancel(table, mapping, animals, catalog, &NoCancellation)
        .expect("NoCancellation cannot cancel")
}

pub fn preview_measurements_with_cancel<C>(
    table: &TabularData,
    mapping: &MeasurementFieldMapping,
    animals: &AnimalDirectory,
    catalog: &MeasurementCatalog,
    cancellation: &C,
) -> Result<MeasurementImportPreview, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let header_index: BTreeMap<&str, usize> = table
        .headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header.as_str(), index))
        .collect();
    let mut issues = validate_mapping_columns(
        &table.headers,
        &mapping.columns,
        &MEASUREMENT_MAPPING_TARGETS,
    );
    for field in REQUIRED_COLUMNS {
        if !mapping.columns.contains_key(field) {
            issues.push(ImportIssue {
                row: None,
                field: Some(field.to_owned()),
                severity: IssueSeverity::Error,
                code: "missing_required_mapping".to_owned(),
                message: format!("必须映射测量字段 {field}"),
            });
        }
    }
    let identity_fields = ["animal_uuid", "display_id"];
    if !identity_fields
        .iter()
        .any(|field| mapping.columns.contains_key(*field))
    {
        issues.push(ImportIssue {
            row: None,
            field: Some("animal_identity".to_owned()),
            severity: IssueSeverity::Error,
            code: "missing_animal_identity_mapping".to_owned(),
            message: "必须映射 animal_uuid 或动物显示编号".to_owned(),
        });
    }
    if !issues.is_empty() {
        return Ok(MeasurementImportPreview {
            total_rows: table.rows.len(),
            accepted_rows: Vec::new(),
            issues,
        });
    }

    let mut accepted_rows = Vec::new();
    let mut seen = BTreeSet::new();
    for (offset, raw) in table.rows.iter().enumerate() {
        cancellation.check_cancelled()?;
        let source_row = offset + 2;
        if raw.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        let value = |field: &str| -> Option<String> {
            mapping
                .columns
                .get(field)
                .and_then(|source| header_index.get(source.as_str()))
                .and_then(|index| raw.get(*index))
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };

        let supplied_display_id = value("display_id");
        let (animal_id, display_id) = if let Some(raw_uuid) = value("animal_uuid") {
            let Ok(animal_id) = Uuid::parse_str(&raw_uuid) else {
                issues.push(row_issue(
                    source_row,
                    "animal_uuid",
                    IssueSeverity::Error,
                    "invalid_animal_uuid",
                    "动物 UUID 格式无效",
                ));
                continue;
            };
            if !animals.contains_id(animal_id) {
                issues.push(row_issue(
                    source_row,
                    "animal_uuid",
                    IssueSeverity::Error,
                    "unknown_animal_uuid",
                    "动物 UUID 不属于当前可用动物目录",
                ));
                continue;
            }
            if let Some(display_id) = supplied_display_id.as_deref()
                && matches!(
                    animals.resolve(display_id),
                    AnimalResolution::Unique(other_id) if other_id != animal_id
                )
            {
                issues.push(row_issue(
                    source_row,
                    "display_id",
                    IssueSeverity::Error,
                    "animal_identity_mismatch",
                    "动物 UUID 与唯一显示编号指向不同动物",
                ));
                continue;
            }
            (
                animal_id,
                animals
                    .display_id(animal_id)
                    .map(str::to_owned)
                    .or(supplied_display_id)
                    .unwrap_or_default(),
            )
        } else {
            let Some(display_id) = supplied_display_id else {
                issues.push(row_issue(
                    source_row,
                    "display_id",
                    IssueSeverity::Error,
                    "missing_animal_identity",
                    "animal_uuid 和动物显示编号不能同时为空",
                ));
                continue;
            };
            let animal_id = match animals.resolve(&display_id) {
                AnimalResolution::Unique(animal_id) => animal_id,
                AnimalResolution::Unknown => {
                    issues.push(row_issue(
                        source_row,
                        "display_id",
                        IssueSeverity::Error,
                        "unknown_animal",
                        "动物编号在当前目录中不存在",
                    ));
                    continue;
                }
                AnimalResolution::Ambiguous => {
                    issues.push(row_issue(
                        source_row,
                        "display_id",
                        IssueSeverity::Error,
                        "ambiguous_animal",
                        "动物编号对应多个动物，必须提供 animal_uuid",
                    ));
                    continue;
                }
            };
            (animal_id, display_id)
        };

        let Some(measurement_key) = value("measurement_key") else {
            issues.push(row_issue(
                source_row,
                "measurement_key",
                IssueSeverity::Error,
                "missing_measurement_key",
                "测量指标不能为空",
            ));
            continue;
        };
        let Some(definition) = catalog.get(&measurement_key) else {
            issues.push(row_issue(
                source_row,
                "measurement_key",
                IssueSeverity::Error,
                "unknown_measurement_key",
                "测量指标未在目录中定义",
            ));
            continue;
        };

        let Some(value_type_label) = value("value_type") else {
            issues.push(row_issue(
                source_row,
                "value_type",
                IssueSeverity::Error,
                "missing_value_type",
                "值类型不能为空",
            ));
            continue;
        };
        let Some(value_type) = MeasurementValueType::parse_label(&value_type_label) else {
            issues.push(row_issue(
                source_row,
                "value_type",
                IssueSeverity::Error,
                "invalid_value_type",
                "值类型必须为 number、text、boolean、date 或 category",
            ));
            continue;
        };
        if value_type != definition.value_type() {
            issues.push(row_issue(
                source_row,
                "value_type",
                IssueSeverity::Error,
                "value_type_mismatch",
                "值类型与测量指标定义不一致",
            ));
            continue;
        }

        let Some(raw_value) = value("value") else {
            issues.push(row_issue(
                source_row,
                "value",
                IssueSeverity::Error,
                "missing_value",
                "测量值不能为空",
            ));
            continue;
        };
        let parsed_value = match parse_measurement_value(&raw_value, value_type) {
            Ok(value) => value,
            Err((code, message)) => {
                issues.push(row_issue(
                    source_row,
                    "value",
                    IssueSeverity::Error,
                    code,
                    message,
                ));
                continue;
            }
        };

        let unit = value("unit");
        if definition.unit_required() && unit.is_none() {
            issues.push(row_issue(
                source_row,
                "unit",
                IssueSeverity::Error,
                "missing_unit",
                "该测量指标必须提供单位",
            ));
            continue;
        }
        if let Some(unit) = unit.as_deref()
            && !definition.allowed_units().contains(unit)
        {
            issues.push(row_issue(
                source_row,
                "unit",
                IssueSeverity::Error,
                "invalid_unit",
                "单位不在该测量指标的允许列表中",
            ));
            continue;
        }

        let Some(measured_at_raw) = value("measured_at") else {
            issues.push(row_issue(
                source_row,
                "measured_at",
                IssueSeverity::Error,
                "missing_measured_at",
                "测量时间不能为空",
            ));
            continue;
        };
        let Some(measured_at) = parse_measured_at(&measured_at_raw) else {
            issues.push(row_issue(
                source_row,
                "measured_at",
                IssueSeverity::Error,
                "invalid_measured_at",
                "测量时间格式无效",
            ));
            continue;
        };

        if !seen.insert((animal_id, measurement_key.clone(), measured_at)) {
            issues.push(row_issue(
                source_row,
                "measurement_key",
                IssueSeverity::Error,
                "duplicate_measurement",
                "文件内存在同一动物、指标和时间的重复测量",
            ));
            continue;
        }
        accepted_rows.push(MeasurementImportRow {
            source_row,
            animal_id,
            display_id,
            measurement_key,
            value_type,
            value: parsed_value,
            unit,
            measured_at,
        });
    }

    Ok(MeasurementImportPreview {
        total_rows: table.rows.len(),
        accepted_rows,
        issues,
    })
}

fn parse_measurement_value(
    value: &str,
    value_type: MeasurementValueType,
) -> Result<MeasurementImportValue, (&'static str, &'static str)> {
    match value_type {
        MeasurementValueType::Number => {
            let number = value
                .parse::<f64>()
                .map_err(|_| ("invalid_number", "数值格式无效"))?;
            if !number.is_finite() {
                return Err(("non_finite_number", "数值不能为 NaN 或无穷大"));
            }
            Ok(MeasurementImportValue::Number(number))
        }
        MeasurementValueType::Text => Ok(MeasurementImportValue::Text(value.to_owned())),
        MeasurementValueType::Boolean => parse_boolean(value)
            .map(MeasurementImportValue::Boolean)
            .ok_or(("invalid_boolean", "布尔值格式无效")),
        MeasurementValueType::Date => parse_date(value)
            .map(MeasurementImportValue::Date)
            .ok_or(("invalid_date_value", "日期值格式无效")),
        MeasurementValueType::Category => Ok(MeasurementImportValue::Category(value.to_owned())),
    }
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "是" => Some(true),
        "false" | "0" | "no" | "n" | "否" => Some(false),
        _ => None,
    }
}

fn parse_measured_at(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Some(timestamp.with_timezone(&Utc));
    }
    let datetime_formats = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y.%m.%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M",
    ];
    if let Some(datetime) = datetime_formats
        .into_iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
    {
        return Some(DateTime::from_naive_utc_and_offset(datetime, Utc));
    }
    parse_date(value).map(|date| {
        DateTime::from_naive_utc_and_offset(NaiveDateTime::new(date, NaiveTime::MIN), Utc)
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rust_xlsxwriter::Workbook;

    use super::*;
    use crate::{CancellationToken, MeasurementDefinition, read_csv, read_xlsx};

    fn fixtures() -> (AnimalDirectory, MeasurementCatalog) {
        let animals = AnimalDirectory::from_entries([("M001", Uuid::nil())]).unwrap();
        let catalog = MeasurementCatalog::new([MeasurementDefinition::new(
            "body_weight",
            MeasurementValueType::Number,
            ["g"],
            true,
        )
        .unwrap()])
        .unwrap();
        (animals, catalog)
    }

    #[test]
    fn numeric_measurement_is_resolved_to_animal_uuid() {
        let table = read_csv(
            b"display_id,measurement_key,value_type,value,unit,measured_at\n\
              M001,body_weight,number,23.5,g,2026-07-18T08:30:00Z\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        assert!(preview.can_confirm(), "{:?}", preview.issues);
        assert_eq!(preview.accepted_rows[0].animal_id, Uuid::nil());
        assert_eq!(
            preview.accepted_rows[0].value,
            MeasurementImportValue::Number(23.5)
        );
    }

    #[test]
    fn explicit_uuid_resolves_one_of_173_duplicate_display_ids() {
        let entries = (1_u128..=173)
            .map(|value| ("M-DUP", Uuid::from_u128(value)))
            .collect::<Vec<_>>();
        let animals = AnimalDirectory::from_entries(entries).unwrap();
        assert_eq!(animals.resolve("M-DUP"), AnimalResolution::Ambiguous);
        let selected_id = Uuid::from_u128(173);
        let table = read_csv(
            format!(
                "animal_uuid,display_id,measurement_key,value_type,value,unit,measured_at\n\
                 {selected_id},M-DUP,body_weight,number,23.5,g,2026-07-18T08:30:00Z\n"
            )
            .as_bytes(),
        )
        .unwrap();
        let (_, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        assert!(preview.can_confirm(), "{:?}", preview.issues);
        assert_eq!(preview.accepted_rows[0].animal_id, selected_id);
        assert_eq!(preview.accepted_rows[0].display_id, "M-DUP");
    }

    #[test]
    fn explicit_uuid_and_different_unique_display_id_are_rejected() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let animals = AnimalDirectory::from_entries([("M1", first), ("M2", second)]).unwrap();
        let table = read_csv(
            format!(
                "animal_uuid,display_id,measurement_key,value_type,value,unit,measured_at\n\
                 {first},M2,body_weight,number,23.5,g,2026-07-18T08:30:00Z\n"
            )
            .as_bytes(),
        )
        .unwrap();
        let (_, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        assert!(!preview.can_confirm());
        assert!(
            preview
                .issues
                .iter()
                .any(|issue| issue.code == "animal_identity_mismatch")
        );
    }

    #[test]
    fn xlsx_numeric_cells_are_preserved_for_ids_and_values() {
        let mut workbook = Workbook::new();
        {
            let worksheet = workbook.add_worksheet();
            let headers = [
                "display_id",
                "measurement_key",
                "value_type",
                "value",
                "unit",
                "measured_at",
            ];
            for (column, header) in headers.iter().enumerate() {
                worksheet.write_string(0, column as u16, *header).unwrap();
            }
            worksheet.write_number(1, 0, 101.0).unwrap();
            worksheet.write_string(1, 1, "body_weight").unwrap();
            worksheet.write_string(1, 2, "number").unwrap();
            worksheet.write_number(1, 3, 23.75).unwrap();
            worksheet.write_string(1, 4, "g").unwrap();
            worksheet.write_string(1, 5, "2026-07-18 08:30").unwrap();
        }
        let table = read_xlsx(Cursor::new(workbook.save_to_buffer().unwrap())).unwrap();
        let animals = AnimalDirectory::from_entries([("101", Uuid::nil())]).unwrap();
        let (_, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        assert!(preview.can_confirm(), "{:?}", preview.issues);
        assert_eq!(preview.accepted_rows[0].display_id, "101");
        assert_eq!(
            preview.accepted_rows[0].value,
            MeasurementImportValue::Number(23.75)
        );
    }

    #[test]
    fn unknown_animal_bad_unit_non_finite_and_duplicate_are_rejected() {
        let table = read_csv(
            b"display_id,measurement_key,value_type,value,unit,measured_at\n\
              M404,body_weight,number,20,g,2026-07-18\n\
              M001,body_weight,number,20,kg,2026-07-18\n\
              M001,body_weight,number,NaN,g,2026-07-18\n\
              M001,body_weight,number,20,g,2026-07-18\n\
              M001,body_weight,number,21,g,2026-07-18\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        let codes = preview
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"unknown_animal"));
        assert!(codes.contains(&"invalid_unit"));
        assert!(codes.contains(&"non_finite_number"));
        assert!(codes.contains(&"duplicate_measurement"));
        assert_eq!(preview.accepted_rows.len(), 1);
    }

    #[test]
    fn missing_value_and_required_unit_are_rejected() {
        let table = read_csv(
            b"display_id,measurement_key,value_type,value,unit,measured_at\n\
              M001,body_weight,number,,g,2026-07-18\n\
              M001,body_weight,number,20,,2026-07-19\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let preview = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        let codes = preview
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"missing_value"));
        assert!(codes.contains(&"missing_unit"));
        assert!(preview.accepted_rows.is_empty());
    }

    #[test]
    fn explicit_measurement_mapping_repairs_custom_headers() {
        let table = read_csv(
            b"mouse,metric_name,kind,result_value,result_unit,when\n\
              M001,body_weight,number,20,g,2026-07-18\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let inferred = preview_measurements(
            &table,
            &MeasurementFieldMapping::infer(&table.headers),
            &animals,
            &catalog,
        );
        assert!(!inferred.can_confirm());

        let mapping = MeasurementFieldMapping {
            columns: BTreeMap::from([
                ("display_id".to_owned(), "mouse".to_owned()),
                ("measurement_key".to_owned(), "metric_name".to_owned()),
                ("value_type".to_owned(), "kind".to_owned()),
                ("value".to_owned(), "result_value".to_owned()),
                ("unit".to_owned(), "result_unit".to_owned()),
                ("measured_at".to_owned(), "when".to_owned()),
            ]),
        };
        let repaired = preview_measurements(&table, &mapping, &animals, &catalog);
        assert!(repaired.can_confirm(), "{:?}", repaired.issues);
        assert_eq!(repaired.accepted_rows.len(), 1);
    }

    #[test]
    fn measurement_mapping_rejects_unknown_target_and_reused_source() {
        let table = read_csv(
            b"mouse,metric,kind,value,unit,when\n\
              M001,body_weight,number,20,g,2026-07-18\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let mapping = MeasurementFieldMapping {
            columns: BTreeMap::from([
                ("display_id".to_owned(), "mouse".to_owned()),
                ("measurement_key".to_owned(), "metric".to_owned()),
                ("value_type".to_owned(), "kind".to_owned()),
                ("value".to_owned(), "value".to_owned()),
                ("unit".to_owned(), "when".to_owned()),
                ("measured_at".to_owned(), "when".to_owned()),
                ("unknown".to_owned(), "unit".to_owned()),
            ]),
        };
        let preview = preview_measurements(&table, &mapping, &animals, &catalog);
        assert!(
            preview
                .issues
                .iter()
                .any(|issue| issue.code == "unknown_mapping_target")
        );
        assert!(
            preview
                .issues
                .iter()
                .any(|issue| issue.code == "duplicate_source_mapping")
        );
        assert!(!preview.can_confirm());
    }

    #[test]
    fn cancellation_stops_preview_before_rows_are_returned() {
        let table = read_csv(
            b"display_id,measurement_key,value_type,value,unit,measured_at\n\
              M001,body_weight,number,20,g,2026-07-18\n"
                .as_slice(),
        )
        .unwrap();
        let (animals, catalog) = fixtures();
        let token = CancellationToken::default();
        token.cancel();
        assert!(matches!(
            preview_measurements_with_cancel(
                &table,
                &MeasurementFieldMapping::infer(&table.headers),
                &animals,
                &catalog,
                &token,
            ),
            Err(ImportError::Cancelled)
        ));
    }
}
