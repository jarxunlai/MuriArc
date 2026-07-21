use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use chrono::{DateTime, NaiveTime, Utc};
use muriarc_core::{
    Animal, AnimalEvent, AnimalEventKind, GenotypingRecord, GenotypingState, ImportPlan,
    Measurement, MeasurementValue, ParentType, Pedigree, RecordMeta, Sex,
};
use sha2::{Digest, Sha256};
use thiserror::Error as ThisError;
use uuid::Uuid;

use crate::{
    AnimalDirectory, AnimalImportRow, AnimalResolution, ImportIssue, ImportPreview, IssueSeverity,
    MeasurementImportPreview, MeasurementImportRow, MeasurementImportValue, MeasurementValueType,
};

/// Resolution result for a UUID-backed import directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryResolution {
    Unknown,
    Unique(Uuid),
    Ambiguous,
}

/// Lab-scoped cage lookup used while converting a confirmed animal preview.
///
/// An unqualified display identifier is accepted only when it resolves to one
/// cage across the entire directory. A caller can disambiguate it with either
/// `section/display_id` or `section::display_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CageDirectory {
    by_qualified_id: BTreeMap<(String, String), BTreeSet<Uuid>>,
    by_display_id: BTreeMap<String, BTreeSet<Uuid>>,
}

impl CageDirectory {
    pub fn from_entries<Section, DisplayId>(
        entries: impl IntoIterator<Item = (Section, DisplayId, Uuid)>,
    ) -> Result<Self, DirectoryError>
    where
        Section: Into<String>,
        DisplayId: Into<String>,
    {
        let mut directory = Self::default();
        for (section, display_id, cage_id) in entries {
            directory.insert(section, display_id, cage_id)?;
        }
        Ok(directory)
    }

    pub fn insert(
        &mut self,
        section: impl Into<String>,
        display_id: impl Into<String>,
        cage_id: Uuid,
    ) -> Result<(), DirectoryError> {
        let section = section.into().trim().to_owned();
        let display_id = display_id.into().trim().to_owned();
        if section.is_empty() {
            return Err(DirectoryError::EmptyCageSection);
        }
        if display_id.is_empty() {
            return Err(DirectoryError::EmptyCageDisplayId);
        }
        if cage_id.is_nil() {
            return Err(DirectoryError::NilIdentifier);
        }
        self.by_qualified_id
            .entry((section, display_id.clone()))
            .or_default()
            .insert(cage_id);
        self.by_display_id
            .entry(display_id)
            .or_default()
            .insert(cage_id);
        Ok(())
    }

    pub fn resolve(&self, reference: &str) -> DirectoryResolution {
        let reference = reference.trim();
        if let Some((section, display_id)) = parse_qualified_cage(reference) {
            return resolution(
                self.by_qualified_id
                    .get(&(section.to_owned(), display_id.to_owned())),
            );
        }
        resolution(self.by_display_id.get(reference))
    }

    pub fn resolve_in_section(&self, section: &str, display_id: &str) -> DirectoryResolution {
        resolution(
            self.by_qualified_id
                .get(&(section.trim().to_owned(), display_id.trim().to_owned())),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.by_qualified_id.is_empty()
    }
}

/// Lab-scoped gene locus and allele lookup for strict genotype conversion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneticDirectory {
    loci_by_symbol: BTreeMap<String, BTreeSet<Uuid>>,
    locus_ids: BTreeSet<Uuid>,
    alleles_by_locus_and_symbol: BTreeMap<(Uuid, String), BTreeSet<Uuid>>,
    allele_loci: BTreeMap<Uuid, Uuid>,
    definitions_by_components: BTreeMap<Vec<(Uuid, Uuid, Uuid)>, BTreeSet<Uuid>>,
}

impl GeneticDirectory {
    pub fn from_entries<LocusSymbol, AlleleSymbol>(
        loci: impl IntoIterator<Item = (LocusSymbol, Uuid)>,
        alleles: impl IntoIterator<Item = (Uuid, AlleleSymbol, Uuid)>,
    ) -> Result<Self, DirectoryError>
    where
        LocusSymbol: Into<String>,
        AlleleSymbol: Into<String>,
    {
        let mut directory = Self::default();
        for (symbol, locus_id) in loci {
            directory.insert_locus(symbol, locus_id)?;
        }
        for (locus_id, symbol, allele_id) in alleles {
            directory.insert_allele(locus_id, symbol, allele_id)?;
        }
        Ok(directory)
    }

    pub fn from_entries_with_definitions<LocusSymbol, AlleleSymbol>(
        loci: impl IntoIterator<Item = (LocusSymbol, Uuid)>,
        alleles: impl IntoIterator<Item = (Uuid, AlleleSymbol, Uuid)>,
        definitions: impl IntoIterator<Item = (Uuid, Vec<(Uuid, Uuid, Uuid)>)>,
    ) -> Result<Self, DirectoryError>
    where
        LocusSymbol: Into<String>,
        AlleleSymbol: Into<String>,
    {
        let mut directory = Self::from_entries(loci, alleles)?;
        for (definition_id, components) in definitions {
            directory.insert_definition(definition_id, components)?;
        }
        Ok(directory)
    }

    pub fn insert_locus(
        &mut self,
        symbol: impl Into<String>,
        locus_id: Uuid,
    ) -> Result<(), DirectoryError> {
        let symbol = symbol.into().trim().to_owned();
        if symbol.is_empty() {
            return Err(DirectoryError::EmptyLocusSymbol);
        }
        if locus_id.is_nil() {
            return Err(DirectoryError::NilIdentifier);
        }
        self.loci_by_symbol
            .entry(symbol)
            .or_default()
            .insert(locus_id);
        self.locus_ids.insert(locus_id);
        Ok(())
    }

