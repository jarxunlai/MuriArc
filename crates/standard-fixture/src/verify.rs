use std::{collections::BTreeSet, fs, path::Path};

use muriarc_core::{
    AnimalDraftStatus, AnimalFilter, BreedingPairStatus, DeploymentGenerationManifest,
    ExperimentStatus, LOCAL_LAB_ID, MuriArcStore, ProjectAnimalAssignmentFilter, RecordStatus,
};
use muriarc_data::AttachmentFiles;
use muriarc_store_sqlite::SqliteStore;

use crate::{
    DATABASE_FILE, FixtureBundle, FixtureResult, GENERATION_MANIFEST_FILE, RECEIPT_FILE,
    SeedReceipt, ensure, invalid, read_regular_file, sha256,
};

pub(super) async fn verify(
    bundle: &FixtureBundle,
    root: &Path,
    source_commit: &str,
) -> FixtureResult<SeedReceipt> {
    let metadata = fs::symlink_metadata(root)?;
    ensure(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "output data root must be a real directory",
    )?;
    reject_symlinks(root, root)?;
    ensure(
        root.join(RECEIPT_FILE).is_file(),
        format!("existing output is missing {RECEIPT_FILE}; refusing to modify or clear it"),
    )?;
    let receipt_bytes = read_regular_file(&root.join(RECEIPT_FILE), RECEIPT_FILE)?;
    let receipt: SeedReceipt = serde_json::from_slice(&receipt_bytes)?;
    ensure(
        receipt.schema_version == 1,
        "seed receipt schemaVersion must be 1",
    )?;
    ensure(receipt.status == "PASS", "seed receipt is not complete")?;
    ensure(
        receipt.dataset_id == bundle.dataset.dataset_id,
        "seed dataset id differs",
    )?;
    ensure(
        receipt.dataset_version == bundle.manifest.dataset_version,
        "seed dataset version differs",
    )?;
    ensure(
        receipt.dataset_sha256 == bundle.dataset_sha256,
        "standard-v1 dataset digest drift; create a new generation instead",
    )?;
    ensure(
        receipt.manifest_sha256 == bundle.manifest_sha256,
        "standard-v1 manifest digest drift; create a new generation instead",
    )?;
    ensure(
        receipt.source_commit == source_commit,
        "seed source commit differs",
    )?;
    ensure(
        receipt.application_version == "1.0.0",
        "fixture is not application 1.0.0",
    )?;
    ensure(
        receipt.data_epoch == "E0001",
        "fixture is not data epoch E0001",
    )?;
    ensure(
        receipt.backend == "sqlite",
        "fixture backend must be sqlite",
    )?;
    ensure(
        receipt.expected_counts == bundle.dataset.expected_counts,
        "seed expected counts differ",
    )?;
    ensure(
        receipt.attachment_files == bundle.manifest.files,
        "seed attachment manifest differs",
    )?;

    let database = root.join(DATABASE_FILE);
    let database_metadata = fs::symlink_metadata(&database)?;
    ensure(
        database_metadata.is_file() && !database_metadata.file_type().is_symlink(),
        format!("{DATABASE_FILE} must be a regular non-symlink file"),
    )?;
    let store = SqliteStore::connect_path(&database).await?;
    let result = async {
        store.health_check().await?;
        let report = store.compatibility_report().await?;
        let deployment = report.require_compatible().map_err(invalid)?.clone();
        ensure(
            deployment.generation_id == receipt.generation_id,
            "deployment generation differs from the seed receipt",
        )?;
        ensure(
            deployment.identity.application_version.as_str() == receipt.application_version,
            "deployment application version differs",
        )?;
        ensure(
            deployment.identity.data_epoch.as_str() == receipt.data_epoch,
            "deployment data epoch differs",
        )?;
        let generation_bytes = read_regular_file(
            &root.join(GENERATION_MANIFEST_FILE),
            GENERATION_MANIFEST_FILE,
        )?;
        let generation: DeploymentGenerationManifest = serde_json::from_slice(&generation_bytes)?;
        generation
            .validate(&deployment)
            .map_err(|issue| invalid(format!("{}: {}", issue.code, issue.detail)))?;

        verify_counts(bundle, &receipt)?;
        verify_domain(bundle, &store, &receipt).await?;
        verify_attachments(bundle, root, &store, &receipt).await?;
        let inventory = store.persistent_recovery_inventory().await?;
        ensure(
            inventory.attachment_records
                == u64::try_from(bundle.dataset.attachments.len())
                    .map_err(|_| invalid("count overflow"))?,
            "persistent attachment inventory differs",
        )?;
        ensure(
            inventory.audit_records > 0,
            "seeded fixture has no Audit evidence",
        )?;
        ensure(
            inventory.encrypted_secret_records == 0 && inventory.secret_reference_records == 0,
            "synthetic fixture must not contain AI secret records",
        )?;
        Ok(receipt)
    }
    .await;
    // The verifier owns this pool. Awaiting close is required before a caller
    // can rename or archive the verified data root on Windows.
    store.pool().close().await;
    result
}

