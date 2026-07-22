use std::{collections::BTreeSet, io::Write};

use chrono::{DateTime, NaiveDate, Utc};
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CancellationCheck, ImportError, NoCancellation, safe_csv_cell};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAnimalStatus {
    Planned,
    Alive,
    InExperiment,
    Sampled,
    Deceased,
    Euthanized,
    Lost,
    Archived,
}

impl ExportAnimalStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Alive => "alive",
            Self::InExperiment => "in_experiment",
            Self::Sampled => "sampled",
            Self::Deceased => "deceased",
            Self::Euthanized => "euthanized",
            Self::Lost => "lost",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportGenotypingState {
    Unknown,
    Expected,
    Confirmed,
    Rejected,
}

impl ExportGenotypingState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Expected => "expected",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
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
    pub definition: String,
    pub state: ExportGenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub locus: String,
    pub allele_1: String,
    pub allele_2: Option<String>,
    pub component_mode: String,
    pub display_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalExportRecord {
    /// Internal correlation only. Business renderers never emit this UUID.
    pub animal_id: Uuid,
    pub identifier_scope: String,
    pub project_name: Option<String>,
    pub display_id: String,
    pub sex: ExportSex,
    pub birth_date: Option<NaiveDate>,
    pub registered_at: DateTime<Utc>,
    pub strain: Option<String>,
    pub status: ExportAnimalStatus,
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
    pub strains: BTreeSet<String>,
    pub statuses: BTreeSet<ExportAnimalStatus>,
    pub genotype_definitions: BTreeSet<String>,
    pub genotyping_states: BTreeSet<ExportGenotypingState>,
    pub gene_loci: BTreeSet<String>,
    pub alleles: BTreeSet<String>,
    pub birth_date_from: Option<NaiveDate>,
    pub birth_date_to: Option<NaiveDate>,
    pub registered_at_from: Option<DateTime<Utc>>,
    pub registered_at_to: Option<DateTime<Utc>>,
    pub assessed_at_from: Option<DateTime<Utc>>,
    pub assessed_at_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimalExportField {
    IdentifierScope,
    ProjectName,
    DisplayId,
    Sex,
    BirthDate,
    RegisteredAt,
    Strain,
    Status,
    CageLocation,
    CageSection,
    CageDisplayId,
    CurrentGenotypeSummary,
}

impl AnimalExportField {
    const ALL: [Self; 12] = [
        Self::IdentifierScope,
        Self::ProjectName,
        Self::DisplayId,
        Self::Sex,
        Self::BirthDate,
        Self::RegisteredAt,
        Self::Strain,
        Self::Status,
        Self::CageLocation,
        Self::CageSection,
        Self::CageDisplayId,
        Self::CurrentGenotypeSummary,
    ];

    const fn header(self) -> &'static str {
        match self {
            Self::IdentifierScope => "identifier_scope",
            Self::ProjectName => "project_name",
            Self::DisplayId => "display_id",
            Self::Sex => "sex",
            Self::BirthDate => "birth_date",
            Self::RegisteredAt => "registered_at",
            Self::Strain => "strain",
            Self::Status => "status",
            Self::CageLocation => "cage_location",
            Self::CageSection => "cage_section",
            Self::CageDisplayId => "cage_display_id",
            Self::CurrentGenotypeSummary => "current_genotype_summary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalExportOptions {
    #[serde(default)]
    pub filter: AnimalExportFilter,
    #[serde(default = "default_export_fields")]
    pub fields: BTreeSet<AnimalExportField>,
    #[serde(default = "default_true")]
    pub include_genotype_details: bool,
}

impl Default for AnimalExportOptions {
    fn default() -> Self {
        Self {
            filter: AnimalExportFilter::default(),
            fields: default_export_fields(),
            include_genotype_details: true,
        }
    }
}

fn default_export_fields() -> BTreeSet<AnimalExportField> {
    AnimalExportField::ALL.into_iter().collect()
}

const fn default_true() -> bool {
    true
}

fn selected_fields(options: &AnimalExportOptions) -> Vec<AnimalExportField> {
    // Human-readable compound identity is mandatory because UUID is forbidden
    // in an ordinary business export.
    let mut fields = options.fields.clone();
    fields.extend([
        AnimalExportField::IdentifierScope,
        AnimalExportField::ProjectName,
        AnimalExportField::DisplayId,
    ]);
    AnimalExportField::ALL
        .into_iter()
        .filter(|field| fields.contains(field))
        .collect()
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
    export_animals_csv_with_options(
        records,
        &AnimalExportOptions {
            filter: filter.clone(),
            ..Default::default()
        },
        writer,
    )
}

pub fn export_animals_csv_with_options(
    records: &[AnimalExportRecord],
    options: &AnimalExportOptions,
    writer: impl Write,
) -> Result<(), ImportError> {
    export_animals_csv_with_options_and_cancel(records, options, writer, &NoCancellation)
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
    export_animals_csv_with_options_and_cancel(
        records,
        &AnimalExportOptions {
            filter: filter.clone(),
            ..Default::default()
        },
        writer,
        cancellation,
    )
}

fn export_animals_csv_with_options_and_cancel<W, C>(
    records: &[AnimalExportRecord],
    options: &AnimalExportOptions,
    writer: W,
    cancellation: &C,
) -> Result<(), ImportError>
where
    W: Write,
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let fields = selected_fields(options);
    let mut csv = csv::Writer::from_writer(writer);
    csv.write_record(fields.iter().map(|field| field.header()))?;
    for record in records {
        cancellation.check_cancelled()?;
        if matches_filter(record, &options.filter) {
            let values = render_animal(record, &fields);
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
    export_animals_xlsx_with_options(
        records,
        &AnimalExportOptions {
            filter: filter.clone(),
            ..Default::default()
        },
    )
}

pub fn export_animals_xlsx_with_options(
    records: &[AnimalExportRecord],
    options: &AnimalExportOptions,
) -> Result<Vec<u8>, ImportError> {
    export_animals_xlsx_with_options_and_cancel(records, options, &NoCancellation)
}

pub fn export_animals_xlsx_with_cancel<C>(
    records: &[AnimalExportRecord],
    filter: &AnimalExportFilter,
    cancellation: &C,
) -> Result<Vec<u8>, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    export_animals_xlsx_with_options_and_cancel(
        records,
        &AnimalExportOptions {
            filter: filter.clone(),
            ..Default::default()
        },
        cancellation,
    )
}

fn export_animals_xlsx_with_options_and_cancel<C>(
    records: &[AnimalExportRecord],
    options: &AnimalExportOptions,
    cancellation: &C,
) -> Result<Vec<u8>, ImportError>
where
    C: CancellationCheck + ?Sized,
{
    cancellation.check_cancelled()?;
    let selected = records
        .iter()
        .filter(|record| matches_filter(record, &options.filter))
        .collect::<Vec<_>>();
    let fields = selected_fields(options);
    let mut workbook = Workbook::new();
    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("animals")?;
        for (column, field) in fields.iter().enumerate() {
            worksheet.write_string(0, column as u16, field.header())?;
        }
        for (row, record) in selected.iter().enumerate() {
            cancellation.check_cancelled()?;
            for (column, value) in render_animal(record, &fields).iter().enumerate() {
                worksheet.write_string((row + 1) as u32, column as u16, value)?;
            }
        }
    }
    if options.include_genotype_details {
        let headers = [
            "identifier_scope",
            "project_name",
            "display_id",
            "genotype_definition",
            "state",
            "assessed_at",
            "method",
            "locus",
            "allele_1",
            "allele_2",
            "component_mode",
        ];
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("genotypes")?;
        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string(0, column as u16, *header)?;
        }
        let mut row = 1_u32;
        for record in &selected {
            for genotype in &record.genotypes {
                cancellation.check_cancelled()?;
                let values = [
                    record.identifier_scope.clone(),
                    record.project_name.clone().unwrap_or_default(),
                    record.display_id.clone(),
                    genotype.definition.clone(),
                    genotype.state.as_str().to_owned(),
                    genotype
                        .assessed_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default(),
                    genotype.method.clone().unwrap_or_default(),
                    genotype.locus.clone(),
                    genotype.allele_1.clone(),
                    genotype.allele_2.clone().unwrap_or_default(),
                    genotype.component_mode.clone(),
                ];
                for (column, value) in values.iter().enumerate() {
                    worksheet.write_string(row, column as u16, value)?;
                }
                row = row.saturating_add(1);
            }
        }
    }
    cancellation.check_cancelled()?;
    Ok(workbook.save_to_buffer()?)
}