    pub fn insert_allele(
        &mut self,
        locus_id: Uuid,
        symbol: impl Into<String>,
        allele_id: Uuid,
    ) -> Result<(), DirectoryError> {
        let symbol = symbol.into().trim().to_owned();
        if symbol.is_empty() {
            return Err(DirectoryError::EmptyAlleleSymbol);
        }
        if locus_id.is_nil() || allele_id.is_nil() {
            return Err(DirectoryError::NilIdentifier);
        }
        if !self.locus_ids.contains(&locus_id) {
            return Err(DirectoryError::UnknownAlleleLocus(locus_id));
        }
        if let Some(existing_locus_id) = self.allele_loci.insert(allele_id, locus_id)
            && existing_locus_id != locus_id
        {
            return Err(DirectoryError::AlleleBelongsToMultipleLoci {
                allele_id,
                first_locus_id: existing_locus_id,
                second_locus_id: locus_id,
            });
        }
        self.alleles_by_locus_and_symbol
            .entry((locus_id, symbol))
            .or_default()
            .insert(allele_id);
        Ok(())
    }

    pub fn resolve_locus(&self, symbol: &str) -> DirectoryResolution {
        resolution(self.loci_by_symbol.get(symbol.trim()))
    }

    pub fn resolve_allele(&self, locus_id: Uuid, symbol: &str) -> DirectoryResolution {
        resolution(
            self.alleles_by_locus_and_symbol
                .get(&(locus_id, symbol.trim().to_owned())),
        )
    }

    pub fn insert_definition(
        &mut self,
        definition_id: Uuid,
        components: Vec<(Uuid, Uuid, Uuid)>,
    ) -> Result<(), DirectoryError> {
        if definition_id.is_nil()
            || components
                .iter()
                .any(|(locus_id, allele_1_id, allele_2_id)| {
                    locus_id.is_nil() || allele_1_id.is_nil() || allele_2_id.is_nil()
                })
        {
            return Err(DirectoryError::NilIdentifier);
        }
        if components.is_empty() {
            return Err(DirectoryError::EmptyGenotypeDefinition);
        }
        let mut signature = Vec::with_capacity(components.len());
        let mut loci = BTreeSet::new();
        for (locus_id, allele_1_id, allele_2_id) in components {
            if !self.locus_ids.contains(&locus_id)
                || self.allele_loci.get(&allele_1_id) != Some(&locus_id)
                || self.allele_loci.get(&allele_2_id) != Some(&locus_id)
            {
                return Err(DirectoryError::InvalidDefinitionComponent);
            }
            if !loci.insert(locus_id) {
                return Err(DirectoryError::DuplicateDefinitionLocus(locus_id));
            }
            let (first, second) = if allele_1_id <= allele_2_id {
                (allele_1_id, allele_2_id)
            } else {
                (allele_2_id, allele_1_id)
            };
            signature.push((locus_id, first, second));
        }
        signature.sort_unstable();
        self.definitions_by_components
            .entry(signature)
            .or_default()
            .insert(definition_id);
        Ok(())
    }

    pub fn resolve_definition(&self, components: &[(Uuid, Uuid, Uuid)]) -> DirectoryResolution {
        let mut signature = components
            .iter()
            .map(|(locus_id, allele_1_id, allele_2_id)| {
                let (first, second) = if allele_1_id <= allele_2_id {
                    (*allele_1_id, *allele_2_id)
                } else {
                    (*allele_2_id, *allele_1_id)
                };
                (*locus_id, first, second)
            })
            .collect::<Vec<_>>();
        signature.sort_unstable();
        resolution(self.definitions_by_components.get(&signature))
    }

    pub fn is_empty(&self) -> bool {
        self.locus_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum DirectoryError {
    #[error("directory identifiers must not be nil UUIDs")]
    NilIdentifier,
    #[error("cage section must not be empty")]
    EmptyCageSection,
    #[error("cage display identifier must not be empty")]
    EmptyCageDisplayId,
    #[error("gene locus symbol must not be empty")]
    EmptyLocusSymbol,
    #[error("allele symbol must not be empty")]
    EmptyAlleleSymbol,
    #[error("allele references a locus that is not present in this directory: {0}")]
    UnknownAlleleLocus(Uuid),
    #[error("genotype definition must contain at least one import-compatible component")]
    EmptyGenotypeDefinition,
    #[error("genotype definition contains a locus or allele outside the active directory")]
    InvalidDefinitionComponent,
    #[error("genotype definition contains locus {0} more than once")]
    DuplicateDefinitionLocus(Uuid),
    #[error(
        "allele {allele_id} belongs to both locus {first_locus_id} and locus {second_locus_id}"
    )]
    AlleleBelongsToMultipleLoci {
        allele_id: Uuid,
        first_locus_id: Uuid,
        second_locus_id: Uuid,
    },
}

/// Shared immutable metadata for a preview-confirmed import plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlanContext {
    pub lab_id: Uuid,
    pub actor_user_id: Uuid,
    pub idempotency_key: String,
    pub preview_hash: String,
    pub confirmed_at: DateTime<Utc>,
}

impl ImportPlanContext {
    pub fn new(
        lab_id: Uuid,
        actor_user_id: Uuid,
        idempotency_key: impl Into<String>,
        preview_hash: impl Into<String>,
        confirmed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            lab_id,
            actor_user_id,
            idempotency_key: idempotency_key.into(),
            preview_hash: preview_hash.into(),
            confirmed_at,
        }
    }
}

/// Project and experiment scope required for measurement imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementImportPlanContext {
    pub import: ImportPlanContext,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    /// Measurement key -> human-readable label. No key fallback is allowed.
    pub measurement_labels: BTreeMap<String, String>,
}

impl MeasurementImportPlanContext {
    pub fn new<Key, Label>(
        import: ImportPlanContext,
        project_id: Uuid,
        experiment_id: Uuid,
        measurement_labels: impl IntoIterator<Item = (Key, Label)>,
    ) -> Self
    where
        Key: Into<String>,
        Label: Into<String>,
    {
        Self {
            import,
            project_id,
            experiment_id,
            measurement_labels: measurement_labels
                .into_iter()
                .map(|(key, label)| (key.into(), label.into()))
                .collect(),
        }
    }
}

/// A blocking, row-aware explanation of why a preview could not become a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlanBuildError {
    issues: Vec<ImportIssue>,
}

impl ImportPlanBuildError {
    fn new(issues: Vec<ImportIssue>) -> Self {
        debug_assert!(!issues.is_empty());
        Self { issues }
    }

    pub fn issues(&self) -> &[ImportIssue] {
        &self.issues
    }

    pub fn into_issues(self) -> Vec<ImportIssue> {
        self.issues
    }
}

