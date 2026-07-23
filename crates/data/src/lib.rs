#![forbid(unsafe_code)]

mod ai_source_cleanup;
mod ai_source_import;
mod attachment_files;
mod attachment_inspection;
mod source_material;

pub use ai_source_cleanup::{
    AiConversationSourceCleanupReport, cleanup_expired_ai_conversation_sources,
};
pub use ai_source_import::{
    AiSourceImportValidationError, ValidatedAiSourceImport, ai_source_import_idempotency_key,
    validate_ai_source_import,
};
pub use attachment_files::{
    AttachmentFileError, AttachmentFiles, MAX_ATTACHMENT_BYTES, StoredAttachmentObject,
    VerifiedAttachmentObject,
};
pub use attachment_inspection::{
    AttachmentContentKind, AttachmentInspection, AttachmentInspectionError, inspect_attachment,
};
pub use source_material::{
    AiSourceMaterial, AiSourceMaterialError, AiSourceVisionAsset, MAX_AI_SOURCE_COLUMNS,
    MAX_AI_SOURCE_ROWS, MAX_AI_SOURCE_TEXT_BYTES, MAX_AI_SOURCE_VISION_ASSETS,
    extract_ai_source_material, extract_ai_source_vision_assets,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use muriarc_core::{
    AiImportResolution, AnimalFilter, AnimalStatus, Attachment, AuditContext, AuditFilter,
    EntityType, ExperimentFilter, FieldValueType, GenotypeComponentMode, GenotypingState,
    IdentifierScope, ImportCommitOptions, ImportCommitResult, ImportPlan, ImportSourceArchive, Job,
    MeasurementFilter, MuriArcStore, ObservationFilter, ParticipationFilter,
    ProjectAnimalAssignmentFilter, ProvenanceFilter, SampleFilter, Sex, StoreError, TemplateStatus,
    is_ai_managed_attachment_entity_type, is_ai_operational_or_configuration_entity_type,
    is_ai_source_import_job, is_private_ai_source_attachment_audit, is_private_ai_source_job_audit,
};
use muriarc_importer::{
    AnimalDirectory, AnimalExportFilter, AnimalExportOptions, AnimalExportRecord, CageDirectory,
    ExportAnimalStatus, ExportCage, ExportGenotype, ExportGenotypingState, ExportSex, FieldMapping,
    GeneticDirectory, ImportError, ImportIssue, ImportPlanContext, ImportPreview, IssueSeverity,
    MeasurementCatalog, MeasurementDefinition, MeasurementFieldMapping,
    MeasurementImportPlanContext, MeasurementImportPreview, MeasurementImportValue,
    MeasurementValueType, TabularData, build_animal_import_plan, build_measurement_import_plan,
    export_animals_csv_with_options, export_animals_xlsx_with_options,
    preview_animals_with_directory, preview_measurements, read_csv, read_xlsx,
};
use muriarc_snapshot::{
    BundleEntry, EntryKind, SnapshotError, SnapshotManifest, sha256_hex, write_bundle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const WORKFLOW_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    #[default]
    Animal,
    Measurement,
}

#[derive(Debug, Clone)]
pub struct DataFiles {
    root: Arc<PathBuf>,
    max_upload_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportConfirmOptions {
    pub source_archive: Option<ImportSourceArchive>,
    pub ai_resolution: Option<AiImportResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadReceipt {
    pub job_id: Uuid,
    pub original_file_name: String,
    pub extension: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAnimalImport {
    pub schema_version: u32,
    pub job_id: Uuid,
    pub lab_id: Uuid,
    pub created_by: Uuid,
    pub project_id: Option<Uuid>,
    pub source: UploadReceipt,
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub mapping: FieldMapping,
    pub preview_hash: String,
    pub preview: ImportPreview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingMeasurementImport {
    pub schema_version: u32,
    pub job_id: Uuid,
    pub lab_id: Uuid,
    pub created_by: Uuid,
    pub project_id: Uuid,
    pub experiment_id: Uuid,
    pub template_version_id: Uuid,
    pub template_revision: i64,
    pub source: UploadReceipt,
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub mapping: MeasurementFieldMapping,
    pub measurement_labels: BTreeMap<String, String>,
    pub preview_hash: String,
    pub preview: MeasurementImportPreview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimalImportPreviewResponse {
    pub import_kind: ImportKind,
    pub experiment_id: Option<Uuid>,
    pub job_id: Uuid,
    pub file_name: String,
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub mapping: FieldMapping,
    pub preview_hash: String,
    pub total_rows: usize,
    pub accepted_rows: usize,
    pub preview_rows: Vec<BTreeMap<String, String>>,
    pub issues: Vec<ImportIssue>,
    pub can_confirm: bool,
}

impl From<&PendingAnimalImport> for AnimalImportPreviewResponse {
    fn from(value: &PendingAnimalImport) -> Self {
        Self {
            import_kind: ImportKind::Animal,
            experiment_id: None,
            job_id: value.job_id,
            file_name: value.source.original_file_name.clone(),
            sheet_name: value.sheet_name.clone(),
            headers: value.headers.clone(),
            mapping: value.mapping.clone(),
            preview_hash: value.preview_hash.clone(),
            total_rows: value.preview.total_rows,
            accepted_rows: value.preview.accepted_rows.len(),
            preview_rows: value
                .preview
                .accepted_rows
                .iter()
                .take(20)
                .map(|row| {
                    BTreeMap::from([
                        ("display_id".to_owned(), row.display_id.clone()),
                        ("sex".to_owned(), row.sex.clone().unwrap_or_default()),
                        (
                            "birth_date".to_owned(),
                            row.birth_date
                                .map(|date| date.to_string())
                                .unwrap_or_default(),
                        ),
                        ("strain".to_owned(), row.strain.clone().unwrap_or_default()),
                        ("cage".to_owned(), row.cage.clone().unwrap_or_default()),
                        (
                            "genotype".to_owned(),
                            row.genotype.clone().unwrap_or_default(),
                        ),
                        ("father".to_owned(), row.father.clone().unwrap_or_default()),
                        ("mother".to_owned(), row.mother.clone().unwrap_or_default()),
                    ])
                })
                .collect(),
            issues: value.preview.issues.clone(),
            can_confirm: value.preview.can_confirm(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementImportPreviewResponse {
    pub import_kind: ImportKind,
    pub experiment_id: Uuid,
    pub job_id: Uuid,
    pub file_name: String,
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub mapping: MeasurementFieldMapping,
    pub preview_hash: String,
    pub total_rows: usize,
    pub accepted_rows: usize,
    pub preview_rows: Vec<BTreeMap<String, String>>,
    pub issues: Vec<ImportIssue>,
    pub can_confirm: bool,
}

impl From<&PendingMeasurementImport> for MeasurementImportPreviewResponse {
    fn from(value: &PendingMeasurementImport) -> Self {
        Self {
            import_kind: ImportKind::Measurement,
            experiment_id: value.experiment_id,
            job_id: value.job_id,
            file_name: value.source.original_file_name.clone(),
            sheet_name: value.sheet_name.clone(),
            headers: value.headers.clone(),
            mapping: value.mapping.clone(),
            preview_hash: value.preview_hash.clone(),
            total_rows: value.preview.total_rows,
            accepted_rows: value.preview.accepted_rows.len(),
            preview_rows: measurement_preview_rows(&value.preview),
            issues: value.preview.issues.clone(),
            can_confirm: value.preview.can_confirm(),
        }
    }
}

impl From<&PendingMeasurementImport> for AnimalImportPreviewResponse {
    fn from(value: &PendingMeasurementImport) -> Self {
        Self {
            import_kind: ImportKind::Measurement,
            experiment_id: Some(value.experiment_id),
            job_id: value.job_id,
            file_name: value.source.original_file_name.clone(),
            sheet_name: value.sheet_name.clone(),
            headers: value.headers.clone(),
            mapping: FieldMapping {
                columns: value.mapping.columns.clone(),
            },
            preview_hash: value.preview_hash.clone(),
            total_rows: value.preview.total_rows,
            accepted_rows: value.preview.accepted_rows.len(),
            preview_rows: measurement_preview_rows(&value.preview),
            issues: value.preview.issues.clone(),
            can_confirm: value.preview.can_confirm(),
        }
    }
}

fn measurement_preview_rows(preview: &MeasurementImportPreview) -> Vec<BTreeMap<String, String>> {
    preview
        .accepted_rows
        .iter()
        .take(20)
        .map(|row| {
            let value = match &row.value {
                MeasurementImportValue::Number(value) => value.to_string(),
                MeasurementImportValue::Text(value) | MeasurementImportValue::Category(value) => {
                    value.clone()
                }
                MeasurementImportValue::Boolean(value) => value.to_string(),
                MeasurementImportValue::Date(value) => value.to_string(),
            };
            BTreeMap::from([
                ("row_number".to_owned(), row.source_row.to_string()),
                ("animal_id".to_owned(), row.animal_id.to_string()),
                ("animal_display_id".to_owned(), row.display_id.clone()),
                ("measurement_key".to_owned(), row.measurement_key.clone()),
                ("value".to_owned(), value),
                ("unit".to_owned(), row.unit.clone().unwrap_or_default()),
                ("measured_at".to_owned(), row.measured_at.to_rfc3339()),
            ])
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRemapJobResult {
    pub source_job_id: Uuid,
    pub mapping: FieldMapping,
    pub preview: AnimalImportPreviewResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Xlsx,
}

impl ExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Csv => "text/csv; charset=utf-8",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Export,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub schema_version: u32,
    pub job_id: Uuid,
    pub kind: ArtifactKind,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct OpenArtifact {
    pub metadata: ArtifactMetadata,
    pub file: tokio::fs::File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactWriteOutcome {
    Stored,
    Identical,
}

impl DataFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_upload_limit(root, DEFAULT_MAX_UPLOAD_BYTES)
    }

    pub fn with_upload_limit(root: impl Into<PathBuf>, max_upload_bytes: u64) -> Self {
        Self {
            root: Arc::new(root.into()),
            max_upload_bytes,
        }
    }

    pub const fn max_upload_bytes(&self) -> u64 {
        self.max_upload_bytes
    }

    async fn ensure_layout(&self) -> Result<(), DataError> {
        for directory in ["uploads", "pending", "artifacts"] {
            tokio::fs::create_dir_all(self.root.join(directory)).await?;
        }
        Ok(())
    }

    pub async fn instance_id(&self) -> Result<Uuid, DataError> {
        self.ensure_layout().await?;
        let path = self.root.join("instance-id");
        match tokio::fs::read_to_string(&path).await {
            Ok(value) => return parse_uuid_file(&value, "instance-id"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let id = Uuid::new_v4();
        match tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
        {
            Ok(mut file) => {
                file.write_all(id.to_string().as_bytes()).await?;
                file.sync_all().await?;
                Ok(id)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                parse_uuid_file(&tokio::fs::read_to_string(path).await?, "instance-id")
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn write_upload<R>(
        &self,
        job_id: Uuid,
        original_file_name: &str,
        mut reader: R,
    ) -> Result<UploadReceipt, DataError>
    where
        R: AsyncRead + Unpin,
    {
        self.ensure_layout().await?;
        let (safe_name, extension) = validate_upload_name(original_file_name)?;
        let final_path = self.upload_content_path(job_id);
        let metadata_path = self.upload_metadata_path(job_id);
        let temp_path = self
            .root
            .join("uploads")
            .join(format!("{job_id}.{}.part", Uuid::new_v4()));
        let result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .await?;
            let mut hasher = Sha256::new();
            let mut total = 0_u64;
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                total = total
                    .checked_add(read as u64)
                    .ok_or(DataError::UploadTooLarge(self.max_upload_bytes))?;
                if total > self.max_upload_bytes {
                    return Err(DataError::UploadTooLarge(self.max_upload_bytes));
                }
                hasher.update(&buffer[..read]);
                file.write_all(&buffer[..read]).await?;
            }
            if total == 0 {
                return Err(DataError::EmptyUpload);
            }
            file.sync_all().await?;
            drop(file);
            let receipt = UploadReceipt {
                job_id,
                original_file_name: safe_name,
                extension,
                size_bytes: total,
                sha256: format!("{:x}", hasher.finalize()),
            };
            self.install_upload(&temp_path, &final_path, &metadata_path, &receipt)
                .await?;
            Ok(receipt)
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp_path).await;
        }
        result
    }

    pub async fn write_upload_bytes(
        &self,
        job_id: Uuid,
        original_file_name: &str,
        bytes: &[u8],
    ) -> Result<UploadReceipt, DataError> {
        self.write_upload(job_id, original_file_name, std::io::Cursor::new(bytes))
            .await
    }

    async fn install_upload(
        &self,
        temp_path: &Path,
        final_path: &Path,
        metadata_path: &Path,
        receipt: &UploadReceipt,
    ) -> Result<(), DataError> {
        match tokio::fs::hard_link(temp_path, final_path).await {
            Ok(()) => {
                tokio::fs::remove_file(temp_path).await?;
                if let Err(error) = write_json_create_new(metadata_path, receipt).await {
                    let _ = tokio::fs::remove_file(final_path).await;
                    return Err(error);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read_upload_receipt(receipt.job_id).await?;
                let _ = tokio::fs::remove_file(temp_path).await;
                if existing.sha256.eq_ignore_ascii_case(&receipt.sha256)
                    && existing.size_bytes == receipt.size_bytes
                    && existing.extension == receipt.extension
                    && existing.original_file_name == receipt.original_file_name
                {
                    Ok(())
                } else {
                    Err(DataError::Conflict(
                        "job upload already exists with different content or file name".to_owned(),
                    ))
                }
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(temp_path).await;
                Err(error.into())
            }
        }
    }

    pub async fn read_upload_receipt(&self, job_id: Uuid) -> Result<UploadReceipt, DataError> {
        read_json(&self.upload_metadata_path(job_id)).await
    }

    pub async fn read_upload_bytes(&self, job_id: Uuid) -> Result<Vec<u8>, DataError> {
        let receipt = self.read_upload_receipt(job_id).await?;
        let bytes = tokio::fs::read(self.upload_content_path(job_id)).await?;
        verify_content(&bytes, receipt.size_bytes, &receipt.sha256, "upload")?;
        Ok(bytes)
    }

    pub async fn clear_upload(&self, job_id: Uuid) -> Result<(), DataError> {
        remove_if_exists(self.upload_content_path(job_id)).await?;
        remove_if_exists(self.upload_metadata_path(job_id)).await?;
        Ok(())
    }

    /// Copies a verified upload into a new job namespace. The source bytes are
    /// checksum-verified before the destination receipt is created.
    pub async fn copy_upload(
        &self,
        source_job_id: Uuid,
        destination_job_id: Uuid,
    ) -> Result<UploadReceipt, DataError> {
        let source = self.read_upload_receipt(source_job_id).await?;
        let bytes = self.read_upload_bytes(source_job_id).await?;
        let copied = self
            .write_upload_bytes(destination_job_id, &source.original_file_name, &bytes)
            .await?;
        if copied.sha256 != source.sha256 || copied.size_bytes != source.size_bytes {
            return Err(DataError::ChecksumMismatch("copied upload"));
        }
        Ok(copied)
    }

    async fn read_upload_table(
        &self,
        job_id: Uuid,
    ) -> Result<(UploadReceipt, TabularData), DataError> {
        let source = self.read_upload_receipt(job_id).await?;
        let bytes = self.read_upload_bytes(job_id).await?;
        let table = match source.extension.as_str() {
            "csv" => read_csv(Cursor::new(bytes))?,
            "xlsx" => read_xlsx(Cursor::new(bytes))?,
            extension => return Err(DataError::UnsupportedUpload(extension.to_owned())),
        };
        Ok((source, table))
    }

    pub async fn preview_animal_import(
        &self,
        job: &Job,
        store: &dyn MuriArcStore,
    ) -> Result<PendingAnimalImport, DataError> {
        self.preview_animal_import_with_mapping(job, store, None)
            .await
    }

    pub async fn preview_animal_import_with_mapping(
        &self,
        job: &Job,
        store: &dyn MuriArcStore,
        mapping: Option<FieldMapping>,
    ) -> Result<PendingAnimalImport, DataError> {
        let (source, table) = self.read_upload_table(job.id).await?;
        let mapping = mapping.unwrap_or_else(|| FieldMapping::infer(&table.headers));
        let animals = store
            .list_animals(&AnimalFilter {
                lab_id: job.lab_id,
                ..AnimalFilter::default()
            })
            .await?;
        let animal_directory = AnimalDirectory::from_entries(
            animals
                .iter()
                .map(|animal| (animal.display_id.clone(), animal.id)),
        )
        .map_err(|error| DataError::Directory(error.to_string()))?;
        let mut preview = preview_animals_with_directory(&table, &mapping, &animal_directory);
        if preview.can_confirm() {
            let (existing_animals, cages, genetics) =
                animal_import_directories(store, job.lab_id).await?;
            let context = ImportPlanContext::new(
                job.lab_id,
                job.created_by,
                job.idempotency_key.clone(),
                source.sha256.clone(),
                Utc::now(),
            );
            if let Err(error) =
                build_animal_import_plan(&preview, &context, &existing_animals, &cages, &genetics)
            {
                preview.issues.extend(error.into_issues());
            }
        }
        let preview_hash = hash_json(&(
            WORKFLOW_SCHEMA_VERSION,
            source.sha256.as_str(),
            &mapping,
            &preview,
        ))?;
        let pending = PendingAnimalImport {
            schema_version: WORKFLOW_SCHEMA_VERSION,
            job_id: job.id,
            lab_id: job.lab_id,
            created_by: job.created_by,
            project_id: job.project_id,
            source,
            sheet_name: table.sheet_name,
            headers: table.headers,
            mapping,
            preview_hash,
            preview,
        };
        self.write_pending_import(&pending).await?;
        Ok(pending)
    }

    pub async fn preview_measurement_import(
        &self,
        job: &Job,
        experiment_id: Uuid,
        store: &dyn MuriArcStore,
    ) -> Result<PendingMeasurementImport, DataError> {
        self.preview_measurement_import_with_mapping(job, experiment_id, store, None)
            .await
    }

    pub async fn preview_measurement_import_with_mapping(
        &self,
        job: &Job,
        experiment_id: Uuid,
        store: &dyn MuriArcStore,
        mapping: Option<MeasurementFieldMapping>,
    ) -> Result<PendingMeasurementImport, DataError> {
        let project_id = job.project_id.ok_or_else(|| {
            DataError::Directory("measurement import requires an explicit project".to_owned())
        })?;
        let (source, table) = self.read_upload_table(job.id).await?;
        let mapping = mapping.unwrap_or_else(|| MeasurementFieldMapping::infer(&table.headers));
        let environment =
            measurement_import_environment(store, job.lab_id, project_id, experiment_id).await?;
        let mut preview =
            preview_measurements(&table, &mapping, &environment.animals, &environment.catalog);
        remove_existing_measurements(&mut preview, store, project_id, experiment_id).await?;
        let preview_hash = hash_json(&(
            WORKFLOW_SCHEMA_VERSION,
            ImportKind::Measurement,
            source.sha256.as_str(),
            experiment_id,
            environment.template_version_id,
            environment.template_revision,
            &mapping,
            &preview,
        ))?;
        let pending = PendingMeasurementImport {
            schema_version: WORKFLOW_SCHEMA_VERSION,
            job_id: job.id,
            lab_id: job.lab_id,
            created_by: job.created_by,
            project_id,
            experiment_id,
            template_version_id: environment.template_version_id,
            template_revision: environment.template_revision,
            source,
            sheet_name: table.sheet_name,
            headers: table.headers,
            mapping,
            measurement_labels: environment.measurement_labels,
            preview_hash,
            preview,
        };
        self.write_pending_measurement_import(&pending).await?;
        Ok(pending)
    }

    pub async fn read_pending_import(
        &self,
        job_id: Uuid,
    ) -> Result<PendingAnimalImport, DataError> {
        let pending: PendingAnimalImport = read_json(&self.pending_path(job_id)).await?;
        if pending.schema_version != WORKFLOW_SCHEMA_VERSION || pending.job_id != job_id {
            return Err(DataError::CorruptState("pending import identity/version"));
        }
        Ok(pending)
    }

    pub async fn read_pending_measurement_import(
        &self,
        job_id: Uuid,
    ) -> Result<PendingMeasurementImport, DataError> {
        let pending: PendingMeasurementImport = read_json(&self.pending_path(job_id)).await?;
        if pending.schema_version != WORKFLOW_SCHEMA_VERSION || pending.job_id != job_id {
            return Err(DataError::CorruptState(
                "pending measurement import identity/version",
            ));
        }
        Ok(pending)
    }

    async fn write_pending_import(&self, pending: &PendingAnimalImport) -> Result<(), DataError> {
        let path = self.pending_path(pending.job_id);
        match write_json_create_new(&path, pending).await {
            Ok(()) => Ok(()),
            Err(DataError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read_pending_import(pending.job_id).await?;
                if existing
                    .preview_hash
                    .eq_ignore_ascii_case(&pending.preview_hash)
                    && existing.lab_id == pending.lab_id
                    && existing.created_by == pending.created_by
                {
                    Ok(())
                } else {
                    Err(DataError::Conflict(
                        "pending import exists with different preview content".to_owned(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn write_pending_measurement_import(
        &self,
        pending: &PendingMeasurementImport,
    ) -> Result<(), DataError> {
        let path = self.pending_path(pending.job_id);
        match write_json_create_new(&path, pending).await {
            Ok(()) => Ok(()),
            Err(DataError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read_pending_measurement_import(pending.job_id).await?;
                if existing
                    .preview_hash
                    .eq_ignore_ascii_case(&pending.preview_hash)
                    && existing.lab_id == pending.lab_id
                    && existing.created_by == pending.created_by
                    && existing.project_id == pending.project_id
                    && existing.experiment_id == pending.experiment_id
                {
                    Ok(())
                } else {
                    Err(DataError::Conflict(
                        "pending measurement import exists with different preview content"
                            .to_owned(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub async fn clear_pending_import(&self, job_id: Uuid) -> Result<(), DataError> {
        remove_if_exists(self.pending_path(job_id)).await
    }

    pub async fn build_animal_import_plan(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        confirmed_at: DateTime<Utc>,
    ) -> Result<ImportPlan, DataError> {
        let pending = self.read_pending_import(job.id).await?;
        ensure_pending_scope(&pending, job, expected_preview_hash)?;
        if !pending.preview.can_confirm() {
            return Err(DataError::PreviewHasErrors);
        }
        let (existing_animals, cages, genetics) =
            animal_import_directories(store, job.lab_id).await?;
        let context = ImportPlanContext::new(
            job.lab_id,
            job.created_by,
            job.idempotency_key.clone(),
            pending.preview_hash,
            confirmed_at,
        );
        build_animal_import_plan(
            &pending.preview,
            &context,
            &existing_animals,
            &cages,
            &genetics,
        )
        .map_err(|error| DataError::Plan(error.into_issues()))
    }

    pub async fn confirm_animal_import(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        audit: &AuditContext,
        confirmed_at: DateTime<Utc>,
    ) -> Result<ImportCommitResult, DataError> {
        self.confirm_animal_import_with_options(
            job,
            expected_preview_hash,
            store,
            audit,
            confirmed_at,
            ImportConfirmOptions::default(),
        )
        .await
    }

    pub async fn confirm_animal_import_with_options(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        audit: &AuditContext,
        confirmed_at: DateTime<Utc>,
        options: ImportConfirmOptions,
    ) -> Result<ImportCommitResult, DataError> {
        ensure_ai_source_import_resolution(job, &options)?;
        let plan = self
            .build_animal_import_plan(job, expected_preview_hash, store, confirmed_at)
            .await?;
        store
            .commit_import(
                &plan,
                ImportCommitOptions {
                    cancellation_requested: job.cancellation_requested,
                    job_id: Some(job.id),
                    source_archive: options.source_archive,
                    ai_resolution: options.ai_resolution,
                },
                audit,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn build_measurement_import_plan(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        confirmed_at: DateTime<Utc>,
    ) -> Result<ImportPlan, DataError> {
        let pending = self.read_pending_measurement_import(job.id).await?;
        ensure_measurement_pending_scope(&pending, job, expected_preview_hash)?;
        if !pending.preview.can_confirm() {
            return Err(DataError::PreviewHasErrors);
        }
        let environment = measurement_import_environment(
            store,
            pending.lab_id,
            pending.project_id,
            pending.experiment_id,
        )
        .await?;
        if environment.template_version_id != pending.template_version_id
            || environment.template_revision != pending.template_revision
            || environment.measurement_labels != pending.measurement_labels
        {
            return Err(DataError::Conflict(
                "experiment measurement template changed after preview".to_owned(),
            ));
        }
        if pending
            .preview
            .accepted_rows
            .iter()
            .any(|row| !environment.animals.contains_id(row.animal_id))
        {
            return Err(DataError::Conflict(
                "experiment participation changed after preview".to_owned(),
            ));
        }
        let mut revalidated = pending.preview.clone();
        remove_existing_measurements(
            &mut revalidated,
            store,
            pending.project_id,
            pending.experiment_id,
        )
        .await?;
        if revalidated.accepted_rows.len() != pending.preview.accepted_rows.len() {
            return Err(DataError::Conflict(
                "a measurement from the reviewed preview now exists".to_owned(),
            ));
        }
        let context = MeasurementImportPlanContext::new(
            ImportPlanContext::new(
                job.lab_id,
                job.created_by,
                job.idempotency_key.clone(),
                pending.preview_hash,
                confirmed_at,
            ),
            pending.project_id,
            pending.experiment_id,
            pending.measurement_labels,
        );
        build_measurement_import_plan(&pending.preview, &context)
            .map_err(|error| DataError::Plan(error.into_issues()))
    }

    pub async fn confirm_measurement_import(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        audit: &AuditContext,
        confirmed_at: DateTime<Utc>,
    ) -> Result<ImportCommitResult, DataError> {
        self.confirm_measurement_import_with_options(
            job,
            expected_preview_hash,
            store,
            audit,
            confirmed_at,
            ImportConfirmOptions::default(),
        )
        .await
    }

    pub async fn confirm_measurement_import_with_options(
        &self,
        job: &Job,
        expected_preview_hash: &str,
        store: &dyn MuriArcStore,
        audit: &AuditContext,
        confirmed_at: DateTime<Utc>,
        options: ImportConfirmOptions,
    ) -> Result<ImportCommitResult, DataError> {
        ensure_ai_source_import_resolution(job, &options)?;
        let plan = self
            .build_measurement_import_plan(job, expected_preview_hash, store, confirmed_at)
            .await?;
        store
            .commit_import(
                &plan,
                ImportCommitOptions {
                    cancellation_requested: job.cancellation_requested,
                    job_id: Some(job.id),
                    source_archive: options.source_archive,
                    ai_resolution: options.ai_resolution,
                },
                audit,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn write_artifact(
        &self,
        metadata: &ArtifactMetadata,
        bytes: &[u8],
    ) -> Result<ArtifactWriteOutcome, DataError> {
        self.ensure_layout().await?;
        validate_artifact_metadata(metadata, bytes)?;
        let content_path = self.artifact_content_path(metadata.job_id);
        let metadata_path = self.artifact_metadata_path(metadata.job_id);
        if tokio::fs::try_exists(&content_path).await?
            || tokio::fs::try_exists(&metadata_path).await?
        {
            let existing = self.artifact_metadata(metadata.job_id).await?;
            let existing_bytes = tokio::fs::read(&content_path).await?;
            verify_content(
                &existing_bytes,
                existing.size_bytes,
                &existing.sha256,
                "artifact",
            )?;
            if existing == *metadata && existing.sha256.eq_ignore_ascii_case(&sha256_hex(bytes)) {
                return Ok(ArtifactWriteOutcome::Identical);
            }
            return Err(DataError::Conflict(
                "artifact already exists with different content".to_owned(),
            ));
        }
        let temp = self.root.join("artifacts").join(format!(
            "{}.{}.part",
            metadata.job_id,
            Uuid::new_v4()
        ));
        tokio::fs::write(&temp, bytes).await?;
        tokio::fs::rename(&temp, &content_path).await?;
        if let Err(error) = write_json_create_new(&metadata_path, metadata).await {
            let _ = tokio::fs::remove_file(&content_path).await;
            return Err(error);
        }
        Ok(ArtifactWriteOutcome::Stored)
    }

    pub async fn artifact_metadata(&self, job_id: Uuid) -> Result<ArtifactMetadata, DataError> {
        let metadata: ArtifactMetadata = read_json(&self.artifact_metadata_path(job_id)).await?;
        if metadata.schema_version != WORKFLOW_SCHEMA_VERSION || metadata.job_id != job_id {
            return Err(DataError::CorruptState("artifact identity/version"));
        }
        validate_fixed_file_name(&metadata.file_name)?;
        Ok(metadata)
    }

    pub async fn open_artifact(&self, job_id: Uuid) -> Result<OpenArtifact, DataError> {
        let metadata = self.artifact_metadata(job_id).await?;
        let mut file = tokio::fs::File::open(self.artifact_content_path(job_id)).await?;
        verify_open_file(&mut file, metadata.size_bytes, &metadata.sha256, "artifact").await?;
        Ok(OpenArtifact { metadata, file })
    }

    pub async fn read_artifact_bytes(
        &self,
        job_id: Uuid,
    ) -> Result<(ArtifactMetadata, Vec<u8>), DataError> {
        let metadata = self.artifact_metadata(job_id).await?;
        let bytes = tokio::fs::read(self.artifact_content_path(job_id)).await?;
        verify_content(&bytes, metadata.size_bytes, &metadata.sha256, "artifact")?;
        Ok((metadata, bytes))
    }

    pub async fn clear_artifact(&self, job_id: Uuid) -> Result<(), DataError> {
        remove_if_exists(self.artifact_content_path(job_id)).await?;
        remove_if_exists(self.artifact_metadata_path(job_id)).await?;
        Ok(())
    }

    fn upload_content_path(&self, job_id: Uuid) -> PathBuf {
        self.root.join("uploads").join(format!("{job_id}.bin"))
    }
    fn upload_metadata_path(&self, job_id: Uuid) -> PathBuf {
        self.root.join("uploads").join(format!("{job_id}.json"))
    }
    fn pending_path(&self, job_id: Uuid) -> PathBuf {
        self.root.join("pending").join(format!("{job_id}.json"))
    }
    fn artifact_content_path(&self, job_id: Uuid) -> PathBuf {
        self.root.join("artifacts").join(format!("{job_id}.bin"))
    }
    fn artifact_metadata_path(&self, job_id: Uuid) -> PathBuf {
        self.root.join("artifacts").join(format!("{job_id}.json"))
    }
}

fn ensure_ai_source_import_resolution(
    job: &Job,
    options: &ImportConfirmOptions,
) -> Result<(), DataError> {
    if is_ai_source_import_job(job)
        && (options.source_archive.is_none() || options.ai_resolution.is_none())
    {
        Err(DataError::Conflict(
            "AI source import requires its reviewed source archive and approval resolution"
                .to_owned(),
        ))
    } else {
        Ok(())
    }
}

async fn animal_import_directories(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
) -> Result<(AnimalDirectory, CageDirectory, GeneticDirectory), DataError> {
    let animals = store
        .list_animals(&AnimalFilter {
            lab_id,
            ..AnimalFilter::default()
        })
        .await?;
    let existing_animals = AnimalDirectory::from_entries(
        animals
            .iter()
            .map(|animal| (animal.display_id.clone(), animal.id)),
    )
    .map_err(|error| DataError::Directory(error.to_string()))?;
    let cages = CageDirectory::from_entries(
        store
            .list_cages(lab_id)
            .await?
            .into_iter()
            .map(|cage| (cage.section, cage.display_id, cage.id)),
    )
    .map_err(|error| DataError::Directory(error.to_string()))?;
    let loci = store.list_gene_loci(lab_id).await?;
    let mut alleles = Vec::new();
    for locus in &loci {
        alleles.extend(
            store
                .list_alleles(locus.id)
                .await?
                .into_iter()
                .map(|allele| (locus.id, allele.symbol, allele.id)),
        );
    }
    let definitions = store
        .list_genotype_definitions(lab_id)
        .await?
        .into_iter()
        .filter_map(|definition| {
            definition
                .components
                .into_iter()
                .map(|component| {
                    component
                        .allele_2_id
                        .map(|allele_2_id| (component.locus_id, component.allele_1_id, allele_2_id))
                })
                .collect::<Option<Vec<_>>>()
                .map(|components| (definition.id, components))
        })
        .collect::<Vec<_>>();
    let genetics = GeneticDirectory::from_entries_with_definitions(
        loci.into_iter().map(|locus| (locus.symbol, locus.id)),
        alleles,
        definitions,
    )
    .map_err(|error| DataError::Directory(error.to_string()))?;
    Ok((existing_animals, cages, genetics))
}

pub async fn export_animals(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    format: ExportFormat,
    filter: &AnimalExportFilter,
) -> Result<Vec<u8>, DataError> {
    export_animals_scoped(store, lab_id, None, format, filter).await
}

pub async fn export_animals_with_options(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    format: ExportFormat,
    options: &AnimalExportOptions,
) -> Result<Vec<u8>, DataError> {
    export_animals_scoped_with_options(store, lab_id, None, format, options).await
}

/// Exports the lab registry or only animals participating in one project.
///
/// Project scope is applied while reading from the Store rather than after
/// serialization so callers cannot accidentally export the whole lab and
/// attempt to filter the resulting artifact in an untrusted layer.
pub async fn export_animals_scoped(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    project_id: Option<Uuid>,
    format: ExportFormat,
    filter: &AnimalExportFilter,
) -> Result<Vec<u8>, DataError> {
    export_animals_scoped_with_options(
        store,
        lab_id,
        project_id,
        format,
        &AnimalExportOptions {
            filter: filter.clone(),
            ..Default::default()
        },
    )
    .await
}

pub async fn export_animals_scoped_with_options(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    project_id: Option<Uuid>,
    format: ExportFormat,
    options: &AnimalExportOptions,
) -> Result<Vec<u8>, DataError> {
    let records = collect_animal_export_records_scoped(store, lab_id, project_id).await?;
    match format {
        ExportFormat::Csv => {
            let mut bytes = Vec::new();
            export_animals_csv_with_options(&records, options, &mut bytes)?;
            Ok(bytes)
        }
        ExportFormat::Xlsx => {
            export_animals_xlsx_with_options(&records, options).map_err(Into::into)
        }
    }
}

pub async fn collect_animal_export_records(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
) -> Result<Vec<AnimalExportRecord>, DataError> {
    collect_animal_export_records_scoped(store, lab_id, None).await
}

pub async fn collect_animal_export_records_scoped(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Vec<AnimalExportRecord>, DataError> {
    let cages = store
        .list_cages(lab_id)
        .await?
        .into_iter()
        .map(|cage| (cage.id, cage))
        .collect::<BTreeMap<_, _>>();
    let projects = store
        .list_projects(lab_id)
        .await?
        .into_iter()
        .map(|project| (project.id, project.name))
        .collect::<BTreeMap<_, _>>();
    let mut animals = store
        .list_animals(&AnimalFilter {
            lab_id,
            project_id,
            cage_id: None,
            status: None,
            query: None,
        })
        .await?;
    animals.sort_by_key(|animal| animal.id);
    let mut records = Vec::with_capacity(animals.len());
    for animal in animals {
        let cage = animal
            .current_cage_id
            .and_then(|id| cages.get(&id))
            .map(|cage| ExportCage {
                display_id: cage.display_id.clone(),
                section: Some(cage.section.clone()),
                location: cage.location.clone(),
            });
        let (identifier_scope, scope_project_id) = match &animal.identifier_scope {
            IdentifierScope::Lab => ("lab".to_owned(), project_id),
            IdentifierScope::Project { project_id } => ("project".to_owned(), Some(*project_id)),
            IdentifierScope::Legacy { source } => (format!("legacy:{source}"), project_id),
        };
        let mut genotypes = Vec::new();
        for record in store.list_current_genotyping_records(animal.id).await? {
            let definition = store
                .get_genotype_definition(record.genotype_definition_id)
                .await?;
            let mut seen_loci = BTreeSet::new();
            let mut components = definition.components;
            components.sort_by_key(|component| (component.display_order, component.id));
            for component in components {
                if !seen_loci.insert(component.locus_id) {
                    return Err(DataError::Directory(format!(
                        "genotype definition {} repeats locus {}",
                        definition.id, component.locus_id
                    )));
                }
                let locus = store.get_gene_locus(component.locus_id).await?;
                let allele_1 = store.get_allele(component.allele_1_id).await?;
                let allele_2 = match component.allele_2_id {
                    Some(id) => Some(store.get_allele(id).await?.symbol),
                    None => None,
                };
                genotypes.push(ExportGenotype {
                    definition: definition.name.clone(),
                    state: match record.state {
                        GenotypingState::Unknown => ExportGenotypingState::Unknown,
                        GenotypingState::Expected => ExportGenotypingState::Expected,
                        GenotypingState::Confirmed => ExportGenotypingState::Confirmed,
                        GenotypingState::Rejected => ExportGenotypingState::Rejected,
                    },
                    assessed_at: record.assessed_at,
                    method: record.method.clone(),
                    locus: locus.symbol,
                    allele_1: allele_1.symbol,
                    allele_2,
                    component_mode: match component.mode {
                        GenotypeComponentMode::Diploid => "diploid",
                        GenotypeComponentMode::Hemizygous => "hemizygous",
                        GenotypeComponentMode::TransgenePresence => "transgene_presence",
                        GenotypeComponentMode::Conditional => "conditional",
                    }
                    .to_owned(),
                    display_order: component.display_order,
                });
            }
        }
        records.push(AnimalExportRecord {
            animal_id: animal.id,
            identifier_scope,
            project_name: scope_project_id.and_then(|id| projects.get(&id).cloned()),
            display_id: animal.display_id,
            sex: match animal.sex {
                Sex::Male => ExportSex::Male,
                Sex::Female => ExportSex::Female,
                Sex::Unknown => ExportSex::Unknown,
            },
            birth_date: animal.birth_date,
            registered_at: animal.meta.created_at,
            strain: animal.strain,
            status: match animal.current_status {
                AnimalStatus::Planned => ExportAnimalStatus::Planned,
                AnimalStatus::Alive => ExportAnimalStatus::Alive,
                AnimalStatus::InExperiment => ExportAnimalStatus::InExperiment,
                AnimalStatus::Sampled => ExportAnimalStatus::Sampled,
                AnimalStatus::Deceased => ExportAnimalStatus::Deceased,
                AnimalStatus::Euthanized => ExportAnimalStatus::Euthanized,
                AnimalStatus::Lost => ExportAnimalStatus::Lost,
                AnimalStatus::Archived => ExportAnimalStatus::Archived,
            },
            cage,
            genotypes,
        });
    }
    Ok(records)
}

pub async fn build_lab_snapshot(
    store: &dyn MuriArcStore,
    attachment_root: &Path,
    snapshot_id: Uuid,
    origin_instance_id: Uuid,
    lab_id: Uuid,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
) -> Result<Vec<u8>, DataError> {
    let lab = store.get_lab(lab_id).await?;
    let mut projects = store.list_projects(lab_id).await?;
    let mut cages = store.list_cages(lab_id).await?;
    let mut animals = store
        .list_animals(&AnimalFilter {
            lab_id,
            ..AnimalFilter::default()
        })
        .await?;
    let mut project_animal_assignments = store
        .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
            lab_id,
            project_id: None,
            animal_id: None,
        })
        .await?;
    projects.sort_by_key(|record| record.id);
    cages.sort_by_key(|record| record.id);
    animals.sort_by_key(|record| record.id);

    let mut events = Vec::new();
    let mut genotypes = Vec::new();
    let mut genotyping_records = Vec::new();
    let mut pedigrees = Vec::new();
    for animal in &animals {
        events.extend(store.list_animal_events(animal.id).await?);
        genotypes.extend(store.list_genotypes(animal.id).await?);
        genotyping_records.extend(store.list_genotyping_records(animal.id).await?);
        pedigrees.extend(store.list_pedigrees(animal.id).await?);
    }
    let mut loci = store.list_gene_loci(lab_id).await?;
    let mut alleles = Vec::new();
    for locus in &loci {
        alleles.extend(store.list_alleles(locus.id).await?);
    }
    let mut genotype_definitions = store.list_genotype_definitions(lab_id).await?;
    let mut breeding_lines = store.list_breeding_lines(lab_id).await?;
    let mut colonies = store.list_colonies(lab_id, None).await?;
    let mut breeding_pairs = store.list_breeding_pairs(lab_id, None).await?;
    let mut mating_events = Vec::new();
    let mut litters = Vec::new();
    for pair in &breeding_pairs {
        mating_events.extend(store.list_mating_events(pair.id).await?);
        litters.extend(store.list_litters(pair.id).await?);
    }
    let mut animal_drafts = Vec::new();
    for litter in &litters {
        animal_drafts.extend(store.list_animal_drafts(litter.id).await?);
    }
    let mut templates = store.list_template_versions(lab_id, None).await?;
    let mut experiments = Vec::new();
    let mut cohorts = Vec::new();
    let mut participations = Vec::new();
    let mut procedures = Vec::new();
    let mut measurements = Vec::new();
    let mut samples = Vec::new();
    let mut experiment_events = Vec::new();
    let mut observation_definitions = Vec::new();
    let mut observations = Vec::new();
    let mut observation_values = Vec::new();
    for project in &projects {
        let project_experiments = store
            .list_experiments(&ExperimentFilter {
                project_id: project.id,
                status: None,
            })
            .await?;
        for experiment in &project_experiments {
            cohorts.extend(store.list_cohorts(experiment.id).await?);
            procedures.extend(store.list_procedures(experiment.id, None).await?);
            experiment_events.extend(store.list_experiment_events(experiment.id).await?);
            observation_definitions
                .extend(store.list_observation_definitions(experiment.id).await?);
            let experiment_observations = store
                .list_observations(&ObservationFilter {
                    experiment_id: experiment.id,
                    experiment_event_id: None,
                    subject_type: None,
                    subject_id: None,
                })
                .await?;
            for observation in &experiment_observations {
                observation_values.extend(store.list_observation_values(observation.id).await?);
            }
            observations.extend(experiment_observations);
        }
        experiments.extend(project_experiments);
        participations.extend(
            store
                .list_participations(&ParticipationFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: None,
                    cohort_id: None,
                })
                .await?,
        );
        measurements.extend(
            store
                .list_measurements(&MeasurementFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: None,
                })
                .await?,
        );
        samples.extend(
            store
                .list_samples(&SampleFilter {
                    project_id: project.id,
                    experiment_id: None,
                    animal_id: None,
                })
                .await?,
        );
    }
    let mut attachments = store.list_lab_attachments(lab_id).await?;
    let mut audits = store
        .list_audit_entries(&AuditFilter {
            lab_id,
            project_id: None,
            entity_id: None,
        })
        .await?;
    let mut provenance = store
        .list_provenance(&ProvenanceFilter {
            lab_id,
            ..ProvenanceFilter::default()
        })
        .await?;
    let staged_ai_attachment_ids = attachments
        .iter()
        .filter(|attachment| {
            attachment.project_id.is_none()
                && is_ai_managed_attachment_entity_type(&attachment.entity_type)
        })
        .map(|attachment| attachment.id)
        .collect::<BTreeSet<_>>();
    let formal_ai_attachment_ids = attachments
        .iter()
        .filter(|attachment| {
            attachment.project_id.is_some()
                && is_ai_managed_attachment_entity_type(&attachment.entity_type)
        })
        .map(|attachment| attachment.id)
        .collect::<BTreeSet<_>>();
    attachments.retain(|attachment| !staged_ai_attachment_ids.contains(&attachment.id));
    let mut excluded_entities = audits
        .iter()
        .filter(|entry| {
            is_ai_operational_or_configuration_entity_type(entry.entity_type)
                || (entry.entity_type == EntityType::Attachment
                    && staged_ai_attachment_ids.contains(&entry.entity_id))
                || (is_private_ai_source_attachment_audit(entry)
                    && !formal_ai_attachment_ids.contains(&entry.entity_id))
                || is_private_ai_source_job_audit(entry)
        })
        .map(|entry| (entry.entity_type.as_str(), entry.entity_id))
        .collect::<BTreeSet<_>>();
    excluded_entities.extend(
        staged_ai_attachment_ids
            .iter()
            .map(|id| (EntityType::Attachment.as_str(), *id)),
    );
    audits.retain(|entry| {
        !excluded_entities.contains(&(entry.entity_type.as_str(), entry.entity_id))
    });
    provenance.retain(|record| {
        !is_ai_operational_or_configuration_entity_type(record.entity_type)
            && !excluded_entities.contains(&(record.entity_type.as_str(), record.entity_id))
    });
    sort_by_id(&mut events, |value| value.id);
    sort_by_id(&mut project_animal_assignments, |value| value.id);
    sort_by_id(&mut loci, |value| value.id);
    sort_by_id(&mut alleles, |value| value.id);
    sort_by_id(&mut genotypes, |value| value.id);
    sort_by_id(&mut genotype_definitions, |value| value.id);
    sort_by_id(&mut genotyping_records, |value| value.id);
    sort_by_id(&mut breeding_lines, |value| value.id);
    sort_by_id(&mut colonies, |value| value.id);
    sort_by_id(&mut breeding_pairs, |value| value.id);
    sort_by_id(&mut mating_events, |value| value.id);
    sort_by_id(&mut litters, |value| value.id);
    sort_by_id(&mut animal_drafts, |value| value.id);
    sort_by_id(&mut pedigrees, |value| value.id);
    sort_by_id(&mut templates, |value| value.id);
    sort_by_id(&mut experiments, |value| value.id);
    sort_by_id(&mut cohorts, |value| value.id);
    sort_by_id(&mut participations, |value| value.id);
    sort_by_id(&mut procedures, |value| value.id);
    sort_by_id(&mut measurements, |value| value.id);
    sort_by_id(&mut samples, |value| value.id);
    sort_by_id(&mut experiment_events, |value| value.id);
    sort_by_id(&mut observation_definitions, |value| value.id);
    sort_by_id(&mut observations, |value| value.id);
    sort_by_id(&mut observation_values, |value| value.id);
    sort_by_id(&mut attachments, |value| value.id);
    sort_by_id(&mut audits, |value| value.id);
    sort_by_id(&mut provenance, |value| value.id);

    let mut entries = vec![jsonl_entry("lab", std::slice::from_ref(&lab))?];
    entries.extend([
        jsonl_entry("project", &projects)?,
        jsonl_entry("cage", &cages)?,
        jsonl_entry("animal", &animals)?,
        jsonl_entry("project_animal_assignment", &project_animal_assignments)?,
        jsonl_entry("animal_event", &events)?,
        jsonl_entry("gene_locus", &loci)?,
        jsonl_entry("allele", &alleles)?,
        jsonl_entry("genotype", &genotypes)?,
        jsonl_entry("genotype_definition", &genotype_definitions)?,
        jsonl_entry("genotyping_record", &genotyping_records)?,
        jsonl_entry("breeding_line", &breeding_lines)?,
        jsonl_entry("colony", &colonies)?,
        jsonl_entry("breeding_pair", &breeding_pairs)?,
        jsonl_entry("mating_event", &mating_events)?,
        jsonl_entry("litter", &litters)?,
        jsonl_entry("animal_draft", &animal_drafts)?,
        jsonl_entry("pedigree", &pedigrees)?,
        jsonl_entry("experiment_template_version", &templates)?,
        jsonl_entry("experiment", &experiments)?,
        jsonl_entry("cohort", &cohorts)?,
        jsonl_entry("participation", &participations)?,
        jsonl_entry("procedure", &procedures)?,
        jsonl_entry("measurement", &measurements)?,
        jsonl_entry("sample", &samples)?,
        jsonl_entry("experiment_event", &experiment_events)?,
        jsonl_entry("observation_definition", &observation_definitions)?,
        jsonl_entry("observation", &observations)?,
        jsonl_entry("observation_value", &observation_values)?,
        jsonl_entry("attachment", &attachments)?,
        jsonl_entry("audit", &audits)?,
        jsonl_entry("provenance", &provenance)?,
    ]);
    for attachment in &attachments {
        entries.push(read_attachment_entry(attachment_root, attachment).await?);
    }

    let mut manifest = SnapshotManifest::new(origin_instance_id, lab_id, created_by, created_at);
    manifest.snapshot_id = snapshot_id;
    manifest.project_ids = projects.iter().map(|project| project.id).collect();
    let cursor = write_bundle(Cursor::new(Vec::new()), manifest, entries)?;
    Ok(cursor.into_inner())
}

fn jsonl_entry<T: Serialize>(entity_type: &str, records: &[T]) -> Result<BundleEntry, DataError> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, record)?;
        bytes.push(b'\n');
    }
    Ok(BundleEntry {
        path: format!("data/{entity_type}.jsonl"),
        kind: EntryKind::JsonLines,
        entity_type: Some(entity_type.to_owned()),
        record_count: Some(records.len() as u64),
        bytes,
    })
}

async fn read_attachment_entry(
    root: &Path,
    attachment: &Attachment,
) -> Result<BundleEntry, DataError> {
    let bytes = AttachmentFiles::new(root)
        .read_verified_bytes(attachment)
        .await
        .map_err(|error| {
            DataError::Attachment(format!(
                "attachment {} is unavailable or invalid: {error}",
                attachment.id
            ))
        })?;
    Ok(BundleEntry {
        path: format!(
            "attachments/{}/v{}/content",
            attachment.id, attachment.version
        ),
        kind: EntryKind::Attachment,
        entity_type: Some("attachment_content".to_owned()),
        record_count: None,
        bytes,
    })
}

fn sort_by_id<T>(values: &mut [T], id: impl Fn(&T) -> Uuid) {
    values.sort_by_key(id);
}

fn ensure_pending_scope(
    pending: &PendingAnimalImport,
    job: &Job,
    expected_preview_hash: &str,
) -> Result<(), DataError> {
    if pending.job_id != job.id
        || pending.lab_id != job.lab_id
        || pending.created_by != job.created_by
        || pending.project_id != job.project_id
    {
        return Err(DataError::ScopeMismatch);
    }
    if !pending
        .preview_hash
        .eq_ignore_ascii_case(expected_preview_hash.trim())
    {
        return Err(DataError::Conflict(
            "confirmed preview hash does not match the reviewed preview".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_measurement_pending_scope(
    pending: &PendingMeasurementImport,
    job: &Job,
    expected_preview_hash: &str,
) -> Result<(), DataError> {
    if pending.job_id != job.id
        || pending.lab_id != job.lab_id
        || pending.created_by != job.created_by
        || Some(pending.project_id) != job.project_id
    {
        return Err(DataError::ScopeMismatch);
    }
    if !pending
        .preview_hash
        .eq_ignore_ascii_case(expected_preview_hash.trim())
    {
        return Err(DataError::Conflict(
            "confirmed preview hash does not match the reviewed preview".to_owned(),
        ));
    }
    Ok(())
}

struct MeasurementImportEnvironment {
    animals: AnimalDirectory,
    catalog: MeasurementCatalog,
    measurement_labels: BTreeMap<String, String>,
    template_version_id: Uuid,
    template_revision: i64,
}

async fn measurement_import_environment(
    store: &dyn MuriArcStore,
    lab_id: Uuid,
    project_id: Uuid,
    experiment_id: Uuid,
) -> Result<MeasurementImportEnvironment, DataError> {
    let experiment = store.get_experiment(experiment_id).await?;
    if experiment.lab_id != lab_id || experiment.project_id != project_id {
        return Err(DataError::ScopeMismatch);
    }
    let template_version_id = experiment.template_version_id.ok_or_else(|| {
        DataError::Directory("selected experiment has no published measurement template".to_owned())
    })?;
    let template = store.get_template_version(template_version_id).await?;
    if template.lab_id != lab_id || template.status != TemplateStatus::Published {
        return Err(DataError::Directory(
            "selected experiment measurement template is not published in this lab".to_owned(),
        ));
    }
    if template.fields.is_empty() {
        return Err(DataError::Directory(
            "selected experiment template defines no measurement fields".to_owned(),
        ));
    }
    let definitions = template
        .fields
        .iter()
        .map(|field| {
            MeasurementDefinition::new(
                field.key.clone(),
                match field.value_type {
                    FieldValueType::Number => MeasurementValueType::Number,
                    FieldValueType::Text => MeasurementValueType::Text,
                    FieldValueType::Boolean => MeasurementValueType::Boolean,
                    FieldValueType::Date => MeasurementValueType::Date,
                    FieldValueType::Category => MeasurementValueType::Category,
                },
                field.unit.iter().cloned(),
                field.unit.is_some(),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| DataError::Directory(error.to_string()))?;
    let catalog = MeasurementCatalog::new(definitions)
        .map_err(|error| DataError::Directory(error.to_string()))?;
    let measurement_labels = template
        .fields
        .iter()
        .map(|field| (field.key.clone(), field.label.clone()))
        .collect();
    let participations = store
        .list_participations(&ParticipationFilter {
            project_id,
            experiment_id: Some(experiment_id),
            animal_id: None,
            cohort_id: None,
        })
        .await?;
    let mut entries = Vec::with_capacity(participations.len());
    for participation in participations {
        let animal = store.get_animal(participation.animal_id).await?;
        if animal.lab_id != lab_id {
            return Err(DataError::ScopeMismatch);
        }
        entries.push((animal.display_id, animal.id));
    }
    let animals = AnimalDirectory::from_entries(entries)
        .map_err(|error| DataError::Directory(error.to_string()))?;
    Ok(MeasurementImportEnvironment {
        animals,
        catalog,
        measurement_labels,
        template_version_id,
        template_revision: template.meta.revision,
    })
}

async fn remove_existing_measurements(
    preview: &mut MeasurementImportPreview,
    store: &dyn MuriArcStore,
    project_id: Uuid,
    experiment_id: Uuid,
) -> Result<(), DataError> {
    let existing = store
        .list_measurements(&MeasurementFilter {
            project_id,
            experiment_id: Some(experiment_id),
            animal_id: None,
        })
        .await?
        .into_iter()
        .map(|measurement| {
            (
                measurement.animal_id,
                measurement.key.trim().to_owned(),
                measurement.measured_at,
            )
        })
        .collect::<BTreeSet<_>>();
    let rows = std::mem::take(&mut preview.accepted_rows);
    for row in rows {
        let identity = (
            row.animal_id,
            row.measurement_key.trim().to_owned(),
            row.measured_at,
        );
        if existing.contains(&identity) {
            preview.issues.push(ImportIssue {
                row: Some(row.source_row),
                field: Some("measurement_key".to_owned()),
                severity: IssueSeverity::Error,
                code: "existing_measurement".to_owned(),
                message: "该动物、指标和时间的测量已存在".to_owned(),
            });
        } else {
            preview.accepted_rows.push(row);
        }
    }
    Ok(())
}

fn validate_upload_name(value: &str) -> Result<(String, String), DataError> {
    let file_name = Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(DataError::InvalidFileName)?;
    if file_name != value
        || file_name.chars().count() > 255
        || file_name.chars().any(char::is_control)
    {
        return Err(DataError::InvalidFileName);
    }
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(DataError::InvalidFileName)?;
    if !matches!(extension.as_str(), "csv" | "xlsx") {
        return Err(DataError::UnsupportedUpload(extension));
    }
    Ok((file_name.to_owned(), extension))
}

fn validate_fixed_file_name(value: &str) -> Result<(), DataError> {
    if value.is_empty()
        || value.chars().count() > 160
        || value.starts_with('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(DataError::InvalidFileName)
    } else {
        Ok(())
    }
}

fn validate_artifact_metadata(metadata: &ArtifactMetadata, bytes: &[u8]) -> Result<(), DataError> {
    if metadata.schema_version != WORKFLOW_SCHEMA_VERSION || metadata.job_id.is_nil() {
        return Err(DataError::CorruptState("artifact identity/version"));
    }
    validate_fixed_file_name(&metadata.file_name)?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(DataError::ArtifactTooLarge(MAX_ARTIFACT_BYTES));
    }
    verify_content(bytes, metadata.size_bytes, &metadata.sha256, "artifact")
}

pub fn artifact_metadata(
    job_id: Uuid,
    kind: ArtifactKind,
    file_name: String,
    media_type: String,
    bytes: &[u8],
    created_at: DateTime<Utc>,
) -> Result<ArtifactMetadata, DataError> {
    let metadata = ArtifactMetadata {
        schema_version: WORKFLOW_SCHEMA_VERSION,
        job_id,
        kind,
        file_name,
        media_type,
        size_bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        created_at,
    };
    validate_artifact_metadata(&metadata, bytes)?;
    Ok(metadata)
}

fn verify_content(
    bytes: &[u8],
    expected_size: u64,
    expected_sha: &str,
    label: &'static str,
) -> Result<(), DataError> {
    if bytes.len() as u64 != expected_size {
        return Err(DataError::ChecksumMismatch(label));
    }
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected_sha) {
        return Err(DataError::ChecksumMismatch(label));
    }
    Ok(())
}

async fn verify_open_file(
    file: &mut tokio::fs::File,
    expected_size: u64,
    expected_sha: &str,
    label: &'static str,
) -> Result<(), DataError> {
    if file.metadata().await?.len() != expected_size {
        return Err(DataError::ChecksumMismatch(label));
    }

    file.seek(SeekFrom::Start(0)).await?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(DataError::ChecksumMismatch(label))?;
        hasher.update(&buffer[..read]);
    }
    let actual_sha = format!("{:x}", hasher.finalize());
    if total != expected_size || !actual_sha.eq_ignore_ascii_case(expected_sha) {
        return Err(DataError::ChecksumMismatch(label));
    }

    file.seek(SeekFrom::Start(0)).await?;
    Ok(())
}

fn hash_json(value: &impl Serialize) -> Result<String, DataError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn parse_uuid_file(value: &str, label: &'static str) -> Result<Uuid, DataError> {
    Uuid::parse_str(value.trim()).map_err(|_| DataError::CorruptState(label))
}

async fn write_json_create_new(path: &Path, value: &impl Serialize) -> Result<(), DataError> {
    let bytes = serde_json::to_vec(value)?;
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    Ok(())
}

async fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, DataError> {
    let bytes = tokio::fs::read(path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            DataError::NotFound
        } else {
            DataError::Io(error)
        }
    })?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

async fn remove_if_exists(path: PathBuf) -> Result<(), DataError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Error)]
pub enum DataError {
    #[error("data object was not found")]
    NotFound,
    #[error("invalid file name")]
    InvalidFileName,
    #[error("empty upload is not allowed")]
    EmptyUpload,
    #[error("unsupported upload extension: {0}")]
    UnsupportedUpload(String),
    #[error("upload exceeds the {0}-byte limit")]
    UploadTooLarge(u64),
    #[error("artifact exceeds the {0}-byte limit")]
    ArtifactTooLarge(u64),
    #[error("data conflict: {0}")]
    Conflict(String),
    #[error("pending import scope does not match its job")]
    ScopeMismatch,
    #[error("preview contains blocking errors")]
    PreviewHasErrors,
    #[error("import plan contains blocking issues")]
    Plan(Vec<ImportIssue>),
    #[error("import directory is invalid: {0}")]
    Directory(String),
    #[error("{0} checksum or size mismatch")]
    ChecksumMismatch(&'static str),
    #[error("attachment error: {0}")]
    Attachment(String),
    #[error("corrupt persisted workflow state: {0}")]
    CorruptState(&'static str),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Import(#[from] ImportError),
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use muriarc_core::{
        Actor, ActorType, AiConversation, AiConversationMessage, AiConversationMessageRole,
        AiConversationSource, AiConversationSourceKind, AiConversationSourceStatus,
        AiOperationStore, Allele, Animal, AnimalDraft, Approval, ApprovalDecision, AuditContext,
        BreedingLine, BreedingMemberRole, BreedingPair, BreedingPairMember, Colony, Experiment,
        ExperimentEvent, ExperimentTemplateVersion, FieldValueType, GeneLocus, GenotypeComponent,
        GenotypeComponentMode, GenotypeDefinition, GenotypingRecord, GenotypingState, Lab, Litter,
        MatingEvent, MeasurementFilter, MuriArcStore, Observation, ObservationDefinition,
        ObservationPolicy, ObservationSubjectType, ObservationValueData, ObservationValueRecord,
        ObservationValueType, Participation, Project, RecordMeta, RecordStatus, Sex, TemplateField,
        ToolRun, ToolRunStatus, User, WorkspaceStore, WriteSource,
    };
    use muriarc_snapshot::verify_bundle;
    use muriarc_store_sqlite::SqliteStore;
    use std::io::{Cursor, Read};
    use tempfile::tempdir;

    use super::*;

    async fn create_ai_source_fixture(
        store: &SqliteStore,
        files: &AttachmentFiles,
        conversation: &AiConversation,
        file_name: &str,
        bytes: &[u8],
        now: DateTime<Utc>,
    ) -> (AiConversationSource, Attachment) {
        let attachment_id = Uuid::new_v4();
        let object = files.write_bytes(attachment_id, bytes).await.unwrap();
        let source_id = Uuid::new_v4();
        let attachment = Attachment {
            id: attachment_id,
            lab_id: conversation.lab_id,
            project_id: None,
            entity_type: "ai_conversation_source".to_owned(),
            entity_id: source_id,
            file_name: file_name.to_owned(),
            media_type: Some("text/plain".to_owned()),
            relative_path: object.relative_path,
            size_bytes: object.size_bytes,
            sha256: object.sha256,
            version: 1,
            meta: RecordMeta::new(now),
        };
        let source = AiConversationSource {
            id: source_id,
            lab_id: conversation.lab_id,
            user_id: conversation.user_id,
            conversation_id: Some(conversation.id),
            project_id: conversation.project_id,
            attachment_id,
            kind: AiConversationSourceKind::Text,
            status: AiConversationSourceStatus::Ready,
            last_activity_at: now,
            expires_at: now + chrono::Duration::days(7),
            archived_at: None,
            error_code: None,
            meta: RecordMeta::new(now),
        };
        store
            .create_ai_conversation_source(
                &attachment,
                &source,
                &AuditContext::system(WriteSource::Migration),
            )
            .await
            .unwrap();
        (source, attachment)
    }

    fn read_snapshot_entries(snapshot: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = zip::ZipArchive::new(Cursor::new(snapshot)).unwrap();
        let mut entries = BTreeMap::new();
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let path = file.name().to_owned();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            entries.insert(path, bytes);
        }
        entries
    }

    fn snapshot_contains(entries: &BTreeMap<String, Vec<u8>>, needle: &str) -> bool {
        let needle = needle.as_bytes();
        entries
            .values()
            .any(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
    }

    #[tokio::test]
    async fn upload_preview_export_and_snapshot_are_real_and_checksummed() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let lab = Lab::new("Data lab", now).unwrap();
        let audit = AuditContext::system(WriteSource::Migration);
        store.create_lab(&lab, &audit).await.unwrap();
        let existing = Animal::new_mouse(lab.id, "M-existing", Sex::Female, now).unwrap();
        store.create_animal(&existing, &audit).await.unwrap();

        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path().join("data"));
        let job = Job {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: None,
            created_by: Uuid::new_v4(),
            kind: muriarc_core::JobKind::Import,
            status: muriarc_core::JobStatus::Queued,
            idempotency_key: "test-import".to_owned(),
            progress_current: 0,
            progress_total: None,
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: muriarc_core::RecordMeta::new(now),
        };
        files
            .write_upload(
                job.id,
                "animals.csv",
                Cursor::new(b"display_id,sex\nM-new,male\n".to_vec()),
            )
            .await
            .unwrap();
        let pending = files.preview_animal_import(&job, &store).await.unwrap();
        assert!(pending.preview.can_confirm());
        assert_eq!(pending.preview.accepted_rows.len(), 1);
        let plan = files
            .build_animal_import_plan(&job, &pending.preview_hash, &store, now)
            .await
            .unwrap();
        assert_eq!(plan.animals.len(), 1);

        let csv = export_animals(
            &store,
            lab.id,
            ExportFormat::Csv,
            &AnimalExportFilter::default(),
        )
        .await
        .unwrap();
        assert!(String::from_utf8(csv).unwrap().contains("M-existing"));

        let snapshot = build_lab_snapshot(
            &store,
            temp.path(),
            Uuid::new_v4(),
            files.instance_id().await.unwrap(),
            lab.id,
            Some(job.created_by),
            now,
        )
        .await
        .unwrap();
        let manifest = verify_bundle(Cursor::new(snapshot)).unwrap();
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.path == "data/animal.jsonl")
        );
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.path == "data/provenance.jsonl")
        );
    }

    #[tokio::test]
    async fn snapshot_excludes_private_ai_operations_but_keeps_archived_source_attachment() {
        const CHAT_SENTINEL: &str = "OWNER_CHAT_SENTINEL";
        const TOOL_SENTINEL: &str = "TOOL_OUTPUT_SENTINEL";
        const APPROVAL_SENTINEL: &str = "APPROVAL_STATEMENT_SENTINEL";
        const STAGED_BYTES: &[u8] = b"STAGED_SOURCE_BYTES_SENTINEL";
        const ARCHIVED_BYTES: &[u8] = b"ARCHIVED_SOURCE_BYTES_SENTINEL";

        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let migration = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Private AI snapshot lab", now).unwrap();
        store.create_lab(&lab, &migration).await.unwrap();
        let user = User::new(
            lab.id,
            "snapshot-ai-owner@example.test",
            "Snapshot AI owner",
            now,
        )
        .unwrap();
        store.create_user(&user, &migration).await.unwrap();
        let project = Project::new(lab.id, "Formal archive project", now).unwrap();
        store.create_project(&project, &migration).await.unwrap();

        let ai_audit = AuditContext {
            actor: Actor {
                actor_type: ActorType::Ai,
                user_id: Some(user.id),
                display_name: "MuriArc AI".to_owned(),
            },
            source: WriteSource::Ai,
            request_id: Some("snapshot-private-ai-turn".to_owned()),
            reason: None,
        };
        let conversation = AiConversation {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            title: "Owner-private conversation".to_owned(),
            pinned_at: None,
            archived_at: None,
            meta: RecordMeta::new(now),
        };
        store
            .create_ai_conversation(&conversation, &ai_audit)
            .await
            .unwrap();
        let user_message = AiConversationMessage::new(
            conversation.id,
            lab.id,
            Some(project.id),
            user.id,
            1,
            AiConversationMessageRole::User,
            CHAT_SENTINEL,
            None,
            now + chrono::Duration::milliseconds(1),
        )
        .unwrap();
        let assistant_message = AiConversationMessage::new(
            conversation.id,
            lab.id,
            Some(project.id),
            user.id,
            2,
            AiConversationMessageRole::Assistant,
            "Safe assistant response",
            Some(serde_json::json!({"content": "Safe assistant response"})),
            now + chrono::Duration::milliseconds(2),
        )
        .unwrap();
        let tool_run = ToolRun {
            id: Uuid::new_v4(),
            conversation_id: Some(conversation.id),
            lab_id: lab.id,
            project_id: Some(project.id),
            user_id: user.id,
            tool_name: "private_snapshot_probe".to_owned(),
            input: serde_json::json!({"operation": "read"}),
            output: Some(serde_json::json!({"private": TOOL_SENTINEL})),
            status: ToolRunStatus::AwaitingApproval,
            source: WriteSource::Ai,
            started_at: Some(now + chrono::Duration::milliseconds(1)),
            completed_at: None,
            error: None,
            meta: RecordMeta::new(now + chrono::Duration::milliseconds(1)),
        };
        let approval = Approval {
            id: Uuid::new_v4(),
            tool_run_id: tool_run.id,
            requested_diff: serde_json::json!({"statement": APPROVAL_SENTINEL}),
            decision: ApprovalDecision::Pending,
            decided_by: None,
            decided_at: None,
            reason: None,
            meta: RecordMeta::new(now + chrono::Duration::milliseconds(1)),
        };
        store
            .append_ai_turn_records(
                &user_message,
                &assistant_message,
                std::slice::from_ref(&tool_run),
                std::slice::from_ref(&approval),
                0,
                &ai_audit,
            )
            .await
            .unwrap();

        let temp = tempdir().unwrap();
        let attachment_root = temp.path().join("attachments");
        let files = AttachmentFiles::new(&attachment_root);
        let (staged_source, staged_attachment) = create_ai_source_fixture(
            &store,
            &files,
            &conversation,
            "staged-private.txt",
            STAGED_BYTES,
            now,
        )
        .await;
        let (archived_source, archived_attachment) = create_ai_source_fixture(
            &store,
            &files,
            &conversation,
            "archived-formal.txt",
            ARCHIVED_BYTES,
            now,
        )
        .await;
        store
            .archive_ai_conversation_source(
                archived_source.id,
                project.id,
                archived_source.meta.revision,
                now + chrono::Duration::seconds(1),
                &AuditContext::system(WriteSource::Desktop),
            )
            .await
            .unwrap();

        let snapshot = build_lab_snapshot(
            &store,
            &attachment_root,
            Uuid::new_v4(),
            Uuid::new_v4(),
            lab.id,
            Some(user.id),
            now + chrono::Duration::seconds(2),
        )
        .await
        .unwrap();
        verify_bundle(Cursor::new(snapshot.as_slice())).unwrap();
        let entries = read_snapshot_entries(&snapshot);

        for sentinel in [CHAT_SENTINEL, TOOL_SENTINEL, APPROVAL_SENTINEL] {
            assert!(
                !snapshot_contains(&entries, sentinel),
                "{sentinel} leaked into the shared snapshot"
            );
        }
        assert!(!snapshot_contains(
            &entries,
            std::str::from_utf8(STAGED_BYTES).unwrap()
        ));
        assert!(!snapshot_contains(
            &entries,
            &staged_attachment.relative_path
        ));
        assert!(!snapshot_contains(&entries, &staged_source.id.to_string()));
        assert!(!snapshot_contains(
            &entries,
            &staged_attachment.id.to_string()
        ));

        let archived_content_path = format!(
            "attachments/{}/v{}/content",
            archived_attachment.id, archived_attachment.version
        );
        assert_eq!(
            entries.get(&archived_content_path).map(Vec::as_slice),
            Some(ARCHIVED_BYTES)
        );
        let attachment_jsonl = String::from_utf8(entries["data/attachment.jsonl"].clone()).unwrap();
        assert!(attachment_jsonl.contains(&archived_attachment.id.to_string()));
        assert!(attachment_jsonl.contains("archived-formal.txt"));
        assert!(!attachment_jsonl.contains(&staged_attachment.id.to_string()));

        let audit_jsonl = String::from_utf8(entries["data/audit.jsonl"].clone()).unwrap();
        assert!(!audit_jsonl.contains(&conversation.id.to_string()));
        assert!(!audit_jsonl.contains(&user_message.id.to_string()));
        assert!(!audit_jsonl.contains(&tool_run.id.to_string()));
        assert!(!audit_jsonl.contains(&approval.id.to_string()));

        let provenance_jsonl = String::from_utf8(entries["data/provenance.jsonl"].clone()).unwrap();
        assert!(provenance_jsonl.contains(&archived_attachment.id.to_string()));
        assert!(!provenance_jsonl.contains(&archived_source.id.to_string()));
        assert!(!provenance_jsonl.contains(&staged_source.id.to_string()));
        assert!(!provenance_jsonl.contains(&staged_attachment.id.to_string()));
    }

    #[tokio::test]
    async fn animal_preview_surfaces_plan_level_directory_errors_before_confirmation() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let lab = Lab::new("Plan preview lab", now).unwrap();
        let audit = AuditContext::system(WriteSource::Migration);
        store.create_lab(&lab, &audit).await.unwrap();
        let files_root = tempdir().unwrap();
        let files = DataFiles::new(files_root.path().join("data"));
        let job = Job {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: None,
            created_by: Uuid::new_v4(),
            kind: muriarc_core::JobKind::Import,
            status: muriarc_core::JobStatus::Queued,
            idempotency_key: "plan-preview".to_owned(),
            progress_current: 0,
            progress_total: None,
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: RecordMeta::new(now),
        };
        files
            .write_upload(
                job.id,
                "animals.csv",
                Cursor::new(
                    b"display_id,cage,genotype\nM-CAGE,C404,\nM-GENE,,{Missing}[+]/[+]\n".to_vec(),
                ),
            )
            .await
            .unwrap();

        let pending = files.preview_animal_import(&job, &store).await.unwrap();
        let codes = pending
            .preview
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("unknown_cage"));
        assert!(codes.contains("unknown_locus"));
        assert!(!pending.preview.can_confirm());
        assert!(matches!(
            files
                .build_animal_import_plan(&job, &pending.preview_hash, &store, now)
                .await,
            Err(DataError::PreviewHasErrors)
        ));
    }

    #[tokio::test]
    async fn business_export_flattens_every_current_definition_component_without_uuid() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let lab = Lab::new("Multi locus export lab", now).unwrap();
        let audit = AuditContext::system(WriteSource::Migration);
        store.create_lab(&lab, &audit).await.unwrap();
        let animal = Animal::new_mouse(lab.id, "M-THREE", Sex::Female, now).unwrap();
        store.create_animal(&animal, &audit).await.unwrap();

        let mut components = Vec::new();
        for (display_order, symbol) in ["Trp53", "Rosa26", "Cre"].into_iter().enumerate() {
            let locus = GeneLocus {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                symbol: symbol.to_owned(),
                description: None,
                meta: RecordMeta::new(now),
            };
            store.create_gene_locus(&locus, &audit).await.unwrap();
            let first = Allele {
                id: Uuid::new_v4(),
                locus_id: locus.id,
                symbol: "+".to_owned(),
                description: None,
                is_wild_type: true,
                meta: RecordMeta::new(now),
            };
            let second = Allele {
                id: Uuid::new_v4(),
                locus_id: locus.id,
                symbol: format!("{symbol}-target"),
                description: None,
                is_wild_type: false,
                meta: RecordMeta::new(now),
            };
            store.create_allele(&first, &audit).await.unwrap();
            store.create_allele(&second, &audit).await.unwrap();
            components.push((locus, first, second, display_order));
        }
        let mut definition =
            GenotypeDefinition::new(lab.id, "Three locus definition", now).unwrap();
        definition
            .replace_components(
                components
                    .iter()
                    .map(|(locus, first, second, display_order)| {
                        GenotypeComponent::new(
                            definition.id,
                            locus.id,
                            first.id,
                            Some(second.id),
                            GenotypeComponentMode::Diploid,
                            *display_order as i32,
                            now,
                        )
                        .unwrap()
                    })
                    .collect(),
            )
            .unwrap();
        store
            .create_genotype_definition(&definition, &audit)
            .await
            .unwrap();
        let record = GenotypingRecord::new(
            lab.id,
            animal.id,
            definition.id,
            GenotypingState::Confirmed,
            Some(now),
            now,
        )
        .unwrap();
        store
            .create_genotyping_record(&record, &audit)
            .await
            .unwrap();

        let records = collect_animal_export_records(&store, lab.id).await.unwrap();
        assert_eq!(records[0].genotypes.len(), 3);
        let bytes = export_animals_with_options(
            &store,
            lab.id,
            ExportFormat::Xlsx,
            &AnimalExportOptions::default(),
        )
        .await
        .unwrap();
        let animals_sheet = read_xlsx(Cursor::new(bytes)).unwrap();
        assert_eq!(animals_sheet.sheet_name, "animals");
        assert!(
            !animals_sheet
                .headers
                .iter()
                .any(|header| header == "animal_uuid")
        );
        assert!(
            !animals_sheet
                .rows
                .iter()
                .flatten()
                .any(|value| value == &animal.id.to_string())
        );

        let csv = export_animals_with_options(
            &store,
            lab.id,
            ExportFormat::Csv,
            &AnimalExportOptions::default(),
        )
        .await
        .unwrap();
        let rendered = String::from_utf8(csv).unwrap();
        assert!(!rendered.contains("animal_uuid"));
        assert!(!rendered.contains(&animal.id.to_string()));
    }

    #[tokio::test]
    async fn snapshot_includes_every_research_extension_aggregate() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let audit = AuditContext::system(WriteSource::Migration);
        let lab = Lab::new("Snapshot research lab", now).unwrap();
        store.create_lab(&lab, &audit).await.unwrap();
        let project = Project::new(lab.id, "Snapshot project", now).unwrap();
        store.create_project(&project, &audit).await.unwrap();

        let male = Animal::new_mouse(lab.id, "SNAP-M", Sex::Male, now).unwrap();
        let female = Animal::new_mouse(lab.id, "SNAP-F", Sex::Female, now).unwrap();
        store.create_animal(&male, &audit).await.unwrap();
        store.create_animal(&female, &audit).await.unwrap();

        let locus = GeneLocus {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            symbol: "SnapshotGene".to_owned(),
            description: None,
            meta: RecordMeta::new(now),
        };
        store.create_gene_locus(&locus, &audit).await.unwrap();
        let allele_1 = Allele {
            id: Uuid::new_v4(),
            locus_id: locus.id,
            symbol: "+".to_owned(),
            description: None,
            is_wild_type: true,
            meta: RecordMeta::new(now),
        };
        let allele_2 = Allele {
            id: Uuid::new_v4(),
            locus_id: locus.id,
            symbol: "flox".to_owned(),
            description: None,
            is_wild_type: false,
            meta: RecordMeta::new(now),
        };
        store.create_allele(&allele_1, &audit).await.unwrap();
        store.create_allele(&allele_2, &audit).await.unwrap();

        let mut genotype_definition =
            GenotypeDefinition::new(lab.id, "Snapshot genotype", now).unwrap();
        genotype_definition
            .replace_components(vec![
                GenotypeComponent::new(
                    genotype_definition.id,
                    locus.id,
                    allele_1.id,
                    Some(allele_2.id),
                    GenotypeComponentMode::Diploid,
                    0,
                    now,
                )
                .unwrap(),
            ])
            .unwrap();
        store
            .create_genotype_definition(&genotype_definition, &audit)
            .await
            .unwrap();
        let genotyping_record = GenotypingRecord::new(
            lab.id,
            female.id,
            genotype_definition.id,
            GenotypingState::Confirmed,
            Some(now),
            now,
        )
        .unwrap();
        store
            .create_genotyping_record(&genotyping_record, &audit)
            .await
            .unwrap();

        let mut line = BreedingLine::new(lab.id, "Snapshot line", now).unwrap();
        line.replace_genotype_definitions(vec![genotype_definition.id])
            .unwrap();
        store.create_breeding_line(&line, &audit).await.unwrap();
        let colony = Colony::new(lab.id, line.id, "Snapshot colony", now).unwrap();
        store.create_colony(&colony, &audit).await.unwrap();
        let mut pair = BreedingPair::new(lab.id, colony.id, "Snapshot pair", now, now).unwrap();
        pair.replace_members(vec![
            BreedingPairMember::new(pair.id, male.id, BreedingMemberRole::Male, now, now).unwrap(),
            BreedingPairMember::new(pair.id, female.id, BreedingMemberRole::Female, now, now)
                .unwrap(),
        ])
        .unwrap();
        store.create_breeding_pair(&pair, &audit).await.unwrap();
        let mating = MatingEvent::new(lab.id, pair.id, male.id, female.id, now, now).unwrap();
        store.create_mating_event(&mating, &audit).await.unwrap();
        let birth_date = now.date_naive();
        let litter = Litter::new(lab.id, mating.id, birth_date, 1, 1, now).unwrap();
        let draft =
            AnimalDraft::new(lab.id, litter.id, "SNAP-P1", Sex::Unknown, birth_date, now).unwrap();
        store
            .create_litter(&litter, std::slice::from_ref(&draft), &audit)
            .await
            .unwrap();

        let experiment = Experiment::new(lab.id, project.id, "Snapshot study", now).unwrap();
        store.create_experiment(&experiment, &audit).await.unwrap();
        let participation = Participation::enroll(experiment.id, female.id, now);
        store
            .create_participation(&participation, &audit)
            .await
            .unwrap();
        let event = ExperimentEvent::new(
            lab.id,
            project.id,
            experiment.id,
            "baseline",
            "Baseline",
            now,
            now,
        )
        .unwrap();
        store.create_experiment_event(&event, &audit).await.unwrap();
        let mut observation_definition = ObservationDefinition::new(
            lab.id,
            project.id,
            experiment.id,
            "weight",
            "Weight",
            ObservationValueType::Number,
            ObservationPolicy::Versioned,
            now,
        )
        .unwrap();
        observation_definition.unit = Some("g".to_owned());
        store
            .create_observation_definition(&observation_definition, &audit)
            .await
            .unwrap();
        let observation = Observation::new(
            lab.id,
            project.id,
            experiment.id,
            event.id,
            observation_definition.id,
            ObservationSubjectType::Animal,
            female.id,
            now,
        )
        .unwrap();
        let value = ObservationValueRecord::new(
            observation.id,
            1,
            ObservationValueData::Number(22.5),
            now,
            now,
        )
        .unwrap();
        store
            .create_observation(&observation, &value, &audit)
            .await
            .unwrap();

        let temp = tempdir().unwrap();
        let snapshot = build_lab_snapshot(
            &store,
            temp.path(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            lab.id,
            None,
            now,
        )
        .await
        .unwrap();
        let manifest = verify_bundle(Cursor::new(snapshot)).unwrap();
        for entity in [
            "genotype_definition",
            "genotyping_record",
            "breeding_line",
            "colony",
            "breeding_pair",
            "mating_event",
            "litter",
            "animal_draft",
            "experiment_event",
            "observation_definition",
            "observation",
            "observation_value",
        ] {
            let path = format!("data/{entity}.jsonl");
            let entry = manifest
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .unwrap_or_else(|| panic!("snapshot is missing {path}"));
            assert_eq!(entry.record_count, Some(1), "unexpected count for {path}");
        }
    }

    #[tokio::test]
    async fn repeated_upload_is_idempotent_and_never_overwrites_changed_content() {
        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path());
        let job_id = Uuid::new_v4();
        files
            .write_upload_bytes(job_id, "animals.csv", b"display_id\nM-1\n")
            .await
            .unwrap();
        files
            .write_upload_bytes(job_id, "animals.csv", b"display_id\nM-1\n")
            .await
            .unwrap();
        assert!(matches!(
            files
                .write_upload_bytes(job_id, "animals.csv", b"display_id\nCHANGED\n")
                .await,
            Err(DataError::Conflict(_))
        ));
        assert_eq!(
            files.read_upload_bytes(job_id).await.unwrap(),
            b"display_id\nM-1\n"
        );
    }

    #[tokio::test]
    async fn explicit_animal_mapping_reparses_a_verified_copy_and_changes_preview_hash() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let lab = Lab::new("Remap lab", now).unwrap();
        let audit = AuditContext::system(WriteSource::Migration);
        store.create_lab(&lab, &audit).await.unwrap();
        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path());
        let original = Job {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: None,
            created_by: Uuid::new_v4(),
            kind: muriarc_core::JobKind::Import,
            status: muriarc_core::JobStatus::AwaitingConfirmation,
            idempotency_key: "auto-map".to_owned(),
            progress_current: 2,
            progress_total: Some(3),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: muriarc_core::RecordMeta::new(now),
        };
        files
            .write_upload_bytes(original.id, "animals.csv", b"custom_code,gender\nM-1,F\n")
            .await
            .unwrap();
        let inferred = files
            .preview_animal_import(&original, &store)
            .await
            .unwrap();
        assert!(!inferred.preview.can_confirm());

        let remapped_job = Job {
            id: Uuid::new_v4(),
            idempotency_key: "manual-map".to_owned(),
            ..original
        };
        let copied = files
            .copy_upload(inferred.job_id, remapped_job.id)
            .await
            .unwrap();
        assert_eq!(copied.sha256, inferred.source.sha256);
        let remapped = files
            .preview_animal_import_with_mapping(
                &remapped_job,
                &store,
                Some(FieldMapping {
                    columns: BTreeMap::from([
                        ("display_id".to_owned(), "custom_code".to_owned()),
                        ("sex".to_owned(), "gender".to_owned()),
                    ]),
                }),
            )
            .await
            .unwrap();
        assert!(
            remapped.preview.can_confirm(),
            "{:?}",
            remapped.preview.issues
        );
        assert_eq!(remapped.preview.accepted_rows[0].display_id, "M-1");
        assert_ne!(remapped.preview_hash, inferred.preview_hash);
    }

    #[tokio::test]
    async fn repeated_artifact_is_idempotent_but_changed_content_conflicts() {
        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path());
        let job_id = Uuid::new_v4();
        let bytes = b"stable";
        let metadata = artifact_metadata(
            job_id,
            ArtifactKind::Export,
            "animals.csv".to_owned(),
            "text/csv".to_owned(),
            bytes,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(
            files.write_artifact(&metadata, bytes).await.unwrap(),
            ArtifactWriteOutcome::Stored
        );
        assert_eq!(
            files.write_artifact(&metadata, bytes).await.unwrap(),
            ArtifactWriteOutcome::Identical
        );
        let changed = artifact_metadata(
            job_id,
            ArtifactKind::Export,
            "animals.csv".to_owned(),
            "text/csv".to_owned(),
            b"changed",
            metadata.created_at,
        )
        .unwrap();
        assert!(matches!(
            files.write_artifact(&changed, b"changed").await,
            Err(DataError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn artifact_reads_reject_same_length_tampering_before_returning_bytes() {
        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path());
        let job_id = Uuid::new_v4();
        let bytes = b"stable";
        let metadata = artifact_metadata(
            job_id,
            ArtifactKind::Export,
            "animals.csv".to_owned(),
            "text/csv".to_owned(),
            bytes,
            Utc::now(),
        )
        .unwrap();
        files.write_artifact(&metadata, bytes).await.unwrap();

        let tampered = b"mutant";
        assert_eq!(tampered.len(), bytes.len());
        tokio::fs::write(
            temp.path().join("artifacts").join(format!("{job_id}.bin")),
            tampered,
        )
        .await
        .unwrap();

        assert!(matches!(
            files.open_artifact(job_id).await,
            Err(DataError::ChecksumMismatch("artifact"))
        ));
        assert!(matches!(
            files.read_artifact_bytes(job_id).await,
            Err(DataError::ChecksumMismatch("artifact"))
        ));
    }

    #[tokio::test]
    async fn measurement_preview_and_confirmation_use_the_published_template() {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let lab = Lab::new("Measurement lab", now).unwrap();
        let system = AuditContext::system(WriteSource::Migration);
        store.create_lab(&lab, &system).await.unwrap();
        let user = User::new(lab.id, "researcher@example.test", "Researcher", now).unwrap();
        store.create_user(&user, &system).await.unwrap();
        let audit = AuditContext {
            actor: Actor::human(user.id, user.display_name.clone()),
            source: WriteSource::Desktop,
            request_id: Some(Uuid::new_v4().to_string()),
            reason: Some("measurement import test".to_owned()),
        };
        let project = Project::new(lab.id, "DEMO", now).unwrap();
        store.create_project(&project, &audit).await.unwrap();
        let mut template = ExperimentTemplateVersion::draft(
            lab.id,
            "demo-measurements",
            1,
            "DEMO measurements",
            now,
        )
        .unwrap();
        template
            .replace_fields(
                vec![TemplateField {
                    key: "body_weight".to_owned(),
                    label: "Body weight".to_owned(),
                    value_type: FieldValueType::Number,
                    unit: Some("g".to_owned()),
                    required: true,
                    categories: Vec::new(),
                    minimum: Some(0.0),
                    maximum: None,
                    display_order: 0,
                    ai_writable: true,
                }],
                now,
            )
            .unwrap();
        store
            .create_template_version(&template, &audit)
            .await
            .unwrap();
        let published = store
            .publish_template_version(template.id, template.meta.revision, user.id, now, &audit)
            .await
            .unwrap();
        let mut experiment = Experiment::new(lab.id, project.id, "DEMO-001", now).unwrap();
        experiment.template_version_id = Some(published.id);
        store.create_experiment(&experiment, &audit).await.unwrap();
        let animal = Animal::new_mouse(lab.id, "M001", Sex::Female, now).unwrap();
        store.create_animal(&animal, &audit).await.unwrap();
        let participation = Participation::enroll(experiment.id, animal.id, now);
        store
            .create_participation(&participation, &audit)
            .await
            .unwrap();
        let job = Job {
            id: Uuid::new_v4(),
            lab_id: lab.id,
            project_id: Some(project.id),
            created_by: user.id,
            kind: muriarc_core::JobKind::Import,
            status: muriarc_core::JobStatus::AwaitingConfirmation,
            idempotency_key: "measurement-import-1".to_owned(),
            progress_current: 2,
            progress_total: Some(3),
            result: None,
            error_report: None,
            cancellation_requested: false,
            meta: muriarc_core::RecordMeta::new(now),
        };
        store.create_job(&job, &audit).await.unwrap();
        let temp = tempdir().unwrap();
        let files = DataFiles::new(temp.path());
        files
            .write_upload_bytes(
                job.id,
                "measurements.csv",
                b"mouse,metric,kind,result,result_unit,when\nM001,body_weight,number,22.4,g,2026-07-19T08:00:00Z\n",
            )
            .await
            .unwrap();
        let pending = files
            .preview_measurement_import_with_mapping(
                &job,
                experiment.id,
                &store,
                Some(MeasurementFieldMapping {
                    columns: BTreeMap::from([
                        ("display_id".to_owned(), "mouse".to_owned()),
                        ("measurement_key".to_owned(), "metric".to_owned()),
                        ("value_type".to_owned(), "kind".to_owned()),
                        ("value".to_owned(), "result".to_owned()),
                        ("unit".to_owned(), "result_unit".to_owned()),
                        ("measured_at".to_owned(), "when".to_owned()),
                    ]),
                }),
            )
            .await
            .unwrap();
        assert!(
            pending.preview.can_confirm(),
            "{:?}",
            pending.preview.issues
        );
        assert_eq!(pending.preview.accepted_rows.len(), 1);
        let public_preview = MeasurementImportPreviewResponse::from(&pending);
        assert_eq!(public_preview.preview_rows.len(), 1);
        assert_eq!(public_preview.preview_rows[0]["row_number"], "2");
        assert_eq!(
            public_preview.preview_rows[0]["animal_id"],
            animal.id.to_string()
        );
        assert_eq!(public_preview.preview_rows[0]["animal_display_id"], "M001");
        assert_eq!(
            public_preview.preview_rows[0]["measurement_key"],
            "body_weight"
        );
        assert_eq!(public_preview.preview_rows[0]["value"], "22.4");
        assert_eq!(public_preview.preview_rows[0]["unit"], "g");
        assert_eq!(
            public_preview.preview_rows[0]["measured_at"],
            "2026-07-19T08:00:00+00:00"
        );
        let receipt = files
            .confirm_measurement_import(&job, &pending.preview_hash, &store, &audit, now)
            .await
            .unwrap();
        assert_eq!(receipt.counts.measurements, 1);
        let measurements = store
            .list_measurements(&MeasurementFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal.id),
            })
            .await
            .unwrap();
        assert_eq!(measurements[0].status, RecordStatus::Draft);

        let repeated_job = Job {
            id: Uuid::new_v4(),
            idempotency_key: "measurement-import-2".to_owned(),
            ..job
        };
        files
            .write_upload_bytes(
                repeated_job.id,
                "measurements.csv",
                b"display_id,measurement_key,value_type,value,unit,measured_at\nM001,body_weight,number,22.4,g,2026-07-19T08:00:00Z\n",
            )
            .await
            .unwrap();
        let repeated = files
            .preview_measurement_import(&repeated_job, experiment.id, &store)
            .await
            .unwrap();
        assert!(!repeated.preview.can_confirm());
        assert!(
            repeated
                .preview
                .issues
                .iter()
                .any(|issue| issue.code == "existing_measurement")
        );
        assert!(repeated.preview.accepted_rows.is_empty());
    }

    #[test]
    fn unsafe_upload_names_are_rejected() {
        assert!(validate_upload_name("../animals.csv").is_err());
        assert!(validate_upload_name("animals.xls").is_err());
    }
}
