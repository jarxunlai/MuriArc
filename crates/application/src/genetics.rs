use chrono::{DateTime, Utc};
use muriarc_core::{
    Allele, AuditContext, GeneLocus, Genotype, GenotypeComponent, GenotypeComponentMode,
    GenotypeDefinition, GenotypingRecord, GenotypingState, MuriArcStore, ParentType, Pedigree,
    RecordMeta,
};
use uuid::Uuid;

use crate::validation::{normalized_optional_bytes, normalized_required_bytes};
use crate::{ApplicationError, ApplicationResult};

pub const MAX_GENETIC_SYMBOL_BYTES: usize = 128;
pub const MAX_GENETIC_DESCRIPTION_BYTES: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGeneLocusCommand {
    pub lab_id: Uuid,
    pub symbol: String,
    pub description: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_gene_locus(
    store: &dyn MuriArcStore,
    command: CreateGeneLocusCommand,
    audit: &AuditContext,
) -> ApplicationResult<GeneLocus> {
    let locus = GeneLocus {
        id: Uuid::new_v4(),
        lab_id: command.lab_id,
        symbol: normalized_required_bytes(
            "gene_locus.symbol",
            command.symbol,
            MAX_GENETIC_SYMBOL_BYTES,
        )?,
        description: normalized_optional_bytes(
            "gene_locus.description",
            command.description,
            MAX_GENETIC_DESCRIPTION_BYTES,
        )?,
        meta: RecordMeta::new(command.now),
    };
    store.create_gene_locus(&locus, audit).await?;
    Ok(locus)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAlleleCommand {
    pub locus_id: Uuid,
    pub symbol: String,
    pub description: Option<String>,
    pub is_wild_type: bool,
    pub now: DateTime<Utc>,
}

pub async fn create_allele(
    store: &dyn MuriArcStore,
    command: CreateAlleleCommand,
    audit: &AuditContext,
) -> ApplicationResult<Allele> {
    let allele = Allele {
        id: Uuid::new_v4(),
        locus_id: command.locus_id,
        symbol: normalized_required_bytes(
            "allele.symbol",
            command.symbol,
            MAX_GENETIC_SYMBOL_BYTES,
        )?,
        description: normalized_optional_bytes(
            "allele.description",
            command.description,
            MAX_GENETIC_DESCRIPTION_BYTES,
        )?,
        is_wild_type: command.is_wild_type,
        meta: RecordMeta::new(command.now),
    };
    store.create_allele(&allele, audit).await?;
    Ok(allele)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateGenotypeCommand {
    pub animal_id: Uuid,
    pub locus_id: Uuid,
    pub allele_1_id: Option<Uuid>,
    pub allele_2_id: Option<Uuid>,
    pub assessed_at: Option<DateTime<Utc>>,
    pub project_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

pub async fn create_genotype(
    store: &dyn MuriArcStore,
    command: CreateGenotypeCommand,
    audit: &AuditContext,
) -> ApplicationResult<Genotype> {
    for allele_id in [command.allele_1_id, command.allele_2_id]
        .into_iter()
        .flatten()
    {
        let allele = store.get_allele(allele_id).await?;
        if allele.locus_id != command.locus_id {
            return Err(ApplicationError::Validation(
                "genotype allele belongs to a different gene locus".to_owned(),
            ));
        }
    }
    let genotype = Genotype {
        id: Uuid::new_v4(),
        animal_id: command.animal_id,
        locus_id: command.locus_id,
        allele_1_id: command.allele_1_id,
        allele_2_id: command.allele_2_id,
        assessed_at: command.assessed_at,
        meta: RecordMeta::new(command.now),
    };
    store
        .create_genotype(&genotype, command.project_id, audit)
        .await?;
    Ok(genotype)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatePedigreeCommand {
    pub animal_id: Uuid,
    pub parent_id: Uuid,
    pub parent_type: ParentType,
    pub now: DateTime<Utc>,
}

pub async fn create_pedigree(
    store: &dyn MuriArcStore,
    command: CreatePedigreeCommand,
    audit: &AuditContext,
) -> ApplicationResult<Pedigree> {
    if command.animal_id == command.parent_id {
        return Err(ApplicationError::Validation(
            "an animal cannot be its own parent".to_owned(),
        ));
    }
    let pedigree = Pedigree {
        id: Uuid::new_v4(),
        animal_id: command.animal_id,
        parent_id: command.parent_id,
        parent_type: command.parent_type,
        meta: RecordMeta::new(command.now),
    };
    store.create_pedigree(&pedigree, audit).await?;
    Ok(pedigree)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateGenotypeComponentInput {
    pub locus_id: Uuid,
    pub allele_1_id: Uuid,
    pub allele_2_id: Option<Uuid>,
    pub mode: GenotypeComponentMode,
    pub display_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGenotypeDefinitionCommand {
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub components: Vec<CreateGenotypeComponentInput>,
    pub now: DateTime<Utc>,
}

pub async fn create_genotype_definition(
    store: &dyn MuriArcStore,
    command: CreateGenotypeDefinitionCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypeDefinition> {
    let name = normalized_required_bytes(
        "genotype_definition.name",
        command.name,
        MAX_GENETIC_SYMBOL_BYTES,
    )?;
    let description = normalized_optional_bytes(
        "genotype_definition.description",
        command.description,
        MAX_GENETIC_DESCRIPTION_BYTES,
    )?;
    let mut definition = GenotypeDefinition::new(command.lab_id, name, command.now)?;
    definition.description = description;

    let mut components = Vec::with_capacity(command.components.len());
    for input in command.components {
        let locus = store.get_gene_locus(input.locus_id).await?;
        if locus.lab_id != command.lab_id {
            return Err(ApplicationError::Validation(
                "genotype definition locus belongs to a different lab".to_owned(),
            ));
        }
        for allele_id in [Some(input.allele_1_id), input.allele_2_id]
            .into_iter()
            .flatten()
        {
            let allele = store.get_allele(allele_id).await?;
            if allele.locus_id != input.locus_id {
                return Err(ApplicationError::Validation(
                    "genotype definition allele belongs to a different gene locus".to_owned(),
                ));
            }
        }
        components.push(GenotypeComponent::new(
            definition.id,
            input.locus_id,
            input.allele_1_id,
            input.allele_2_id,
            input.mode,
            input.display_order,
            command.now,
        )?);
    }
    definition.replace_components(components)?;
    store.create_genotype_definition(&definition, audit).await?;
    Ok(definition)
}

pub const MAX_GENOTYPING_METHOD_BYTES: usize = 512;
pub const MAX_GENOTYPING_NOTES_BYTES: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGenotypingRecordCommand {
    pub lab_id: Uuid,
    pub project_id: Option<Uuid>,
    pub animal_id: Uuid,
    pub genotype_definition_id: Uuid,
    pub state: GenotypingState,
    pub assessed_at: Option<DateTime<Utc>>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub now: DateTime<Utc>,
}

pub async fn create_genotyping_record(
    store: &dyn MuriArcStore,
    command: CreateGenotypingRecordCommand,
    audit: &AuditContext,
) -> ApplicationResult<GenotypingRecord> {
    let mut record = GenotypingRecord::new(
        command.lab_id,
        command.animal_id,
        command.genotype_definition_id,
        command.state,
        command.assessed_at,
        command.now,
    )?;
    record.project_id = command.project_id;
    record.method = normalized_optional_bytes(
        "genotyping_record.method",
        command.method,
        MAX_GENOTYPING_METHOD_BYTES,
    )?;
    record.notes = normalized_optional_bytes(
        "genotyping_record.notes",
        command.notes,
        MAX_GENOTYPING_NOTES_BYTES,
    )?;
    record.validate()?;
    store.create_genotyping_record(&record, audit).await?;
    Ok(record)
}
