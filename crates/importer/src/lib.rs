#![forbid(unsafe_code)]

mod animal_schema;
mod cancellation;
mod catalog;
mod export;
mod measurement;
mod plan;

pub use animal_schema::{
    ANIMAL_IMPORT_HEADERS, AnimalImportExample, AnimalImportFieldSpec, AnimalImportFieldType,
    AnimalImportSchema, animal_import_schema, animal_import_template_csv,
    animal_import_template_xlsx,
};
pub use cancellation::{CancellationCheck, CancellationToken, NoCancellation};
pub use catalog::{
    AnimalDirectory, AnimalResolution, CatalogError, MeasurementCatalog, MeasurementDefinition,
    MeasurementValueType,
};
pub use export::{
    AnimalExportField, AnimalExportFilter, AnimalExportOptions, AnimalExportRecord,
    ExportAnimalStatus, ExportCage, ExportGenotype, ExportGenotypingState, ExportSex,
    export_animals_csv, export_animals_csv_with_cancel, export_animals_csv_with_options,
    export_animals_xlsx, export_animals_xlsx_with_cancel, export_animals_xlsx_with_options,
    filter_animals, filter_animals_with_cancel,
};
pub use measurement::{
    MeasurementFieldMapping, MeasurementImportPreview, MeasurementImportRow,
    MeasurementImportValue, preview_measurements, preview_measurements_with_cancel,
};
pub use plan::{
    CageDirectory, DirectoryError, DirectoryResolution, GeneticDirectory, ImportPlanBuildError,
    ImportPlanContext, MeasurementImportPlanContext, build_animal_import_plan,
    build_measurement_import_plan,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, Write},
    path::Path,
};