impl fmt::Display for ImportPlanBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "import plan contains {} blocking issue(s)",
            self.issues.len()
        )
    }
}

impl Error for ImportPlanBuildError {}

/// Converts a confirmed animal preview into one fully UUID-resolved plan.
///
/// UUIDs are derived from the preview hash and row identity, so rebuilding the
/// same confirmed preview yields the same entity graph. All animals are
/// assigned before parent references are resolved, allowing same-batch parents.
pub fn build_animal_import_plan(
    preview: &ImportPreview,
    context: &ImportPlanContext,
    existing_animals: &AnimalDirectory,
    cages: &CageDirectory,
    genetics: &GeneticDirectory,
) -> Result<ImportPlan, ImportPlanBuildError> {
    let mut issues = blocking_preview_issues(&preview.issues);
    validate_import_context(context, &mut issues);
    if preview.accepted_rows.is_empty() {
        issues.push(global_issue(
            "animals",
            "empty_import",
            "animal import has no accepted rows",
        ));
    }

    let mut display_ids = BTreeSet::new();
    let mut source_rows = BTreeSet::new();
    for row in &preview.accepted_rows {
        if row.display_id.trim().is_empty() {
            issues.push(plan_row_issue(
                row.source_row,
                "display_id",
                "missing_display_id",
                "animal display identifier must not be empty",
            ));
        } else if !display_ids.insert(row.display_id.trim().to_owned()) {
            issues.push(plan_row_issue(
                row.source_row,
                "display_id",
                "duplicate_in_file",
                "animal display identifier is duplicated in this preview",
            ));
        }
        if !source_rows.insert(row.source_row) {
            issues.push(plan_row_issue(
                row.source_row,
                "source_row",
                "duplicate_source_row",
                "preview contains a duplicate source row",
            ));
        }
        if existing_animals.contains(&row.display_id) {
            issues.push(plan_row_issue(
                row.source_row,
                "display_id",
                "existing_display_id",
                "animal display identifier already exists in the lab registry",
            ));
        }
    }
    if !issues.is_empty() {
        return Err(ImportPlanBuildError::new(issues));
    }

    // Phase one: allocate the complete, deterministic batch identity map.
    let normalized_hash = context.preview_hash.trim().to_ascii_lowercase();
    let mut batch_ids = BTreeMap::new();
    let mut combined_animals = existing_animals.clone();
    for row in &preview.accepted_rows {
        let animal_id = stable_uuid(
            &normalized_hash,
            "animal",
            &format!("{}\0{}", row.source_row, row.display_id.trim()),
        );
        batch_ids.insert(row.display_id.trim().to_owned(), animal_id);
        combined_animals
            .insert(row.display_id.trim(), animal_id)
            .expect("validated display identifiers are non-empty");
    }

    // Phase two: resolve every foreign reference without producing a partial plan.
    let mut resolved_rows = Vec::with_capacity(preview.accepted_rows.len());
    for row in &preview.accepted_rows {
        if let Some(resolved) = resolve_animal_row(
            row,
            *batch_ids
                .get(row.display_id.trim())
                .expect("phase one assigned every row"),
            &combined_animals,
            cages,
            genetics,
            &mut issues,
        ) {
            resolved_rows.push(resolved);
        }
    }
    if !issues.is_empty() {
        return Err(ImportPlanBuildError::new(issues));
    }

    let mut plan = base_plan(context, &normalized_hash);
    for resolved in resolved_rows {
        append_resolved_animal(&mut plan, context, resolved);
    }
    validate_finished_plan(plan)
}

/// Converts a confirmed measurement preview into draft measurements scoped to
/// one explicit lab/project/experiment.
pub fn build_measurement_import_plan(
    preview: &MeasurementImportPreview,
    context: &MeasurementImportPlanContext,
) -> Result<ImportPlan, ImportPlanBuildError> {
    let mut issues = blocking_preview_issues(&preview.issues);
    validate_import_context(&context.import, &mut issues);
    if context.project_id.is_nil() {
        issues.push(global_issue(
            "project_id",
            "invalid_project_id",
            "measurement import project UUID must not be nil",
        ));
    }
    if context.experiment_id.is_nil() {
        issues.push(global_issue(
            "experiment_id",
            "invalid_experiment_id",
            "measurement import experiment UUID must not be nil",
        ));
    }
    if preview.accepted_rows.is_empty() {
        issues.push(global_issue(
            "measurements",
            "empty_import",
            "measurement import has no accepted rows",
        ));
    }

    let mut seen = BTreeSet::new();
    for row in &preview.accepted_rows {
        if row.animal_id.is_nil() {
            issues.push(plan_row_issue(
                row.source_row,
                "animal_uuid",
                "invalid_animal_uuid",
                "measurement animal UUID must not be nil",
            ));
        }
        if !seen.insert((row.animal_id, row.measurement_key.clone(), row.measured_at)) {
            issues.push(plan_row_issue(
                row.source_row,
                "measurement_key",
                "duplicate_measurement",
                "preview contains a duplicate animal/key/time measurement",
            ));
        }
        if measurement_import_value_type(&row.value) != row.value_type {
            issues.push(plan_row_issue(
                row.source_row,
                "value_type",
                "value_type_mismatch",
                "measurement value does not match its declared value type",
            ));
        }
        match context.measurement_labels.get(row.measurement_key.trim()) {
            None => issues.push(plan_row_issue(
                row.source_row,
                "measurement_key",
                "missing_measurement_label",
                "measurement key has no explicit human-readable label",
            )),
            Some(label) if label.trim().is_empty() => issues.push(plan_row_issue(
                row.source_row,
                "measurement_key",
                "empty_measurement_label",
                "measurement label must not be empty",
            )),
            Some(_) => {}
        }
    }
    if !issues.is_empty() {
        return Err(ImportPlanBuildError::new(issues));
    }

    let normalized_hash = context.import.preview_hash.trim().to_ascii_lowercase();
    let mut plan = base_plan(&context.import, &normalized_hash);
    for row in &preview.accepted_rows {
        let label = context
            .measurement_labels
            .get(row.measurement_key.trim())
            .expect("labels were validated for every row")
            .trim();
        match draft_measurement(row, context, label, &normalized_hash) {
            Ok(measurement) => plan.measurements.push(measurement),
            Err(message) => issues.push(plan_row_issue(
                row.source_row,
                "value",
                "invalid_measurement",
                &message,
            )),
        }
    }
    if !issues.is_empty() {
        return Err(ImportPlanBuildError::new(issues));
    }
    validate_finished_plan(plan)
}