fn verify_counts(bundle: &FixtureBundle, receipt: &SeedReceipt) -> FixtureResult<()> {
    let checks = [
        ("projects", receipt.ids.projects.len()),
        ("cages", receipt.ids.cages.len()),
        ("animals", receipt.ids.animals.len()),
        ("genotypingRecords", receipt.ids.genotyping_records.len()),
        ("breedingLines", receipt.ids.breeding_lines.len()),
        ("colonies", receipt.ids.colonies.len()),
        ("breedingPairs", receipt.ids.breeding_pairs.len()),
        ("matingEvents", receipt.ids.mating_events.len()),
        ("litters", receipt.ids.litters.len()),
        ("animalDrafts", receipt.ids.animal_drafts.len()),
        ("templateVersions", receipt.ids.templates.len()),
        ("experiments", receipt.ids.experiments.len()),
        ("cohorts", receipt.ids.cohorts.len()),
        ("participations", receipt.ids.participations.len()),
        ("procedures", receipt.ids.procedures.len()),
        ("experimentEvents", receipt.ids.events.len()),
        (
            "observationDefinitions",
            receipt.ids.observation_definitions.len(),
        ),
        ("observations", receipt.ids.observations.len()),
        ("measurements", receipt.ids.measurements.len()),
        ("samples", receipt.ids.samples.len()),
        ("attachments", receipt.ids.attachments.len()),
    ];
    for (key, actual) in checks {
        let expected = bundle
            .dataset
            .expected_counts
            .get(key)
            .copied()
            .ok_or_else(|| invalid(format!("expected count missing: {key}")))?;
        ensure(
            actual == expected,
            format!("receipt count differs for {key}: expected {expected}, got {actual}"),
        )?;
    }
    ensure(
        receipt.ids.assignments.len() == bundle.dataset.animals.len(),
        "receipt must contain one project assignment per animal",
    )?;
    ensure(
        receipt.ids.loci.len() == 1
            && receipt.ids.alleles.len() == bundle.dataset.genetics.alleles.len()
            && receipt.ids.genotype_definitions.len() == bundle.dataset.genetics.definitions.len(),
        "genetics receipt counts differ",
    )
}

