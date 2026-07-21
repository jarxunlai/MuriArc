use std::{collections::BTreeSet, io::Write};

use chrono::NaiveDate;
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CancellationCheck, ImportError, NoCancellation, safe_csv_cell};

const EXPORT_HEADERS: [&str; 10] = [
    "animal_uuid",
    "display_id",
    "sex",
    "birth_date",
    "strain",
    "cage_display_id",
    "cage_section",
    "cage_location",
    "gene_loci",
    "alleles",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportSex {
    Male,
    Female,
    Unknown,
}

impl ExportSex {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportCage {
    pub display_id: String,
    pub section: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportGenotype {
    pub locus: String,
    pub allele: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalExportRecord {
    pub animal_id: Uuid,
    pub display_id: String,
    pub sex: ExportSex,
    pub birth_date: Option<NaiveDate>,
    pub strain: Option<String>,
    pub cage: Option<ExportCage>,
    pub genotypes: Vec<ExportGenotype>,
}

/// Empty dimensions do not restrict results. Values within one dimension are
/// OR-ed; populated dimensions are AND-ed with each other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalExportFilter {
    pub sexes: BTreeSet<ExportSex>,
    pub cage_locations: BTreeSet<String>,
    pub cage_sections: BTreeSet<String>,
    pub cage_display_ids: BTreeSet<String>,
    pub gene_loci: BTreeSet<String>,
    pub alleles: BTreeSet<String>,
}

pub fn filter_animals(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
) -> Vec<AnimalExportRecord> {
    filter_animals_with_cancel(records, filter, &NoCancellation)
        .expect("NoCancellation cannot cancel")
}

pub fn filter_animals_with_cancel<C>(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
    cancellation: &C,
) -> Result<Vec<AnimalExportRecord>, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    let mut filtered = Vec::new();
    for record in records {
        cancellation.check_cancelled()?;
        if matches_filter(record, filter) {
            filtered.push(record.clone());
        }
    }
    Ok(filtered)
}

pub fn export_animals_csv(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
    writer: impl Write,
) -> Result<(), ImportError> {
    export_animals_csv_with_cancel(records, filter, writer, &NoCancellation)
}

pub fn export_animals_csv_with_cancel<W, C>(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
    writer: W,
    cancellation: &C,
) -> Result<(), ImportError>
where
    W: Write,
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut csv = csv::Writer::from_writer(writer);
    csv.write_record(EXPORT_HEADERS)?;
    for record in records {
        cancellation.check_cancelled()?;
        if matches_filter(record, filter) {
            let values = render_record(record);
            csv.write_record(values.iter().map(|value| safe_csv_cell(value)))?;
        }
    }
    cancellation.check_cancelled()?;
    csv.flush()?;
    Ok(())
}

pub fn export_animals_xlsx(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
) -> Result<Vec<u8>, ImportError> {
    export_animals_xlsx_with_cancel(records, filter, &NoCancellation)
}

pub fn export_animals_xlsx_with_cancel<C>(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
    cancellation: &C,
) -> Result<Vec<u8>, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let mut workbook = Workbook::new();
    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("animals")?;
        for (column, header) in EXPORT_HEADERS.iter().enumerate() {
            worksheet.write_string(0, column as u16, *header)?;
        }
        let mut output_row = 1_u32;
        for record in records {
            cancellation.check_cancelled()?;
            if !matches_filter(record, filter) {
                continue;
            }
            for (column, value) in render_record(record).iter().enumerate() {
                worksheet.write_string(output_row, column as u16, value)?;
            }
            output_row = output_row.saturating_add(1);
        }
    }
    cancellation.check_cancelled()?;
    Ok(workbook.save_to_buffer()?)
}

fn matches_filter(record: &AnimalExportRecord, filter: &AnimalExportFilter) -> bool {
    if !filter.sexes.is_empty() && !filter.sexes.contains(&record.sex) {
        return false;
    }

    if !filter.cage_display_ids.is_empty()
        || !filter.cage_sections.is_empty()
        || !filter.cage_locations.is_empty()
    {
        let Some(cage) = record.cage.as_ref() else {
            return false;
        };
        if !filter.cage_display_ids.is_empty()
            && !filter
                .cage_display_ids
                .iter()
                .any(|value| value.trim() == cage.display_id.trim())
        {
            return false;
        }
        if !filter.cage_sections.is_empty()
            && !optional_matches(cage.section.as_deref(), &filter.cage_sections)
        {
            return false;
        }
        if !filter.cage_locations.is_empty()
            && !optional_matches(cage.location.as_deref(), &filter.cage_locations)
        {
            return false;
        }
    }

    if !filter.gene_loci.is_empty() || !filter.alleles.is_empty() {
        let genotype_matches = record.genotypes.iter().any(|genotype| {
            (filter.gene_loci.is_empty()
                || filter
                    .gene_loci
                    .iter()
                    .any(|value| text_matches(value, &genotype.locus)))
                && (filter.alleles.is_empty()
                    || filter
                        .alleles
                        .iter()
                        .any(|value| text_matches(value, &genotype.allele)))
        });
        if !genotype_matches {
            return false;
        }
    }
    true
}

fn optional_matches(actual: Option<&str>, expected: &BTreeSet<String>) -> bool {
    actual.is_some_and(|actual| expected.iter().any(|value| text_matches(value, actual)))
}