#[derive(Debug)]
struct ResolvedAnimalRow<'a> {
    row: &'a AnimalImportRow,
    animal_id: Uuid,
    sex: Sex,
    cage_id: Option<Uuid>,
    genotype_definition_id: Option<Uuid>,
    father_id: Option<Uuid>,
    mother_id: Option<Uuid>,
}

fn resolve_animal_row<'a>(
    row: &'a AnimalImportRow,
    animal_id: Uuid,
    animals: &AnimalDirectory,
    cages: &CageDirectory,
    genetics: &GeneticDirectory,
    issues: &mut Vec<ImportIssue>,
) -> Option<ResolvedAnimalRow<'a>> {
    let issue_count_before = issues.len();
    let sex = match row.sex.as_deref().map(str::trim) {
        None | Some("") | Some("unknown") => Sex::Unknown,
        Some("male") => Sex::Male,
        Some("female") => Sex::Female,
        Some(_) => {
            issues.push(plan_row_issue(
                row.source_row,
                "sex",
                "invalid_normalized_sex",
                "animal preview sex must be male, female, or unknown",
            ));
            Sex::Unknown
        }
    };

    let cage_id = row
        .cage
        .as_deref()
        .and_then(|reference| match cages.resolve(reference) {
            DirectoryResolution::Unique(cage_id) => Some(cage_id),
            DirectoryResolution::Unknown => {
                issues.push(plan_row_issue(
                    row.source_row,
                    "cage",
                    "unknown_cage",
                    "cage identifier does not exist in the current lab directory",
                ));
                None
            }
            DirectoryResolution::Ambiguous => {
                issues.push(plan_row_issue(
                    row.source_row,
                    "cage",
                    "ambiguous_cage",
                    "cage display identifier is ambiguous; include its section",
                ));
                None
            }
        });

    let genotype_definition_id = row
        .genotype
        .as_deref()
        .and_then(|value| resolve_genotype_definition(value, row.source_row, genetics, issues));

    let father_id = resolve_parent(
        row.father.as_deref(),
        "father",
        row.source_row,
        animal_id,
        animals,
        issues,
    );
    let mother_id = resolve_parent(
        row.mother.as_deref(),
        "mother",
        row.source_row,
        animal_id,
        animals,
        issues,
    );
    if father_id.is_some() && father_id == mother_id {
        issues.push(plan_row_issue(
            row.source_row,
            "mother",
            "duplicate_parent",
            "father and mother must not resolve to the same animal",
        ));
    }

    (issues.len() == issue_count_before).then_some(ResolvedAnimalRow {
        row,
        animal_id,
        sex,
        cage_id,
        genotype_definition_id,
        father_id,
        mother_id,
    })
}

fn resolve_parent(
    reference: Option<&str>,
    field: &'static str,
    source_row: usize,
    animal_id: Uuid,
    animals: &AnimalDirectory,
    issues: &mut Vec<ImportIssue>,
) -> Option<Uuid> {
    let reference = reference.map(str::trim).filter(|value| !value.is_empty())?;
    match animals.resolve(reference) {
        AnimalResolution::Unique(parent_id) if parent_id == animal_id => {
            issues.push(plan_row_issue(
                source_row,
                field,
                "self_parent",
                "an animal cannot be its own parent",
            ));
            None
        }
        AnimalResolution::Unique(parent_id) => Some(parent_id),
        AnimalResolution::Unknown => {
            issues.push(plan_row_issue(
                source_row,
                field,
                "unknown_parent",
                "parent display identifier does not exist in the combined animal directory",
            ));
            None
        }
        AnimalResolution::Ambiguous => {
            issues.push(plan_row_issue(
                source_row,
                field,
                "ambiguous_parent",
                "parent display identifier is ambiguous and cannot be imported safely",
            ));
            None
        }
    }
}

fn resolve_genotype_definition(
    value: &str,
    source_row: usize,
    genetics: &GeneticDirectory,
    issues: &mut Vec<ImportIssue>,
) -> Option<Uuid> {
    let parsed = match parse_genotype(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                error.code,
                error.message,
            ));
            return None;
        }
    };
    let expected_components = parsed.len();
    let mut resolved = Vec::with_capacity(expected_components);
    let mut resolved_loci = BTreeSet::new();
    for genotype in parsed {
        let locus_id = match genetics.resolve_locus(&genotype.locus) {
            DirectoryResolution::Unique(locus_id) => locus_id,
            DirectoryResolution::Unknown => {
                issues.push(plan_row_issue(
                    source_row,
                    "genotype",
                    "unknown_locus",
                    &format!("unknown genotype locus: {}", genotype.locus),
                ));
                continue;
            }
            DirectoryResolution::Ambiguous => {
                issues.push(plan_row_issue(
                    source_row,
                    "genotype",
                    "ambiguous_locus",
                    &format!("ambiguous genotype locus: {}", genotype.locus),
                ));
                continue;
            }
        };
        if !resolved_loci.insert(locus_id) {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                "duplicate_genotype_locus",
                "genotype contains the same resolved locus more than once",
            ));
            continue;
        }
        let allele_1_id =
            resolve_allele(genetics, locus_id, &genotype.allele_1, source_row, issues);
        let allele_2_id =
            resolve_allele(genetics, locus_id, &genotype.allele_2, source_row, issues);
        if let (Some(allele_1_id), Some(allele_2_id)) = (allele_1_id, allele_2_id) {
            resolved.push((locus_id, allele_1_id, allele_2_id));
        }
    }
    if resolved.len() != expected_components {
        return None;
    }
    match genetics.resolve_definition(&resolved) {
        DirectoryResolution::Unique(definition_id) => Some(definition_id),
        DirectoryResolution::Unknown => {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                "unknown_genotype_definition",
                "resolved loci and alleles do not match an active existing genotype definition",
            ));
            None
        }
        DirectoryResolution::Ambiguous => {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                "ambiguous_genotype_definition",
                "resolved loci and alleles match more than one active genotype definition",
            ));
            None
        }
    }
}