async fn verify_domain(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    receipt: &SeedReceipt,
) -> FixtureResult<()> {
    let projects = store.list_projects(LOCAL_LAB_ID).await?;
    ensure(
        projects.len() == bundle.dataset.projects.len(),
        "project store count differs",
    )?;
    for spec in &bundle.dataset.projects {
        let project = store
            .get_project(id(&receipt.ids.projects, &spec.key, "project")?)
            .await?;
        ensure(
            project.name == spec.name
                && project.description.as_deref() == Some(spec.description.as_str()),
            format!("project baseline drift: {}", spec.key),
        )?;
    }
    let cages = store.list_cages(LOCAL_LAB_ID).await?;
    ensure(
        cages.len() == bundle.dataset.cages.len(),
        "cage store count differs",
    )?;
    for spec in &bundle.dataset.cages {
        let cage = store
            .get_cage(id(&receipt.ids.cages, &spec.key, "cage")?)
            .await?;
        ensure(
            cage.display_id == spec.display_id
                && cage.section == spec.section
                && cage.location.as_deref() == Some(spec.location.as_str())
                && cage.kind == spec.kind
                && cage.capacity == spec.capacity
                && cage.sort_order == spec.sort_order,
            format!("cage baseline drift: {}", spec.key),
        )?;
    }
    let animals = store
        .list_animals(&AnimalFilter {
            lab_id: LOCAL_LAB_ID,
            ..AnimalFilter::default()
        })
        .await?;
    ensure(
        animals.len() == bundle.dataset.animals.len(),
        "animal store count differs",
    )?;
    for spec in &bundle.dataset.animals {
        let animal = store
            .get_animal(id(&receipt.ids.animals, &spec.key, "animal")?)
            .await?;
        ensure(
            animal.display_id == spec.display_id
                && animal.sex == spec.sex
                && animal.strain.as_deref() == Some(spec.strain.as_str())
                && animal.birth_date == Some(spec.birth_date)
                && animal.current_cage_id == Some(id(&receipt.ids.cages, &spec.cage, "cage")?),
            format!("animal baseline drift: {}", spec.key),
        )?;
    }
    let assignments = store
        .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
            lab_id: LOCAL_LAB_ID,
            project_id: None,
            animal_id: None,
        })
        .await?;
    ensure(
        assignments.len() == bundle.dataset.animals.len()
            && assignments
                .iter()
                .map(|value| value.id)
                .collect::<BTreeSet<_>>()
                == receipt
                    .ids
                    .assignments
                    .values()
                    .copied()
                    .collect::<BTreeSet<_>>(),
        "project assignment store ids differ",
    )?;
    for spec in &bundle.dataset.genetics.records {
        let record = store
            .get_genotyping_record(id(
                &receipt.ids.genotyping_records,
                &spec.key,
                "genotyping record",
            )?)
            .await?;
        ensure(
            record.animal_id == id(&receipt.ids.animals, &spec.animal, "animal")?
                && record.genotype_definition_id
                    == id(
                        &receipt.ids.genotype_definitions,
                        &spec.definition,
                        "genotype definition",
                    )?
                && record.state == spec.state
                && record.assessed_at == spec.assessed_at
                && record.method.as_deref() == Some(spec.method.as_str())
                && record.notes.as_deref() == Some(spec.notes.as_str()),
            format!("genotyping record baseline drift: {}", spec.key),
        )?;
    }
    ensure(
        store.list_breeding_lines(LOCAL_LAB_ID).await?.len()
            == bundle.dataset.expected_counts["breedingLines"],
        "breeding line store count differs",
    )?;
    ensure(
        store.list_colonies(LOCAL_LAB_ID, None).await?.len()
            == bundle.dataset.expected_counts["colonies"],
        "colony store count differs",
    )?;
    ensure(
        store.list_breeding_pairs(LOCAL_LAB_ID, None).await?.len()
            == bundle.dataset.expected_counts["breedingPairs"],
        "breeding pair store count differs",
    )?;
    for spec in &bundle.dataset.breeding.pairs {
        let pair = store
            .get_breeding_pair(id(&receipt.ids.breeding_pairs, &spec.key, "breeding pair")?)
            .await?;
        let expected = if spec.status == "retired" {
            BreedingPairStatus::Retired
        } else {
            BreedingPairStatus::Active
        };
        ensure(pair.status == expected, "breeding pair status differs")?;
    }
    let litter = store
        .get_litter(id(
            &receipt.ids.litters,
            &bundle.dataset.breeding.litter.key,
            "litter",
        )?)
        .await?;
    let drafts = store.list_animal_drafts(litter.id).await?;
    ensure(
        drafts.len() == bundle.dataset.expected_counts["animalDrafts"],
        "animal draft store count differs",
    )?;
    ensure(
        drafts
            .iter()
            .filter(|draft| draft.status == AnimalDraftStatus::Registered)
            .count()
            == bundle.dataset.expected_counts["registeredDrafts"],
        "registered draft count differs",
    )?;
    ensure(
        drafts
            .iter()
            .filter(|draft| draft.status == AnimalDraftStatus::Pending)
            .count()
            == bundle.dataset.expected_counts["pendingDrafts"],
        "pending draft count differs",
    )?;
    for spec in &bundle.dataset.experiments {
        let experiment = store
            .get_experiment(id(&receipt.ids.experiments, &spec.key, "experiment")?)
            .await?;
        let expected = match spec.status.as_str() {
            "completed" => ExperimentStatus::Completed,
            "cancelled" => ExperimentStatus::Cancelled,
            "draft" => ExperimentStatus::Draft,
            _ => return Err(invalid("unsupported experiment status").into()),
        };
        ensure(experiment.status == expected, "experiment status differs")?;
    }
    for spec in &bundle.dataset.participations {
        let participation = store
            .get_participation(id(&receipt.ids.participations, &spec.key, "participation")?)
            .await?;
        ensure(
            participation.status == spec.status,
            "participation status differs",
        )?;
    }
    let procedure_ids = receipt
        .ids
        .procedures
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut observed_procedures = BTreeSet::new();
    for experiment_id in receipt.ids.experiments.values() {
        observed_procedures.extend(
            store
                .list_procedures(*experiment_id, None)
                .await?
                .into_iter()
                .map(|value| value.id),
        );
    }
    ensure(
        observed_procedures == procedure_ids,
        "procedure store ids differ",
    )?;
    let mut revision_count = 0;
    for (key, observation_id) in &receipt.ids.observations {
        let values = store.list_observation_values(*observation_id).await?;
        let spec = bundle
            .dataset
            .observations
            .iter()
            .find(|value| &value.key == key)
            .ok_or_else(|| invalid("receipt contains an unknown observation key"))?;
        ensure(
            values.len() == if spec.revision.is_some() { 2 } else { 1 },
            format!("observation value history differs: {key}"),
        )?;
        revision_count += values.len().saturating_sub(1);
    }
    ensure(
        revision_count == bundle.dataset.expected_counts["observationRevisions"],
        "observation revision count differs",
    )?;
    for spec in &bundle.dataset.measurements {
        let measurement = store
            .get_measurement(id(&receipt.ids.measurements, &spec.key, "measurement")?)
            .await?;
        ensure(
            measurement.status == spec.status
                && measurement.key == spec.measurement_key
                && measurement.value == spec.value,
            format!("measurement baseline drift: {}", spec.key),
        )?;
        if spec.status == RecordStatus::Signed {
            ensure(
                measurement.signed_by.is_some() && measurement.signed_at.is_some(),
                "signed measurement lacks signature metadata",
            )?;
        }
    }
    for spec in &bundle.dataset.samples {
        let sample = store
            .get_sample(id(&receipt.ids.samples, &spec.key, "sample")?)
            .await?;
        ensure(
            sample.sample_type == spec.sample_type
                && sample.quantity == Some(spec.quantity)
                && sample.unit.as_deref() == Some(spec.unit.as_str())
                && sample.location.as_deref() == Some(spec.location.as_str()),
            format!("sample baseline drift: {}", spec.key),
        )?;
    }
    Ok(())
}