fn matches_filter(record: &AnimalExportRecord, filter: &AnimalExportFilter) -> bool {
    if !filter.sexes.is_empty() && !filter.sexes.contains(&record.sex) {
        return false;
    }
    if !filter.statuses.is_empty() && !filter.statuses.contains(&record.status) {
        return false;
    }
    if !filter.strains.is_empty()
        && !record.strain.as_deref().is_some_and(|strain| {
            filter
                .strains
                .iter()
                .any(|value| text_matches(value, strain))
        })
    {
        return false;
    }
    if !date_in_range(
        record.birth_date,
        filter.birth_date_from,
        filter.birth_date_to,
    ) {
        return false;
    }
    if filter
        .registered_at_from
        .is_some_and(|from| record.registered_at < from)
        || filter
            .registered_at_to
            .is_some_and(|to| record.registered_at > to)
    {
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
                .any(|value| text_matches(value, &cage.display_id))
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
    let has_genotype_filter = !filter.genotype_definitions.is_empty()
        || !filter.genotyping_states.is_empty()
        || !filter.gene_loci.is_empty()
        || !filter.alleles.is_empty()
        || filter.assessed_at_from.is_some()
        || filter.assessed_at_to.is_some();
    if has_genotype_filter
        && !record.genotypes.iter().any(|genotype| {
            (filter.genotype_definitions.is_empty()
                || filter
                    .genotype_definitions
                    .iter()
                    .any(|value| text_matches(value, &genotype.definition)))
                && (filter.genotyping_states.is_empty()
                    || filter.genotyping_states.contains(&genotype.state))
                && (filter.gene_loci.is_empty()
                    || filter
                        .gene_loci
                        .iter()
                        .any(|value| text_matches(value, &genotype.locus)))
                && (filter.alleles.is_empty()
                    || filter.alleles.iter().any(|value| {
                        text_matches(value, &genotype.allele_1)
                            || genotype
                                .allele_2
                                .as_deref()
                                .is_some_and(|allele| text_matches(value, allele))
                    }))
                && filter
                    .assessed_at_from
                    .is_none_or(|from| genotype.assessed_at.is_some_and(|at| at >= from))
                && filter
                    .assessed_at_to
                    .is_none_or(|to| genotype.assessed_at.is_some_and(|at| at <= to))
        })
    {
        return false;
    }
    true
}

fn date_in_range<T: PartialOrd>(value: Option<T>, from: Option<T>, to: Option<T>) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    value.is_some_and(|value| {
        !from.is_some_and(|from| value < from) && !to.is_some_and(|to| value > to)
    })
}