fn resolve_allele(
    genetics: &GeneticDirectory,
    locus_id: Uuid,
    symbol: &str,
    source_row: usize,
    issues: &mut Vec<ImportIssue>,
) -> Option<Uuid> {
    match genetics.resolve_allele(locus_id, symbol) {
        DirectoryResolution::Unique(allele_id) => Some(allele_id),
        DirectoryResolution::Unknown => {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                "unknown_allele",
                &format!("unknown allele {symbol} for locus {locus_id}"),
            ));
            None
        }
        DirectoryResolution::Ambiguous => {
            issues.push(plan_row_issue(
                source_row,
                "genotype",
                "ambiguous_allele",
                &format!("ambiguous allele {symbol} for locus {locus_id}"),
            ));
            None
        }
    }
}

fn append_resolved_animal(
    plan: &mut ImportPlan,
    context: &ImportPlanContext,
    resolved: ResolvedAnimalRow<'_>,
) {
    let row = resolved.row;
    let hash = &plan.preview_hash;
    let identity = format!("{}\0{}", row.source_row, row.display_id.trim());
    let mut animal = Animal::new_mouse(
        context.lab_id,
        row.display_id.trim(),
        resolved.sex,
        context.confirmed_at,
    )
    .expect("animal display identifier was validated");
    animal.id = resolved.animal_id;
    animal.strain = row
        .strain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    animal.birth_date = row.birth_date;
    animal.current_cage_id = resolved.cage_id;
    plan.animals.push(animal);

    let mut registered = AnimalEvent::new(
        context.lab_id,
        resolved.animal_id,
        AnimalEventKind::Registered,
        context.confirmed_at,
        context.confirmed_at,
    );
    registered.id = stable_uuid(hash, "animal_event:registered", &identity);
    registered.recorded_by = Some(context.actor_user_id);
    plan.animal_events.push(registered);

    if let Some(birth_date) = row.birth_date {
        let occurred_at =
            DateTime::from_naive_utc_and_offset(birth_date.and_time(NaiveTime::MIN), Utc);
        let mut born = AnimalEvent::new(
            context.lab_id,
            resolved.animal_id,
            AnimalEventKind::Born { birth_date },
            occurred_at,
            context.confirmed_at,
        );
        born.id = stable_uuid(hash, "animal_event:born", &identity);
        born.recorded_by = Some(context.actor_user_id);
        plan.animal_events.push(born);
    }

    if let Some(cage_id) = resolved.cage_id {
        let mut transferred = AnimalEvent::new(
            context.lab_id,
            resolved.animal_id,
            AnimalEventKind::Transferred {
                from_cage_id: None,
                to_cage_id: Some(cage_id),
            },
            context.confirmed_at,
            context.confirmed_at,
        );
        transferred.id = stable_uuid(hash, "animal_event:transferred", &identity);
        transferred.recorded_by = Some(context.actor_user_id);
        plan.animal_events.push(transferred);
    }

    if let Some(definition_id) = resolved.genotype_definition_id {
        let mut record = GenotypingRecord::new(
            context.lab_id,
            resolved.animal_id,
            definition_id,
            GenotypingState::Expected,
            None,
            context.confirmed_at,
        )
        .expect("resolved expected genotyping record is valid");
        record.id = stable_uuid(
            hash,
            "genotyping_record",
            &format!("{identity}\0{definition_id}"),
        );
        plan.genotyping_records.push(record);
    }
    if let Some(parent_id) = resolved.father_id {
        plan.pedigrees.push(Pedigree {
            id: stable_uuid(hash, "pedigree:father", &identity),
            animal_id: resolved.animal_id,
            parent_id,
            parent_type: ParentType::Father,
            meta: RecordMeta::new(context.confirmed_at),
        });
    }
    if let Some(parent_id) = resolved.mother_id {
        plan.pedigrees.push(Pedigree {
            id: stable_uuid(hash, "pedigree:mother", &identity),
            animal_id: resolved.animal_id,
            parent_id,
            parent_type: ParentType::Mother,
            meta: RecordMeta::new(context.confirmed_at),
        });
    }
}

fn draft_measurement(
    row: &MeasurementImportRow,
    context: &MeasurementImportPlanContext,
    label: &str,
    preview_hash: &str,
) -> Result<Measurement, String> {
    let mut measurement = Measurement::draft(
        context.import.lab_id,
        context.project_id,
        row.animal_id,
        row.measurement_key.trim(),
        label,
        core_measurement_value(&row.value),
        row.measured_at,
        context.import.confirmed_at,
    )
    .map_err(|error| error.to_string())?;
    measurement.id = stable_uuid(
        preview_hash,
        "measurement",
        &format!(
            "{}\0{}\0{}\0{}",
            row.source_row,
            row.animal_id,
            row.measurement_key.trim(),
            row.measured_at.to_rfc3339()
        ),
    );
    measurement.experiment_id = Some(context.experiment_id);
    measurement.unit.clone_from(&row.unit);
    Ok(measurement)
}

fn core_measurement_value(value: &MeasurementImportValue) -> MeasurementValue {
    match value {
        MeasurementImportValue::Number(value) => MeasurementValue::Number(*value),
        MeasurementImportValue::Text(value) => MeasurementValue::Text(value.clone()),
        MeasurementImportValue::Boolean(value) => MeasurementValue::Boolean(*value),
        MeasurementImportValue::Date(value) => MeasurementValue::Date(*value),
        MeasurementImportValue::Category(value) => MeasurementValue::Category(value.clone()),
    }
}

fn measurement_import_value_type(value: &MeasurementImportValue) -> MeasurementValueType {
    match value {
        MeasurementImportValue::Number(_) => MeasurementValueType::Number,
        MeasurementImportValue::Text(_) => MeasurementValueType::Text,
        MeasurementImportValue::Boolean(_) => MeasurementValueType::Boolean,
        MeasurementImportValue::Date(_) => MeasurementValueType::Date,
        MeasurementImportValue::Category(_) => MeasurementValueType::Category,
    }
}