fn text_matches(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn render_record(record: &AnimalExportRecord) -> [String; EXPORT_HEADERS.len()] {
    let (cage_display_id, cage_section, cage_location) = record
        .cage
        .as_ref()
        .map(|cage| {
            (
                cage.display_id.clone(),
                cage.section.clone().unwrap_or_default(),
                cage.location.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    let gene_loci = record
        .genotypes
        .iter()
        .map(|genotype| genotype.locus.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let alleles = record
        .genotypes
        .iter()
        .map(|genotype| genotype.allele.as_str())
        .collect::<Vec<_>>()
        .join(";");
    [
        record.animal_id.to_string(),
        record.display_id.clone(),
        record.sex.as_str().to_owned(),
        record
            .birth_date
            .map(|date| date.to_string())
            .unwrap_or_default(),
        record.strain.clone().unwrap_or_default(),
        cage_display_id,
        cage_section,
        cage_location,
        gene_loci,
        alleles,
    ]
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{CancellationToken, read_xlsx};

    fn record(
        display_id: &str,
        sex: ExportSex,
        cage: &str,
        genotypes: &[(&str, &str)],
    ) -> AnimalExportRecord {
        AnimalExportRecord {
            animal_id: Uuid::new_v4(),
            display_id: display_id.to_owned(),
            sex,
            birth_date: NaiveDate::from_ymd_opt(2026, 1, 2),
            strain: Some("C57BL/6J".to_owned()),
            cage: Some(ExportCage {
                display_id: cage.to_owned(),
                section: Some("A".to_owned()),
                location: Some("Room 1".to_owned()),
            }),
            genotypes: genotypes
                .iter()
                .map(|(locus, allele)| ExportGenotype {
                    locus: (*locus).to_owned(),
                    allele: (*allele).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn dimensions_are_anded_and_values_are_ored() {
        let records = [
            record(
                "M1",
                ExportSex::Male,
                "C1",
                &[("GeneA", "fl"), ("Cre", "+")],
            ),
            record("M2", ExportSex::Female, "C2", &[("GeneA", "wt")]),
        ];
        let filter = AnimalExportFilter {
            sexes: BTreeSet::from([ExportSex::Male, ExportSex::Female]),
            cage_display_ids: BTreeSet::from(["C1".to_owned(), "C3".to_owned()]),
            gene_loci: BTreeSet::from(["genea".to_owned()]),
            alleles: BTreeSet::from(["fl".to_owned()]),
            ..Default::default()
        };
        let filtered = filter_animals(&records, &filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_id, "M1");
    }

    #[test]
    fn locus_and_allele_must_match_the_same_genotype() {
        let records = [record(
            "M1",
            ExportSex::Male,
            "C1",
            &[("LocusA", "allele-a"), ("LocusB", "allele-b")],
        )];
        let filter = AnimalExportFilter {
            gene_loci: BTreeSet::from(["LocusA".to_owned()]),
            alleles: BTreeSet::from(["allele-b".to_owned()]),
            ..Default::default()
        };
        assert!(filter_animals(&records, &filter).is_empty());
    }

    #[test]
    fn csv_and_xlsx_have_the_same_filtered_rows() {
        let records = [
            record("M1", ExportSex::Male, "C1", &[("GeneA", "fl")]),
            record("M2", ExportSex::Female, "C2", &[("GeneA", "wt")]),
        ];
        let filter = AnimalExportFilter {
            sexes: BTreeSet::from([ExportSex::Female]),
            ..Default::default()
        };
        let mut csv_bytes = Vec::new();
        export_animals_csv(&records, &filter, &mut csv_bytes).unwrap();
        let csv_text = String::from_utf8(csv_bytes).unwrap();
        assert!(!csv_text.contains("M1"));
        assert!(csv_text.contains("M2"));

        let xlsx_bytes = export_animals_xlsx(&records, &filter).unwrap();
        let table = read_xlsx(Cursor::new(xlsx_bytes)).unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0][1], "M2");
    }

    #[test]
    fn csv_export_neutralizes_spreadsheet_formulas_but_xlsx_keeps_strings() {
        let records = [record(
            "=HYPERLINK(\"https://invalid.example\")",
            ExportSex::Male,
            "+1",
            &[("@cmd", "-1")],
        )];
        let mut csv_bytes = Vec::new();
        export_animals_csv(&records, &AnimalExportFilter::default(), &mut csv_bytes).unwrap();
        let csv_text = String::from_utf8(csv_bytes).unwrap();
        assert!(csv_text.contains("'=HYPERLINK"));
        assert!(csv_text.contains("'+1"));
        assert!(csv_text.contains("'@cmd"));
        assert!(csv_text.contains("'-1"));

        let xlsx = read_xlsx(Cursor::new(
            export_animals_xlsx(&records, &AnimalExportFilter::default()).unwrap(),
        ))
        .unwrap();
        assert_eq!(xlsx.rows[0][1], "=HYPERLINK(\"https://invalid.example\")");
    }

    #[test]
    fn cancellation_stops_export() {
        let records = [record("M1", ExportSex::Male, "C1", &[])];
        let token = CancellationToken::default();
        token.cancel();
        assert!(matches!(
            export_animals_xlsx_with_cancel(&records, &AnimalExportFilter::default(), &token),
            Err(ImportError::Cancelled)
        ));
    }
}