fn optional_matches(actual: Option<&str>, expected: &BTreeSet<String>) -> bool {
    actual.is_some_and(|actual| expected.iter().any(|value| text_matches(value, actual)))
}

fn text_matches(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn genotype_summary(record: &AnimalExportRecord) -> String {
    let mut definitions = Vec::<String>::new();
    for genotype in &record.genotypes {
        let component = format!(
            "{} {}/{}",
            genotype.locus,
            genotype.allele_1,
            genotype.allele_2.as_deref().unwrap_or("—")
        );
        if let Some(existing) = definitions.iter_mut().find(|value| {
            value.starts_with(&format!(
                "{} [{}]:",
                genotype.definition,
                genotype.state.as_str()
            ))
        }) {
            existing.push_str(" & ");
            existing.push_str(&component);
        } else {
            definitions.push(format!(
                "{} [{}]: {}",
                genotype.definition,
                genotype.state.as_str(),
                component
            ));
        }
    }
    definitions.join(" | ")
}

fn render_animal(record: &AnimalExportRecord, fields: &[AnimalExportField]) -> Vec<String> {
    fields
        .iter()
        .map(|field| match field {
            AnimalExportField::IdentifierScope => record.identifier_scope.clone(),
            AnimalExportField::ProjectName => record.project_name.clone().unwrap_or_default(),
            AnimalExportField::DisplayId => record.display_id.clone(),
            AnimalExportField::Sex => record.sex.as_str().to_owned(),
            AnimalExportField::BirthDate => record
                .birth_date
                .map(|date| date.to_string())
                .unwrap_or_default(),
            AnimalExportField::RegisteredAt => record.registered_at.to_rfc3339(),
            AnimalExportField::Strain => record.strain.clone().unwrap_or_default(),
            AnimalExportField::Status => record.status.as_str().to_owned(),
            AnimalExportField::CageLocation => record
                .cage
                .as_ref()
                .and_then(|cage| cage.location.clone())
                .unwrap_or_default(),
            AnimalExportField::CageSection => record
                .cage
                .as_ref()
                .and_then(|cage| cage.section.clone())
                .unwrap_or_default(),
            AnimalExportField::CageDisplayId => record
                .cage
                .as_ref()
                .map(|cage| cage.display_id.clone())
                .unwrap_or_default(),
            AnimalExportField::CurrentGenotypeSummary => genotype_summary(record),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use calamine::Reader;

    use super::*;
    use crate::{CancellationToken, read_xlsx};

    fn record(
        display_id: &str,
        sex: ExportSex,
        state: ExportGenotypingState,
    ) -> AnimalExportRecord {
        AnimalExportRecord {
            animal_id: Uuid::new_v4(),
            identifier_scope: "lab".to_owned(),
            project_name: None,
            display_id: display_id.to_owned(),
            sex,
            birth_date: NaiveDate::from_ymd_opt(2026, 1, 2),
            registered_at: DateTime::parse_from_rfc3339("2026-01-03T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            strain: Some("C57BL/6J".to_owned()),
            status: ExportAnimalStatus::Alive,
            cage: Some(ExportCage {
                display_id: "C1".to_owned(),
                section: Some("A".to_owned()),
                location: Some("Room 1".to_owned()),
            }),
            genotypes: vec![ExportGenotype {
                definition: "Three loci".to_owned(),
                state,
                assessed_at: Some(
                    DateTime::parse_from_rfc3339("2026-01-04T00:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                method: Some("PCR".to_owned()),
                locus: "GeneA".to_owned(),
                allele_1: "+".to_owned(),
                allele_2: Some("flox".to_owned()),
                component_mode: "diploid".to_owned(),
                display_order: 0,
            }],
        }
    }

    #[test]
    fn business_csv_never_contains_uuid_and_keeps_human_identity() {
        let row = record("M1", ExportSex::Male, ExportGenotypingState::Expected);
        let mut bytes = Vec::new();
        export_animals_csv(&[row.clone()], &AnimalExportFilter::default(), &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("animal_uuid"));
        assert!(!text.contains(&row.animal_id.to_string()));
        assert!(text.starts_with("identifier_scope,project_name,display_id"));
        assert!(text.contains("Three loci [expected]: GeneA +/flox"));
    }

    #[test]
    fn filters_are_anded_and_genotype_dimensions_match_one_component() {
        let records = [
            record("M1", ExportSex::Male, ExportGenotypingState::Confirmed),
            record("M2", ExportSex::Female, ExportGenotypingState::Expected),
        ];
        let filter = AnimalExportFilter {
            sexes: BTreeSet::from([ExportSex::Male, ExportSex::Female]),
            strains: BTreeSet::from(["c57bl/6j".to_owned()]),
            genotype_definitions: BTreeSet::from(["Three loci".to_owned()]),
            genotyping_states: BTreeSet::from([ExportGenotypingState::Confirmed]),
            gene_loci: BTreeSet::from(["genea".to_owned()]),
            alleles: BTreeSet::from(["flox".to_owned()]),
            ..Default::default()
        };
        assert_eq!(filter_animals(&records, &filter)[0].display_id, "M1");
    }

    #[test]
    fn xlsx_has_separate_animals_and_genotypes_sheets() {
        let bytes = export_animals_xlsx(
            &[record(
                "M1",
                ExportSex::Male,
                ExportGenotypingState::Confirmed,
            )],
            &AnimalExportFilter::default(),
        )
        .unwrap();
        let mut workbook = calamine::Xlsx::new(Cursor::new(bytes)).unwrap();
        assert_eq!(workbook.sheet_names(), &["animals", "genotypes"]);
        let genotypes = workbook.worksheet_range("genotypes").unwrap();
        assert_eq!(genotypes.height(), 2);
        assert_eq!(genotypes.width(), 11);
    }

    #[test]
    fn cancellation_is_observed() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let mut bytes = Vec::new();
        assert!(matches!(
            export_animals_csv_with_cancel(
                &[record(
                    "M1",
                    ExportSex::Male,
                    ExportGenotypingState::Confirmed
                )],
                &AnimalExportFilter::default(),
                &mut bytes,
                &cancellation,
            ),
            Err(ImportError::Cancelled)
        ));
    }

    #[test]
    fn selected_fields_apply_to_csv_and_xlsx_animals_sheet() {
        let options = AnimalExportOptions {
            fields: BTreeSet::from([AnimalExportField::Sex]),
            include_genotype_details: false,
            ..Default::default()
        };
        let row = record("M1", ExportSex::Male, ExportGenotypingState::Confirmed);
        let mut csv = Vec::new();
        export_animals_csv_with_options(&[row.clone()], &options, &mut csv).unwrap();
        assert!(
            String::from_utf8(csv)
                .unwrap()
                .starts_with("identifier_scope,project_name,display_id,sex")
        );
        let xlsx = export_animals_xlsx_with_options(&[row], &options).unwrap();
        let table = read_xlsx(Cursor::new(xlsx)).unwrap();
        assert_eq!(
            table.headers,
            ["identifier_scope", "project_name", "display_id", "sex"]
        );
    }
}
