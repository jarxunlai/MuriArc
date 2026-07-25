use std::collections::{BTreeMap, BTreeSet};

use muriarc_core::GenotypingState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    AnimalDirectory, AnimalResolution, ImportIssue, IssueSeverity, TabularData, normalize_header,
    row_issue, validate_mapping_columns,
};

const REQUIRED_COLUMNS: [&str; 2] = ["display_id", "state"];
const MAPPING_TARGETS: [&str; 3] = ["display_id", "state", "notes"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingFieldMapping {
    /// Canonical genotyping field -> source header.
    pub columns: BTreeMap<String, String>,
}

impl GenotypingFieldMapping {
    pub fn infer(headers: &[String]) -> Self {
        let aliases: BTreeMap<&str, &[&str]> = BTreeMap::from([
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
                "state",
                &[
                    "state",
                    "status",
                    "result",
                    "genotyping_state",
                    "鉴定状态",
                    "鉴定结果",
                    "结果",
                ] as &[_],
            ),
            (
                "notes",
                &["notes", "note", "remark", "remarks", "备注", "说明"] as &[_],
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingImportRow {
    pub source_row: usize,
    pub animal_id: Uuid,
    pub display_id: String,
    pub state: GenotypingState,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenotypingImportPreview {
    pub total_rows: usize,
    pub accepted_rows: Vec<GenotypingImportRow>,
    pub issues: Vec<ImportIssue>,
    pub preview_hash: String,
}

impl GenotypingImportPreview {
    pub fn can_confirm(&self) -> bool {
        !self.accepted_rows.is_empty()
            && !self
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

pub fn preview_genotyping(
    table: &TabularData,
    mapping: &GenotypingFieldMapping,
    animals: &AnimalDirectory,
) -> GenotypingImportPreview {
    let mut issues = validate_mapping_columns(&table.headers, &mapping.columns, &MAPPING_TARGETS);
    for field in REQUIRED_COLUMNS {
        if !mapping.columns.contains_key(field) {
            issues.push(ImportIssue {
                row: None,
                field: Some(field.to_owned()),
                severity: IssueSeverity::Error,
                code: "missing_required_mapping".to_owned(),
                message: format!("必须映射基因鉴定字段 {field}"),
            });
        }
    }
    if !issues.is_empty() {
        return build_preview(table.rows.len(), Vec::new(), issues);
    }

    let header_index: BTreeMap<&str, usize> = table
        .headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header.as_str(), index))
        .collect();
    let mut accepted_rows = Vec::new();
    let mut seen_animals = BTreeSet::new();
    for (offset, raw) in table.rows.iter().enumerate() {
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

        let Some(display_id) = value("display_id") else {
            issues.push(row_issue(
                source_row,
                "display_id",
                IssueSeverity::Error,
                "missing_animal_identity",
                "动物显示编号不能为空",
            ));
            continue;
        };
        let animal_id = match animals.resolve(&display_id) {
            AnimalResolution::Unique(id) => id,
            AnimalResolution::Unknown => {
                issues.push(row_issue(
                    source_row,
                    "display_id",
                    IssueSeverity::Error,
                    "unknown_animal",
                    "动物编号在当前可用目录中不存在",
                ));
                continue;
            }
            AnimalResolution::Ambiguous => {
                issues.push(row_issue(
                    source_row,
                    "display_id",
                    IssueSeverity::Error,
                    "ambiguous_animal",
                    "动物编号对应多个动物，无法用于批次鉴定",
                ));
                continue;
            }
        };
        if !seen_animals.insert(animal_id) {
            issues.push(row_issue(
                source_row,
                "display_id",
                IssueSeverity::Error,
                "duplicate_animal",
                "同一鉴定批次中不能重复出现同一动物",
            ));
            continue;
        }

        let Some(raw_state) = value("state") else {
            issues.push(row_issue(
                source_row,
                "state",
                IssueSeverity::Error,
                "missing_state",
                "鉴定状态不能为空",
            ));
            continue;
        };
        let Some(state) = parse_state(&raw_state) else {
            issues.push(row_issue(
                source_row,
                "state",
                IssueSeverity::Error,
                "invalid_state",
                "鉴定状态必须为 unknown、expected、confirmed 或 rejected",
            ));
            continue;
        };
        accepted_rows.push(GenotypingImportRow {
            source_row,
            animal_id,
            display_id,
            state,
            notes: value("notes"),
        });
    }
    if accepted_rows.is_empty()
        && !issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error)
    {
        issues.push(ImportIssue {
            row: None,
            field: None,
            severity: IssueSeverity::Error,
            code: "empty_genotyping_batch".to_owned(),
            message: "鉴定结果表中没有可确认的数据行".to_owned(),
        });
    }
    build_preview(table.rows.len(), accepted_rows, issues)
}

pub fn genotyping_template_csv() -> Vec<u8> {
    b"display_id,state,notes\nMOUSE-001,confirmed,first gel lane\nMOUSE-002,rejected,repeat recommended\n"
        .to_vec()
}

fn build_preview(
    total_rows: usize,
    accepted_rows: Vec<GenotypingImportRow>,
    issues: Vec<ImportIssue>,
) -> GenotypingImportPreview {
    let preview_hash = hash_rows(&accepted_rows);
    GenotypingImportPreview {
        total_rows,
        accepted_rows,
        issues,
        preview_hash,
    }
}

fn hash_rows(rows: &[GenotypingImportRow]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"muriarc.genotyping-preview.v1");
    digest.update((rows.len() as u64).to_be_bytes());
    for row in rows {
        digest.update((row.source_row as u64).to_be_bytes());
        digest.update(row.animal_id.as_bytes());
        hash_text(&mut digest, &row.display_id);
        hash_text(&mut digest, state_label(row.state));
        match row.notes.as_deref() {
            Some(notes) => {
                digest.update([1]);
                hash_text(&mut digest, notes);
            }
            None => digest.update([0]),
        }
    }
    format!("{:x}", digest.finalize())
}

fn hash_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

fn parse_state(value: &str) -> Option<GenotypingState> {
    match value.trim().to_ascii_lowercase().as_str() {
        "unknown" | "未知" | "待定" => Some(GenotypingState::Unknown),
        "expected" | "预期" | "符合预期" => Some(GenotypingState::Expected),
        "confirmed" | "确认" | "阳性" | "已确认" => Some(GenotypingState::Confirmed),
        "rejected" | "排除" | "阴性" | "不符合" => Some(GenotypingState::Rejected),
        _ => None,
    }
}

fn state_label(state: GenotypingState) -> &'static str {
    match state {
        GenotypingState::Unknown => "unknown",
        GenotypingState::Expected => "expected",
        GenotypingState::Confirmed => "confirmed",
        GenotypingState::Rejected => "rejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_csv;

    #[test]
    fn previews_batch_and_rejects_duplicate_animals() {
        let animal_id = Uuid::new_v4();
        let directory = AnimalDirectory::from_entries([("MOUSE-001", animal_id)]).unwrap();
        let table = read_csv(
            b"display_id,state,notes\nMOUSE-001,confirmed,lane 1\nMOUSE-001,rejected,lane 2\n"
                .as_slice(),
        )
        .unwrap();
        let preview = preview_genotyping(
            &table,
            &GenotypingFieldMapping::infer(&table.headers),
            &directory,
        );
        assert_eq!(preview.accepted_rows.len(), 1);
        assert_eq!(preview.error_count(), 1);
        assert!(!preview.can_confirm());
        assert_eq!(preview.preview_hash.len(), 64);
    }

    #[test]
    fn preview_hash_changes_with_result_state() {
        let animal_id = Uuid::new_v4();
        let directory = AnimalDirectory::from_entries([("MOUSE-001", animal_id)]).unwrap();
        let preview = |state: &str| {
            let csv = format!("display_id,state\nMOUSE-001,{state}\n");
            let table = read_csv(csv.as_bytes()).unwrap();
            preview_genotyping(
                &table,
                &GenotypingFieldMapping::infer(&table.headers),
                &directory,
            )
        };
        assert_ne!(
            preview("confirmed").preview_hash,
            preview("rejected").preview_hash
        );
    }
}