fn base_plan(context: &ImportPlanContext, normalized_hash: &str) -> ImportPlan {
    ImportPlan {
        commit_id: stable_uuid(
            normalized_hash,
            "import_commit",
            context.idempotency_key.trim(),
        ),
        lab_id: context.lab_id,
        idempotency_key: context.idempotency_key.trim().to_owned(),
        preview_hash: normalized_hash.to_owned(),
        animals: Vec::new(),
        animal_events: Vec::new(),
        genotyping_records: Vec::new(),
        pedigrees: Vec::new(),
        measurements: Vec::new(),
    }
}

fn validate_finished_plan(plan: ImportPlan) -> Result<ImportPlan, ImportPlanBuildError> {
    if let Err(error) = plan.validate() {
        return Err(ImportPlanBuildError::new(vec![global_issue(
            "plan",
            "invalid_import_plan",
            &error.to_string(),
        )]));
    }
    Ok(plan)
}

fn validate_import_context(context: &ImportPlanContext, issues: &mut Vec<ImportIssue>) {
    if context.lab_id.is_nil() {
        issues.push(global_issue(
            "lab_id",
            "invalid_lab_id",
            "import lab UUID must not be nil",
        ));
    }
    if context.actor_user_id.is_nil() {
        issues.push(global_issue(
            "actor_user_id",
            "invalid_actor_user_id",
            "import actor UUID must not be nil",
        ));
    }
    let idempotency_key = context.idempotency_key.trim();
    if idempotency_key.is_empty()
        || idempotency_key.chars().count() > 128
        || idempotency_key.chars().any(char::is_control)
    {
        issues.push(global_issue(
            "idempotency_key",
            "invalid_idempotency_key",
            "idempotency key must contain 1-128 non-control characters",
        ));
    }
    let preview_hash = context.preview_hash.trim();
    if preview_hash.len() != 64 || !preview_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        issues.push(global_issue(
            "preview_hash",
            "invalid_preview_hash",
            "preview hash must be a 64-character hexadecimal SHA-256",
        ));
    }
}

fn blocking_preview_issues(issues: &[ImportIssue]) -> Vec<ImportIssue> {
    issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Error)
        .cloned()
        .collect()
}