async fn verify_attachments(
    bundle: &FixtureBundle,
    root: &Path,
    store: &SqliteStore,
    receipt: &SeedReceipt,
) -> FixtureResult<()> {
    let files = AttachmentFiles::new(root.join("attachments"));
    files.initialize().await?;
    let stored = store.list_lab_attachments(LOCAL_LAB_ID).await?;
    ensure(
        stored.len() == bundle.dataset.attachments.len(),
        "attachment store count differs",
    )?;
    for spec in &bundle.dataset.attachments {
        let attachment = store
            .get_attachment(id(&receipt.ids.attachments, &spec.key, "attachment")?)
            .await?;
        let expected = bundle
            .manifest
            .files
            .get(&spec.file)
            .ok_or_else(|| invalid("attachment manifest entry is missing"))?;
        ensure(
            attachment.file_name == spec.file
                && attachment.media_type.as_deref() == Some(spec.media_type.as_str())
                && attachment.sha256 == *expected
                && attachment.version == 1,
            format!("attachment metadata drift: {}", spec.key),
        )?;
        let mut verified = files.open_verified(&attachment).await?;
        let mut bytes = Vec::new();
        use tokio::io::AsyncReadExt;
        verified.file.read_to_end(&mut bytes).await?;
        ensure(
            sha256(&bytes) == *expected,
            "attachment content checksum differs",
        )?;
    }
    Ok(())
}

fn reject_symlinks(root: &Path, current: &Path) -> FixtureResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure(
            !metadata.file_type().is_symlink(),
            format!(
                "generated data root contains a symlink: {}",
                path.strip_prefix(root).unwrap_or(&path).display()
            ),
        )?;
        if metadata.is_dir() {
            reject_symlinks(root, &path)?;
        }
    }
    Ok(())
}

fn id(
    map: &std::collections::BTreeMap<String, uuid::Uuid>,
    key: &str,
    label: &str,
) -> FixtureResult<uuid::Uuid> {
    map.get(key)
        .copied()
        .ok_or_else(|| invalid(format!("unknown {label} key: {key}")).into())
}