use calamine::{Data, Reader, Xlsx};
use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const REQUIRED_FIELD: &str = "display_id";
const ANIMAL_MAPPING_TARGETS: [&str; 8] = [
    "display_id",
    "sex",
    "birth_date",
    "strain",
    "cage",
    "genotype",
    "father",
    "mother",
];

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("unsupported file extension: {0}")]
    UnsupportedFormat(String),
    #[error("file has no readable worksheet")]
    MissingWorksheet,
    #[error("tabular input has no header row")]
    MissingHeader,
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("XLSX error: {0}")]
    Xlsx(#[from] calamine::XlsxError),
    #[error("XLSX write error: {0}")]
    XlsxWrite(#[from] rust_xlsxwriter::XlsxError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabularData {
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMapping {
    /// Canonical field -> source header.
    pub columns: BTreeMap<String, String>,
}

impl FieldMapping {
    pub fn infer(headers: &[String]) -> Self {
        let aliases: BTreeMap<&str, &[&str]> = BTreeMap::from([
            (
                "display_id",
                &["display_id", "id", "mouse_id", "小鼠id", "小鼠编号", "编号"] as &[_],
            ),
            ("sex", &["sex", "gender", "性别"] as &[_]),
            (
                "birth_date",
                &["birth_date", "birthday", "dob", "出生日期"] as &[_],
            ),
            ("strain", &["strain", "品系"] as &[_]),
            ("cage", &["cage", "cage_id", "笼位", "笼位id"] as &[_]),
            ("genotype", &["genotype", "基因型"] as &[_]),
            ("father", &["father", "sire", "父本"] as &[_]),
            ("mother", &["mother", "dam", "母本"] as &[_]),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportIssue {
    pub row: Option<usize>,
    pub field: Option<String>,
    pub severity: IssueSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalImportRow {
    pub source_row: usize,
    pub display_id: String,
    pub sex: Option<String>,
    pub birth_date: Option<NaiveDate>,
    pub strain: Option<String>,
    pub cage: Option<String>,
    pub genotype: Option<String>,
    pub father: Option<String>,
    pub mother: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub total_rows: usize,
    pub accepted_rows: Vec<AnimalImportRow>,
    pub issues: Vec<ImportIssue>,
}

impl ImportPreview {
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

pub fn read_path(path: impl AsRef<Path>) -> Result<TabularData, ImportError> {
    read_path_with_cancel(path, &NoCancellation)
}

pub fn read_path_with_cancel<C>(
    path: impl AsRef<Path>,
    cancellation: &C,
) -> Result<TabularData, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let path = path.as_ref();
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => read_csv_with_cancel(std::fs::File::open(path)?, cancellation),
        Some("xlsx") => read_xlsx_with_cancel(std::fs::File::open(path)?, cancellation),
        other => Err(ImportError::UnsupportedFormat(
            other.unwrap_or_default().to_owned(),
        )),
    }
}

pub fn read_csv(reader: impl Read) -> Result<TabularData, ImportError> {
    read_csv_with_cancel(reader, &NoCancellation)
}

pub fn read_csv_with_cancel<R, C>(reader: R, cancellation: &C) -> Result<TabularData, ImportError>
where
    R: Read,
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut csv = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(reader);
    let headers = csv
        .headers()?
        .iter()
        .map(strip_bom)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if headers.is_empty() {
        return Err(ImportError::MissingHeader);
    }
    let mut rows = Vec::new();
    for record in csv.records() {
        cancellation.check_cancelled()?;
        let record = record?;
        rows.push(
            (0..headers.len())
                .map(|index| record.get(index).unwrap_or_default().trim().to_owned())
                .collect(),
        );
    }
    Ok(TabularData {
        sheet_name: "csv".to_owned(),
        headers,
        rows,
    })
}

pub fn read_xlsx<R: Read + Seek>(reader: R) -> Result<TabularData, ImportError> {
    read_xlsx_with_cancel(reader, &NoCancellation)
}

pub fn read_xlsx_with_cancel<R, C>(reader: R, cancellation: &C) -> Result<TabularData, ImportError>
where
    R: Read + Seek,
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut workbook = Xlsx::new(reader)?;
    cancellation.check_cancelled()?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or(ImportError::MissingWorksheet)?;
    let range = workbook.worksheet_range(&sheet_name)?;
    let mut rows = range.rows();
    let headers = rows
        .next()
        .ok_or(ImportError::MissingHeader)?
        .iter()
        .map(cell_text)
        .collect::<Vec<_>>();
    if headers.iter().all(String::is_empty) {
        return Err(ImportError::MissingHeader);
    }
    let mut data_rows = Vec::new();
    for row in rows {
        cancellation.check_cancelled()?;
        data_rows.push(
            (0..headers.len())
                .map(|index| row.get(index).map(cell_text).unwrap_or_default())
                .collect(),
        );
    }
    Ok(TabularData {
        sheet_name,
        headers,
        rows: data_rows,
    })
}

pub fn preview_animals(table: &TabularData, mapping: &FieldMapping) -> ImportPreview {
    preview_animals_inner(table, mapping, None, &NoCancellation)
        .expect("NoCancellation cannot cancel")
}

pub fn preview_animals_with_cancel<C>(
    table: &TabularData,
    mapping: &FieldMapping,
    cancellation: &C,
) -> Result<ImportPreview, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    preview_animals_inner(table, mapping, None, cancellation)
}

pub fn preview_animals_with_directory(
    table: &TabularData,
    mapping: &FieldMapping,
    directory: &AnimalDirectory,
) -> ImportPreview {
    preview_animals_inner(table, mapping, Some(directory), &NoCancellation)
        .expect("NoCancellation cannot cancel")
}

pub fn preview_animals_with_directory_and_cancel<C>(
    table: &TabularData,
    mapping: &FieldMapping,
    directory: &AnimalDirectory,
    cancellation: &C,
) -> Result<ImportPreview, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    preview_animals_inner(table, mapping, Some(directory), cancellation)
}

fn preview_animals_inner<C>(
    table: &TabularData,
    mapping: &FieldMapping,
    directory: Option<&AnimalDirectory>,
    cancellation: &C,
) -> Result<ImportPreview, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut issues =
        validate_mapping_columns(&table.headers, &mapping.columns, &ANIMAL_MAPPING_TARGETS);
    if !mapping.columns.contains_key(REQUIRED_FIELD) {
        issues.push(ImportIssue {
            row: None,
            field: Some(REQUIRED_FIELD.to_owned()),
            severity: IssueSeverity::Error,
            code: "missing_required_mapping".to_owned(),
            message: "必须映射小鼠编号字段".to_owned(),
        });
    }
    if !issues.is_empty() {
        return Ok(ImportPreview {
            total_rows: table.rows.len(),
            accepted_rows: Vec::new(),
            issues,
        });
    }
    let header_index: BTreeMap<&str, usize> = table
        .headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header.as_str(), index))
        .collect();
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
        let display_id = value(REQUIRED_FIELD).unwrap_or_default();
        if display_id.is_empty() {
            issues.push(row_issue(
                source_row,
                REQUIRED_FIELD,
                IssueSeverity::Error,
                "missing_display_id",
                "小鼠编号不能为空",
            ));
            continue;
        }
        if !seen.insert(display_id.clone()) {
            issues.push(row_issue(
                source_row,
                REQUIRED_FIELD,
                IssueSeverity::Error,
                "duplicate_in_file",
                "文件内小鼠编号重复",
            ));
            continue;
        }
        if directory.is_some_and(|directory| directory.contains(&display_id)) {
            issues.push(row_issue(
                source_row,
                REQUIRED_FIELD,
                IssueSeverity::Error,
                "existing_display_id",
                "小鼠编号已存在于动物目录",
            ));
            continue;
        }
        let sex = value("sex").map(|sex| normalize_sex(&sex));
        if matches!(sex.as_deref(), Some("unknown")) {
            issues.push(row_issue(
                source_row,
                "sex",
                IssueSeverity::Warning,
                "unknown_sex",
                "性别无法识别，将作为 unknown 导入",
            ));
        }
        let birth_raw = value("birth_date");
        let birth_date = birth_raw.as_deref().and_then(parse_date);
        if birth_raw.is_some() && birth_date.is_none() {
            issues.push(row_issue(
                source_row,
                "birth_date",
                IssueSeverity::Error,
                "invalid_date",
                "出生日期格式无效，应为 YYYY-MM-DD、YYYY/MM/DD 或 Excel 日期",
            ));
            continue;
        }
        accepted_rows.push(AnimalImportRow {
            source_row,
            display_id,
            sex,
            birth_date,
            strain: value("strain"),
            cage: value("cage"),
            genotype: value("genotype"),
            father: value("father"),
            mother: value("mother"),
        });
    }
    Ok(ImportPreview {
        total_rows: table.rows.len(),
        accepted_rows,
        issues,
    })
}