fn plan_row_issue(row: usize, field: &str, code: &str, message: &str) -> ImportIssue {
    ImportIssue {
        row: Some(row),
        field: Some(field.to_owned()),
        severity: IssueSeverity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn global_issue(field: &str, code: &str, message: &str) -> ImportIssue {
    ImportIssue {
        row: None,
        field: Some(field.to_owned()),
        severity: IssueSeverity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn resolution(ids: Option<&BTreeSet<Uuid>>) -> DirectoryResolution {
    match ids {
        None => DirectoryResolution::Unknown,
        Some(ids) if ids.len() == 1 => {
            DirectoryResolution::Unique(*ids.first().expect("one UUID is present"))
        }
        Some(_) => DirectoryResolution::Ambiguous,
    }
}

fn parse_qualified_cage(reference: &str) -> Option<(&str, &str)> {
    let pair = reference
        .split_once("::")
        .or_else(|| reference.split_once('/'))?;
    let section = pair.0.trim();
    let display_id = pair.1.trim();
    (!section.is_empty() && !display_id.is_empty()).then_some((section, display_id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedGenotype {
    locus: String,
    allele_1: String,
    allele_2: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GenotypeSyntaxError {
    code: &'static str,
    message: &'static str,
}

fn parse_genotype(value: &str) -> Result<Vec<ParsedGenotype>, GenotypeSyntaxError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid_genotype_syntax());
    }
    let mut parsed = Vec::new();
    let mut loci = BTreeSet::new();
    for component in value.split('&') {
        let component = component.trim();
        let Some(after_open_locus) = component.strip_prefix('{') else {
            return Err(invalid_genotype_syntax());
        };
        let Some(locus_end) = after_open_locus.find('}') else {
            return Err(invalid_genotype_syntax());
        };
        let locus = after_open_locus[..locus_end].trim();
        if locus.is_empty() || locus.contains(['{', '}', '[', ']', '&', '/']) {
            return Err(invalid_genotype_syntax());
        }
        let remainder = &after_open_locus[locus_end + 1..];
        let Some(after_open_allele_1) = remainder.strip_prefix('[') else {
            return Err(invalid_genotype_syntax());
        };
        let Some(allele_1_end) = after_open_allele_1.find(']') else {
            return Err(invalid_genotype_syntax());
        };
        let allele_1 = after_open_allele_1[..allele_1_end].trim();
        let remainder = &after_open_allele_1[allele_1_end + 1..];
        let Some(after_separator) = remainder.strip_prefix('/') else {
            return Err(invalid_genotype_syntax());
        };
        let Some(after_open_allele_2) = after_separator.strip_prefix('[') else {
            return Err(invalid_genotype_syntax());
        };
        let Some(allele_2_end) = after_open_allele_2.find(']') else {
            return Err(invalid_genotype_syntax());
        };
        let allele_2 = after_open_allele_2[..allele_2_end].trim();
        if !after_open_allele_2[allele_2_end + 1..].is_empty()
            || allele_1.is_empty()
            || allele_2.is_empty()
            || allele_1.contains(['[', ']', '{', '}', '&', '/'])
            || allele_2.contains(['[', ']', '{', '}', '&', '/'])
        {
            return Err(invalid_genotype_syntax());
        }
        if !loci.insert(locus.to_owned()) {
            return Err(GenotypeSyntaxError {
                code: "duplicate_genotype_locus",
                message: "genotype contains the same locus more than once",
            });
        }
        parsed.push(ParsedGenotype {
            locus: locus.to_owned(),
            allele_1: allele_1.to_owned(),
            allele_2: allele_2.to_owned(),
        });
    }
    Ok(parsed)
}

const fn invalid_genotype_syntax() -> GenotypeSyntaxError {
    GenotypeSyntaxError {
        code: "invalid_genotype_syntax",
        message: "genotype must use {Locus}[allele]/[allele] components joined by &",
    }
}

fn stable_uuid(preview_hash: &str, entity: &str, identity: &str) -> Uuid {
    let digest =
        Sha256::digest(format!("MuriArc\0import\0{preview_hash}\0{entity}\0{identity}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUIDv8 is reserved for application-defined deterministic identifiers.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use muriarc_core::{AnimalEventKind, RecordStatus};

    use super::*;

    fn import_context() -> ImportPlanContext {
        ImportPlanContext::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "confirmed-preview-1",
            "ab".repeat(32),
            DateTime::parse_from_rfc3339("2026-07-18T08:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    fn animal_row(display_id: &str) -> AnimalImportRow {
        AnimalImportRow {
            source_row: 2,
            display_id: display_id.to_owned(),
            sex: Some("female".to_owned()),
            birth_date: Some(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
            strain: Some("C57BL/6J".to_owned()),
            cage: None,
            genotype: None,
            father: None,
            mother: None,
        }
    }

    fn preview(rows: Vec<AnimalImportRow>) -> ImportPreview {
        ImportPreview {
            total_rows: rows.len(),
            accepted_rows: rows,
            issues: Vec::new(),
        }
    }

    fn issue_codes(error: &ImportPlanBuildError) -> BTreeSet<&str> {
        error
            .issues()
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }

    #[test]
    fn cage_requires_section_only_when_display_id_is_ambiguous() {
        let first = Uuid::from_u128(10);
        let second = Uuid::from_u128(11);
        let unique = Uuid::from_u128(12);
        let directory = CageDirectory::from_entries([
            ("A", "C1", first),
            ("B", "C1", second),
            ("A", "C2", unique),
        ])
        .unwrap();
        assert_eq!(directory.resolve("C1"), DirectoryResolution::Ambiguous);
        assert_eq!(
            directory.resolve("A/C1"),
            DirectoryResolution::Unique(first)
        );
        assert_eq!(
            directory.resolve("B::C1"),
            DirectoryResolution::Unique(second)
        );
        assert_eq!(directory.resolve("C2"), DirectoryResolution::Unique(unique));
        assert_eq!(directory.resolve("C404"), DirectoryResolution::Unknown);
    }

    #[test]
    fn strict_genotype_parser_accepts_only_the_documented_shape() {
        let parsed = parse_genotype("{GeneA}[+]/[fl]&{Rosa26}[wt]/[tdT]").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].locus, "GeneA");
        for invalid in [
            "GeneA[+]/[fl]",
            "{GeneA}+/fl",
            "{GeneA}[]/[fl]",
            "{GeneA}[+]/[fl] trailing",
            "{GeneA}[+]/[fl]&",
        ] {
            assert_eq!(
                parse_genotype(invalid).unwrap_err().code,
                "invalid_genotype_syntax",
                "{invalid}"
            );
        }
        assert_eq!(
            parse_genotype("{GeneA}[+]/[fl]&{GeneA}[fl]/[fl]")
                .unwrap_err()
                .code,
            "duplicate_genotype_locus"
        );
    }

    #[test]
    fn animal_plan_resolves_same_batch_parent_cage_and_genotype() {
        let cage_id = Uuid::from_u128(20);
        let locus_id = Uuid::from_u128(21);
        let wild_type = Uuid::from_u128(22);
        let floxed = Uuid::from_u128(23);
        let definition_id = Uuid::from_u128(24);
        let cages = CageDirectory::from_entries([("Room A", "C1", cage_id)]).unwrap();
        let genetics = GeneticDirectory::from_entries_with_definitions(
            [("GeneA", locus_id)],
            [(locus_id, "+", wild_type), (locus_id, "fl", floxed)],
            [(definition_id, vec![(locus_id, wild_type, floxed)])],
        )
        .unwrap();
        let mut father = animal_row("F1");
        father.source_row = 2;
        father.sex = Some("male".to_owned());
        father.birth_date = None;
        let mut child = animal_row("C1");
        child.source_row = 3;
        child.cage = Some("Room A/C1".to_owned());
        child.genotype = Some("{GeneA}[+]/[fl]".to_owned());
        child.father = Some("F1".to_owned());
        let input = preview(vec![father, child]);

        let plan = build_animal_import_plan(
            &input,
            &import_context(),
            &AnimalDirectory::default(),
            &cages,
            &genetics,
        )
        .unwrap();
        assert_eq!(plan.animals.len(), 2);
        assert_eq!(plan.genotyping_records.len(), 1);
        assert_eq!(plan.pedigrees.len(), 1);
        let father_id = plan
            .animals
            .iter()
            .find(|animal| animal.display_id == "F1")
            .unwrap()
            .id;
        let child = plan
            .animals
            .iter()
            .find(|animal| animal.display_id == "C1")
            .unwrap();
        assert_eq!(child.current_cage_id, Some(cage_id));
        assert_eq!(plan.pedigrees[0].parent_id, father_id);
        assert_eq!(plan.genotyping_records[0].animal_id, child.id);
        assert_eq!(
            plan.genotyping_records[0].genotype_definition_id,
            definition_id
        );
        assert_eq!(plan.genotyping_records[0].state, GenotypingState::Expected);
        assert_eq!(plan.genotyping_records[0].assessed_at, None);
        assert!(plan.animal_events.iter().all(|event| {
            event.recorded_by == Some(import_context().actor_user_id)
                && matches!(
                    event.kind,
                    AnimalEventKind::Registered
                        | AnimalEventKind::Born { .. }
                        | AnimalEventKind::Transferred { .. }
                )
        }));

        let rebuilt = build_animal_import_plan(
            &input,
            &import_context(),
            &AnimalDirectory::default(),
            &cages,
            &genetics,
        )
        .unwrap();
        assert_eq!(plan.commit_id, rebuilt.commit_id);
        assert_eq!(plan.animals[0].id, rebuilt.animals[0].id);
        assert_eq!(plan.animal_events[0].id, rebuilt.animal_events[0].id);
    }

    #[test]
    fn unknown_and_ambiguous_animal_directories_block_the_plan() {
        let mut row = animal_row("M1");
        row.cage = Some("C1".to_owned());
        row.father = Some("P1".to_owned());
        let cages = CageDirectory::from_entries([
            ("A", "C1", Uuid::from_u128(30)),
            ("B", "C1", Uuid::from_u128(31)),
        ])
        .unwrap();
        let parents = AnimalDirectory::from_entries([
            ("P1", Uuid::from_u128(32)),
            ("P1", Uuid::from_u128(33)),
        ])
        .unwrap();
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &parents,
            &cages,
            &GeneticDirectory::default(),
        )
        .unwrap_err();
        let codes = issue_codes(&error);
        assert!(codes.contains("ambiguous_cage"));
        assert!(codes.contains("ambiguous_parent"));

        let mut unknown = animal_row("M2");
        unknown.cage = Some("C404".to_owned());
        unknown.mother = Some("P404".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![unknown]),
            &import_context(),
            &AnimalDirectory::default(),
            &cages,
            &GeneticDirectory::default(),
        )
        .unwrap_err();
        let codes = issue_codes(&error);
        assert!(codes.contains("unknown_cage"));
        assert!(codes.contains("unknown_parent"));
    }

    #[test]
    fn unknown_and_ambiguous_genetics_never_drop_genotype_text() {
        let locus_a = Uuid::from_u128(40);
        let locus_b = Uuid::from_u128(41);
        let ambiguous = GeneticDirectory::from_entries(
            [("GeneA", locus_a), ("GeneA", locus_b)],
            Vec::<(Uuid, &str, Uuid)>::new(),
        )
        .unwrap();
        let mut row = animal_row("M1");
        row.genotype = Some("{GeneA}[+]/[fl]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &ambiguous,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("ambiguous_locus"));

        let locus_id = Uuid::from_u128(42);
        let unknown_allele = GeneticDirectory::from_entries(
            [("GeneA", locus_id)],
            [(locus_id, "+", Uuid::from_u128(43))],
        )
        .unwrap();
        let mut row = animal_row("M2");
        row.genotype = Some("{GeneA}[+]/[fl]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &unknown_allele,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("unknown_allele"));

        let mut row = animal_row("M3");
        row.genotype = Some("{Missing}[+]/[+]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &unknown_allele,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("unknown_locus"));

        let locus_id = Uuid::from_u128(44);
        let ambiguous_allele = GeneticDirectory::from_entries(
            [("GeneA", locus_id)],
            [
                (locus_id, "+", Uuid::from_u128(45)),
                (locus_id, "+", Uuid::from_u128(46)),
            ],
        )
        .unwrap();
        let mut row = animal_row("M4");
        row.genotype = Some("{GeneA}[+]/[+]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &ambiguous_allele,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("ambiguous_allele"));

        let mut row = animal_row("M5");
        row.genotype = Some("GeneA:+/fl".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &ambiguous_allele,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("invalid_genotype_syntax"));

        let locus_id = Uuid::from_u128(47);
        let first = Uuid::from_u128(48);
        let second = Uuid::from_u128(49);
        let without_definition = GeneticDirectory::from_entries(
            [("GeneB", locus_id)],
            [(locus_id, "+", first), (locus_id, "fl", second)],
        )
        .unwrap();
        let mut row = animal_row("M6");
        row.genotype = Some("{GeneB}[+]/[fl]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &without_definition,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("unknown_genotype_definition"));

        let ambiguous_definition = GeneticDirectory::from_entries_with_definitions(
            [("GeneB", locus_id)],
            [(locus_id, "+", first), (locus_id, "fl", second)],
            [
                (Uuid::from_u128(50), vec![(locus_id, first, second)]),
                (Uuid::from_u128(51), vec![(locus_id, first, second)]),
            ],
        )
        .unwrap();
        let mut row = animal_row("M7");
        row.genotype = Some("{GeneB}[+]/[fl]".to_owned());
        let error = build_animal_import_plan(
            &preview(vec![row]),
            &import_context(),
            &AnimalDirectory::default(),
            &CageDirectory::default(),
            &ambiguous_definition,
        )
        .unwrap_err();
        assert!(issue_codes(&error).contains("ambiguous_genotype_definition"));
    }

    #[test]
    fn measurement_plan_creates_explicitly_scoped_unsigned_drafts() {
        let import = import_context();
        let project_id = Uuid::from_u128(50);
        let experiment_id = Uuid::from_u128(51);
        let animal_id = Uuid::from_u128(52);
        let measured_at = import.confirmed_at;
        let preview = MeasurementImportPreview {
            total_rows: 1,
            accepted_rows: vec![MeasurementImportRow {
                source_row: 2,
                animal_id,
                display_id: "M1".to_owned(),
                measurement_key: "body_weight".to_owned(),
                value_type: MeasurementValueType::Number,
                value: MeasurementImportValue::Number(23.5),
                unit: Some("g".to_owned()),
                measured_at,
            }],
            issues: Vec::new(),
        };
        let context = MeasurementImportPlanContext::new(
            import,
            project_id,
            experiment_id,
            [("body_weight", "Body weight")],
        );
        let plan = build_measurement_import_plan(&preview, &context).unwrap();
        let measurement = &plan.measurements[0];
        assert_eq!(measurement.lab_id, context.import.lab_id);
        assert_eq!(measurement.project_id, project_id);
        assert_eq!(measurement.experiment_id, Some(experiment_id));
        assert_eq!(measurement.animal_id, animal_id);
        assert_eq!(measurement.label, "Body weight");
        assert_eq!(measurement.unit.as_deref(), Some("g"));
        assert_eq!(measurement.status, RecordStatus::Draft);
        assert_eq!(measurement.signed_by, None);
        assert_eq!(measurement.signed_at, None);
    }

    #[test]
    fn measurement_plan_requires_label_and_matching_value_type() {
        let import = import_context();
        let preview = MeasurementImportPreview {
            total_rows: 1,
            accepted_rows: vec![MeasurementImportRow {
                source_row: 2,
                animal_id: Uuid::from_u128(60),
                display_id: "M1".to_owned(),
                measurement_key: "body_weight".to_owned(),
                value_type: MeasurementValueType::Text,
                value: MeasurementImportValue::Number(23.5),
                unit: Some("g".to_owned()),
                measured_at: import.confirmed_at,
            }],
            issues: Vec::new(),
        };
        let context = MeasurementImportPlanContext::new(
            import,
            Uuid::from_u128(61),
            Uuid::from_u128(62),
            Vec::<(String, String)>::new(),
        );
        let error = build_measurement_import_plan(&preview, &context).unwrap_err();
        let codes = issue_codes(&error);
        assert!(codes.contains("missing_measurement_label"));
        assert!(codes.contains("value_type_mismatch"));
    }
}