pub fn write_animals_csv(rows: &[AnimalImportRow], writer: impl Write) -> Result<(), ImportError> {
    write_animals_csv_with_cancel(rows, writer, &NoCancellation)
}

pub fn write_animals_csv_with_cancel<W, C>(
    rows: &[AnimalImportRow],
    writer: W,
    cancellation: &C,
) -> Result<(), ImportError>
where
    W: Write,
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut csv = csv::Writer::from_writer(writer);
    csv.write_record([
        "display_id",
        "sex",
        "birth_date",
        "strain",
        "cage",
        "genotype",
        "father",
        "mother",
    ])?;
    for row in rows {
        cancellation.check_cancelled()?;
        let values = [
            row.display_id.clone(),
            row.sex.clone().unwrap_or_default(),
            row.birth_date
                .map(|date| date.to_string())
                .unwrap_or_default(),
            row.strain.clone().unwrap_or_default(),
            row.cage.clone().unwrap_or_default(),
            row.genotype.clone().unwrap_or_default(),
            row.father.clone().unwrap_or_default(),
            row.mother.clone().unwrap_or_default(),
        ];
        csv.write_record(values.iter().map(|value| safe_csv_cell(value)))?;
    }
    cancellation.check_cancelled()?;
    csv.flush()?;
    Ok(())
}

/// Prevents spreadsheet applications from evaluating untrusted CSV cells as
/// formulas. XLSX exports use explicit string cells and do not need this
/// transformation.
pub(crate) fn safe_csv_cell(value: &str) -> String {
    let candidate = value.trim_start_matches([' ', '\t', '\r', '\n']);
    if candidate
        .chars()
        .next()
        .is_some_and(|character| matches!(character, '=' | '+' | '-' | '@'))
    {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.trim().to_owned(),
        Data::Float(value) if value.fract() == 0.0 => format!("{value:.0}"),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::DateTime(value) => value
            .as_datetime()
            .map(|value| {
                if value.time() == NaiveTime::MIN {
                    value.date().to_string()
                } else {
                    value.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
                }
            })
            .unwrap_or_else(|| value.to_string()),
        Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        Data::Error(value) => format!("#ERROR:{value:?}"),
    }
}

fn normalize_header(value: &str) -> String {
    strip_bom(value)
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}
fn strip_bom(value: &str) -> &str {
    value.strip_prefix('\u{feff}').unwrap_or(value)
}
fn normalize_sex(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "m" | "male" | "雄" | "雄性" => "male".to_owned(),
        "f" | "female" | "雌" | "雌性" => "female".to_owned(),
        _ => "unknown".to_owned(),
    }
}
fn parse_date(value: &str) -> Option<NaiveDate> {
    ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"]
        .into_iter()
        .find_map(|format| NaiveDate::parse_from_str(value.trim(), format).ok())
}
fn row_issue(
    row: usize,
    field: &str,
    severity: IssueSeverity,
    code: &str,
    message: &str,
) -> ImportIssue {
    ImportIssue {
        row: Some(row),
        field: Some(field.to_owned()),
        severity,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

pub(crate) fn validate_mapping_columns(
    headers: &[String],
    columns: &BTreeMap<String, String>,
    allowed_targets: &[&str],
) -> Vec<ImportIssue> {
    let allowed = allowed_targets.iter().copied().collect::<BTreeSet<_>>();
    let mut header_counts = BTreeMap::<&str, usize>::new();
    for header in headers {
        *header_counts.entry(header.as_str()).or_default() += 1;
    }
    let mut source_targets = BTreeMap::<&str, Vec<&str>>::new();
    let mut issues = Vec::new();

    for (target, source) in columns {
        if !allowed.contains(target.as_str()) {
            issues.push(ImportIssue {
                row: None,
                field: Some(target.clone()),
                severity: IssueSeverity::Error,
                code: "unknown_mapping_target".to_owned(),
                message: format!("不支持映射到字段 {target}"),
            });
            continue;
        }
        match header_counts
            .get(source.as_str())
            .copied()
            .unwrap_or_default()
        {
            0 => issues.push(ImportIssue {
                row: None,
                field: Some(target.clone()),
                severity: IssueSeverity::Error,
                code: "unknown_source_column".to_owned(),
                message: format!("找不到映射列 {source}"),
            }),
            1 => {}
            _ => issues.push(ImportIssue {
                row: None,
                field: Some(target.clone()),
                severity: IssueSeverity::Error,
                code: "duplicate_source_column".to_owned(),
                message: format!("源字段 {source} 在表头中重复，无法安全映射"),
            }),
        }
        source_targets
            .entry(source.as_str())
            .or_default()
            .push(target.as_str());
    }

    for (source, targets) in source_targets {
        if targets.len() > 1 {
            issues.push(ImportIssue {
                row: None,
                field: None,
                severity: IssueSeverity::Error,
                code: "duplicate_source_mapping".to_owned(),
                message: format!("源字段 {source} 不能同时映射到 {}", targets.join("、")),
            });
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_numeric_ids_remain_valid_strings() {
        let table =
            read_csv("小鼠ID,性别,出生日期\n101,M,2026-01-02\n102,雌性,2026/01/03\n".as_bytes())
                .unwrap();
        let preview = preview_animals(&table, &FieldMapping::infer(&table.headers));
        assert!(preview.can_confirm(), "{:?}", preview.issues);
        assert_eq!(preview.accepted_rows[0].display_id, "101");
        assert_eq!(preview.accepted_rows[1].sex.as_deref(), Some("female"));
    }

    #[test]
    fn missing_and_duplicate_ids_are_blocking() {
        let table = read_csv("id,sex\n7,M\n7,F\n,F\n".as_bytes()).unwrap();
        let preview = preview_animals(&table, &FieldMapping::infer(&table.headers));
        assert_eq!(preview.error_count(), 2);
        assert!(!preview.can_confirm());
        assert_eq!(preview.accepted_rows.len(), 1);
    }

    #[test]
    fn bad_sex_warns_but_does_not_block() {
        let table = read_csv("id,sex\nA1,?\n".as_bytes()).unwrap();
        let preview = preview_animals(&table, &FieldMapping::infer(&table.headers));
        assert!(preview.can_confirm());
        assert_eq!(preview.issues[0].severity, IssueSeverity::Warning);
    }

    #[test]
    fn explicit_mapping_repairs_an_unrecognized_required_header() {
        let table = read_csv("custom_code,gender_value\nA1,F\n".as_bytes()).unwrap();
        let inferred = preview_animals(&table, &FieldMapping::infer(&table.headers));
        assert!(!inferred.can_confirm());
        assert!(
            inferred
                .issues
                .iter()
                .any(|issue| issue.code == "missing_required_mapping")
        );

        let mapping = FieldMapping {
            columns: BTreeMap::from([
                ("display_id".to_owned(), "custom_code".to_owned()),
                ("sex".to_owned(), "gender_value".to_owned()),
            ]),
        };
        let repaired = preview_animals(&table, &mapping);
        assert!(repaired.can_confirm(), "{:?}", repaired.issues);
        assert_eq!(repaired.accepted_rows[0].display_id, "A1");
    }

    #[test]
    fn invalid_or_ambiguous_mapping_columns_are_rejected() {
        let table = read_csv("code,code\nA1,A2\n".as_bytes()).unwrap();
        let mapping = FieldMapping {
            columns: BTreeMap::from([
                ("display_id".to_owned(), "code".to_owned()),
                ("sex".to_owned(), "code".to_owned()),
                ("unsupported".to_owned(), "missing".to_owned()),
            ]),
        };
        let preview = preview_animals(&table, &mapping);
        let codes = preview
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("unknown_mapping_target"));
        assert!(codes.contains("duplicate_source_column"));
        assert!(codes.contains("duplicate_source_mapping"));
        assert!(!preview.can_confirm());
    }

    #[test]
    fn mapping_to_an_unknown_source_is_rejected() {
        let table = read_csv("code\nA1\n".as_bytes()).unwrap();
        let mapping = FieldMapping {
            columns: BTreeMap::from([("display_id".to_owned(), "missing".to_owned())]),
        };
        let preview = preview_animals(&table, &mapping);
        assert!(
            preview
                .issues
                .iter()
                .any(|issue| issue.code == "unknown_source_column")
        );
    }

    #[test]
    fn existing_display_id_is_reported_and_not_accepted() {
        let table = read_csv("id,sex\nA1,M\nA2,F\n".as_bytes()).unwrap();
        let directory = AnimalDirectory::from_entries([("A1", uuid::Uuid::nil())]).unwrap();
        let preview = preview_animals_with_directory(
            &table,
            &FieldMapping::infer(&table.headers),
            &directory,
        );
        assert_eq!(preview.accepted_rows.len(), 1);
        assert_eq!(preview.accepted_rows[0].display_id, "A2");
        assert!(
            preview
                .issues
                .iter()
                .any(|issue| issue.code == "existing_display_id")
        );
    }

    #[test]
    fn csv_writer_neutralizes_spreadsheet_formulas() {
        let row = AnimalImportRow {
            source_row: 2,
            display_id: "=HYPERLINK(\"https://invalid.example\")".to_owned(),
            sex: Some("male".to_owned()),
            birth_date: None,
            strain: Some("  +SUM(1,1)".to_owned()),
            cage: None,
            genotype: None,
            father: None,
            mother: None,
        };
        let mut output = Vec::new();
        write_animals_csv(&[row], &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("'=HYPERLINK"));
        assert!(text.contains("'  +SUM"));
    }

    #[test]
    fn cancelled_csv_parse_returns_no_partial_table() {
        let token = CancellationToken::default();
        token.cancel();
        assert!(matches!(
            read_csv_with_cancel("id\nA1\n".as_bytes(), &token),
            Err(ImportError::Cancelled)
        ));
    }
}
