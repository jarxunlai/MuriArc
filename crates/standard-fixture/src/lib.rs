#![forbid(unsafe_code)]

mod model;
mod seed;
mod verify;

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use model::{FixtureIds, SeedReceipt};

pub type FixtureResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub const RECEIPT_FILE: &str = "standard-v1-seed.json";
pub const DATABASE_FILE: &str = "muriarc.sqlite3";
pub const GENERATION_MANIFEST_FILE: &str = "deployment-generation.json";

#[derive(Debug)]
struct FixtureBundle {
    root: PathBuf,
    dataset: model::Dataset,
    manifest: model::FixtureManifest,
    dataset_sha256: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Seed,
    Verify,
    #[cfg(feature = "postgres")]
    SeedPostgres,
    #[cfg(feature = "postgres")]
    VerifyPostgres,
}

#[derive(Debug)]
struct Cli {
    command: Command,
    fixture: PathBuf,
    output: PathBuf,
    source_commit: String,
}

pub async fn run_cli<I>(args: I) -> FixtureResult<bool>
where
    I: IntoIterator<Item = OsString>,
{
    let Some(cli) = parse_cli(args)? else {
        return Ok(false);
    };
    let receipt = match cli.command {
        Command::Seed => seed_standard_v1(&cli.fixture, &cli.output, &cli.source_commit).await?,
        Command::Verify => {
            verify_standard_v1(&cli.fixture, &cli.output, &cli.source_commit).await?
        }
        #[cfg(feature = "postgres")]
        Command::SeedPostgres => {
            let database_url = std::env::var("MURIARC_FIXTURE_DATABASE_URL")
                .map_err(|_| invalid("MURIARC_FIXTURE_DATABASE_URL is required"))?;
            seed_postgres_standard_v1(&cli.fixture, &cli.output, &cli.source_commit, &database_url)
                .await?
        }
        #[cfg(feature = "postgres")]
        Command::VerifyPostgres => {
            let database_url = std::env::var("MURIARC_FIXTURE_DATABASE_URL")
                .map_err(|_| invalid("MURIARC_FIXTURE_DATABASE_URL is required"))?;
            verify_postgres_standard_v1(
                &cli.fixture,
                &cli.output,
                &cli.source_commit,
                &database_url,
            )
            .await?
        }
    };
    println!("{}", serde_json::to_string(&receipt)?);
    Ok(true)
}

pub async fn seed_standard_v1(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
) -> FixtureResult<SeedReceipt> {
    validate_source_commit(source_commit)?;
    let bundle = load_fixture(fixture_root.as_ref())?;
    let output = output_root.as_ref();
    match fs::symlink_metadata(output) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid("output data root must be a real directory").into());
        }
        Ok(_) => return verify::verify(&bundle, output, source_commit).await,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let parent = output
        .parent()
        .ok_or_else(|| invalid("output data root must have a parent directory"))?;
    fs::create_dir_all(parent)?;
    let parent = fs::canonicalize(parent)?;
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("output data root must have a valid final component"))?;
    let staging = parent.join(format!(".{name}.staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let result = seed::seed_into(&bundle, &staging, source_commit).await;
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    fs::rename(&staging, output)?;
    verify::verify(&bundle, output, source_commit).await?;
    Ok(receipt)
}

#[cfg(feature = "postgres")]
pub async fn seed_postgres_standard_v1(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
    database_url: &str,
) -> FixtureResult<SeedReceipt> {
    validate_source_commit(source_commit)?;
    ensure(
        !database_url.trim().is_empty(),
        "PostgreSQL URL is required",
    )?;
    let bundle = load_fixture(fixture_root.as_ref())?;
    let output = output_root.as_ref();
    match fs::symlink_metadata(output) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid("output data root must be a real directory").into());
        }
        Ok(_) => {
            return verify_postgres_standard_v1(fixture_root, output, source_commit, database_url)
                .await;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = output
        .parent()
        .ok_or_else(|| invalid("output data root must have a parent directory"))?;
    fs::create_dir_all(parent)?;
    let parent = fs::canonicalize(parent)?;
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("output data root must have a valid final component"))?;
    let staging = parent.join(format!(".{name}.staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let result = seed::seed_postgres_into(&bundle, &staging, source_commit, database_url).await;
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    fs::rename(&staging, output)?;
    verify_postgres_standard_v1(fixture_root, output, source_commit, database_url).await?;
    Ok(receipt)
}

#[cfg(feature = "postgres")]
pub async fn verify_postgres_standard_v1(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
    database_url: &str,
) -> FixtureResult<SeedReceipt> {
    use muriarc_store_postgres::PostgresStore;

    validate_source_commit(source_commit)?;
    ensure(
        !database_url.trim().is_empty(),
        "PostgreSQL URL is required",
    )?;
    let bundle = load_fixture(fixture_root.as_ref())?;
    let store = PostgresStore::connect(database_url).await?;
    let result =
        verify::verify_postgres(&bundle, output_root.as_ref(), source_commit, &store).await;
    store.pool().close().await;
    result
}

pub async fn verify_standard_v1(
    fixture_root: impl AsRef<Path>,
    output_root: impl AsRef<Path>,
    source_commit: &str,
) -> FixtureResult<SeedReceipt> {
    validate_source_commit(source_commit)?;
    let bundle = load_fixture(fixture_root.as_ref())?;
    verify::verify(&bundle, output_root.as_ref(), source_commit).await
}

fn parse_cli<I>(args: I) -> FixtureResult<Option<Cli>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut values = args.into_iter();
    let Some(command) = values.next() else {
        return Ok(None);
    };
    let command = match command.to_string_lossy().as_ref() {
        "seed" => Command::Seed,
        "verify" => Command::Verify,
        #[cfg(feature = "postgres")]
        "seed-postgres" => Command::SeedPostgres,
        #[cfg(feature = "postgres")]
        "verify-postgres" => Command::VerifyPostgres,
        "-h" | "--help" => return Ok(None),
        _ => {
            return Err(invalid(
                "first argument must be seed, verify, seed-postgres, or verify-postgres",
            )
            .into());
        }
    };
    let mut fixture = None;
    let mut output = None;
    let mut source_commit = None;
    while let Some(flag) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| invalid(format!("missing value for {}", flag.to_string_lossy())))?;
        match flag.to_string_lossy().as_ref() {
            "--fixture" => fixture = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--source-commit" => source_commit = Some(value.to_string_lossy().into_owned()),
            _ => {
                return Err(
                    invalid(format!("unknown argument: {}", flag.to_string_lossy())).into(),
                );
            }
        }
    }
    Ok(Some(Cli {
        command,
        fixture: fixture.ok_or_else(|| invalid("--fixture is required"))?,
        output: output.ok_or_else(|| invalid("--output is required"))?,
        source_commit: source_commit.ok_or_else(|| invalid("--source-commit is required"))?,
    }))
}

fn load_fixture(root: &Path) -> FixtureResult<FixtureBundle> {
    let root = canonical_real_directory(root, "fixture root")?;
    let dataset_bytes = read_regular_file(&root.join("dataset.json"), "dataset.json")?;
    let manifest_bytes = read_regular_file(&root.join("manifest.json"), "manifest.json")?;
    let schema_bytes = read_regular_file(&root.join("schema.json"), "schema.json")?;
    scan_sensitive_bytes("dataset.json", &dataset_bytes)?;
    scan_sensitive_bytes("manifest.json", &manifest_bytes)?;
    scan_sensitive_bytes("schema.json", &schema_bytes)?;

    let dataset: model::Dataset = serde_json::from_slice(&dataset_bytes)?;
    let manifest: model::FixtureManifest = serde_json::from_slice(&manifest_bytes)?;
    let schema: Value = serde_json::from_slice(&schema_bytes)?;
    validate_fixture(&root, &dataset, &manifest, &schema)?;
    Ok(FixtureBundle {
        root,
        dataset,
        manifest,
        dataset_sha256: sha256(&dataset_bytes),
        manifest_sha256: sha256(&manifest_bytes),
    })
}

fn validate_fixture(
    root: &Path,
    dataset: &model::Dataset,
    manifest: &model::FixtureManifest,
    schema: &Value,
) -> FixtureResult<()> {
    ensure(
        dataset.schema_version == 1,
        "dataset schemaVersion must be 1",
    )?;
    ensure(
        dataset.dataset_id == "muriarc-standard-v1",
        "unexpected dataset id",
    )?;
    ensure(dataset.synthetic, "dataset must be explicitly synthetic")?;
    ensure(
        dataset.fixed_timeline.starts_at < dataset.fixed_timeline.ends_at,
        "fixed timeline must be increasing",
    )?;
    ensure(
        manifest.schema_version == 1,
        "manifest schemaVersion must be 1",
    )?;
    ensure(
        manifest.dataset_id == dataset.dataset_id,
        "manifest dataset id differs",
    )?;
    ensure(
        manifest.dataset_version == "standard-v1",
        "unexpected dataset version",
    )?;
    ensure(manifest.synthetic, "manifest must be explicitly synthetic")?;
    ensure(
        manifest.fixed_timeline,
        "manifest must require a fixed timeline",
    )?;
    ensure(
        manifest.public_api_only,
        "manifest must require public API writes",
    )?;
    ensure(
        manifest.baseline_policy == "strict",
        "baseline policy must be strict",
    )?;
    ensure(
        manifest.sandbox_policy == "preserve",
        "sandbox policy must be preserve",
    )?;
    ensure(
        !manifest.known_public_api_limits.is_empty(),
        "known public API limits must remain explicit",
    )?;
    ensure(
        dataset.expected_counts == manifest.expected_counts,
        "dataset and manifest counts differ",
    )?;
    ensure(
        schema.get("$id").is_some() || schema.get("title").is_some(),
        "fixture schema is missing its identity",
    )?;

    let derived = derived_counts(dataset);
    ensure(
        derived == dataset.expected_counts,
        format!(
            "fixture counts differ: expected {:?}, derived {derived:?}",
            dataset.expected_counts
        ),
    )?;
    unique_keys(
        "projects",
        dataset.projects.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "cages",
        dataset.cages.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "animals",
        dataset.animals.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "templates",
        dataset.templates.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "experiments",
        dataset.experiments.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "cohorts",
        dataset.cohorts.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "participations",
        dataset
            .participations
            .iter()
            .map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "procedures",
        dataset.procedures.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "events",
        dataset.events.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "observationDefinitions",
        dataset
            .observation_definitions
            .iter()
            .map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "observations",
        dataset.observations.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "measurements",
        dataset.measurements.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "samples",
        dataset.samples.iter().map(|value| value.key.as_str()),
    )?;
    unique_keys(
        "attachments",
        dataset.attachments.iter().map(|value| value.key.as_str()),
    )?;

    for mode in dataset
        .projects
        .iter()
        .filter_map(|value| value.mode.as_deref())
        .chain(
            dataset
                .cages
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
        .chain(
            dataset
                .animals
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
        .chain(
            dataset
                .templates
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
        .chain(
            dataset
                .experiments
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
        .chain(
            dataset
                .cohorts
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
        .chain(
            dataset
                .participations
                .iter()
                .filter_map(|value| value.mode.as_deref()),
        )
    {
        ensure(
            matches!(mode, "baseline" | "sandbox"),
            "fixture contains an invalid mode",
        )?;
    }
    ensure(
        dataset
            .animals
            .iter()
            .filter(|value| value.sex == muriarc_core::Sex::Male)
            .count()
            == 12
            && dataset
                .animals
                .iter()
                .filter(|value| value.sex == muriarc_core::Sex::Female)
                .count()
                == 12,
        "standard-v1 must contain 12 male and 12 female animals",
    )?;
    ensure(
        dataset
            .animals
            .first()
            .map(|value| value.display_id.as_str())
            == Some("STD-M-001")
            && dataset
                .animals
                .last()
                .map(|value| value.display_id.as_str())
                == Some("STD-M-024"),
        "animal ids must cover STD-M-001 through STD-M-024",
    )?;
    for animal in &dataset.animals {
        if animal.source == "litter" {
            ensure(
                animal.temporary_label.is_some(),
                "litter animal is missing temporaryLabel",
            )?;
        }
        if let Some(cohort) = &animal.cohort {
            ensure(
                matches!(cohort.as_str(), "control" | "treatment"),
                "animal cohort label is invalid",
            )?;
        }
    }
    ensure(
        dataset.breeding.litter.pair == dataset.breeding.mating.pair
            && dataset.breeding.litter.mating == dataset.breeding.mating.key,
        "litter and mating references differ",
    )?;

    let files_root = canonical_real_directory(&root.join("files"), "fixture files root")?;
    let actual_names = fs::read_dir(&files_root)?
        .map(|entry| {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(invalid("fixture files must be regular non-symlink files"));
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("fixture file name is not UTF-8"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let expected_names = manifest.files.keys().cloned().collect::<BTreeSet<_>>();
    ensure(
        actual_names == expected_names,
        "fixture attachment file set differs from manifest",
    )?;
    for (name, expected_digest) in &manifest.files {
        ensure(
            is_sha256(expected_digest),
            "fixture attachment digest is invalid",
        )?;
        let bytes = read_regular_file(&files_root.join(name), "fixture attachment")?;
        scan_sensitive_bytes(name, &bytes)?;
        ensure(
            sha256(&bytes) == *expected_digest,
            format!("fixture attachment checksum mismatch: {name}"),
        )?;
    }
    Ok(())
}

fn derived_counts(dataset: &model::Dataset) -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("projects".to_owned(), dataset.projects.len()),
        ("cages".to_owned(), dataset.cages.len()),
        ("animals".to_owned(), dataset.animals.len()),
        (
            "genotypingRecords".to_owned(),
            dataset.genetics.records.len(),
        ),
        ("breedingLines".to_owned(), 1),
        ("colonies".to_owned(), dataset.breeding.colonies.len()),
        ("breedingPairs".to_owned(), dataset.breeding.pairs.len()),
        ("matingEvents".to_owned(), 1),
        ("litters".to_owned(), 1),
        (
            "animalDrafts".to_owned(),
            dataset.breeding.litter.drafts.len(),
        ),
        (
            "registeredDrafts".to_owned(),
            dataset
                .breeding
                .litter
                .drafts
                .iter()
                .filter(|value| value.animal.is_some())
                .count(),
        ),
        (
            "pendingDrafts".to_owned(),
            dataset
                .breeding
                .litter
                .drafts
                .iter()
                .filter(|value| value.animal.is_none())
                .count(),
        ),
        ("templateVersions".to_owned(), dataset.templates.len()),
        ("experiments".to_owned(), dataset.experiments.len()),
        ("cohorts".to_owned(), dataset.cohorts.len()),
        ("participations".to_owned(), dataset.participations.len()),
        ("procedures".to_owned(), dataset.procedures.len()),
        ("experimentEvents".to_owned(), dataset.events.len()),
        (
            "observationDefinitions".to_owned(),
            dataset.observation_definitions.len(),
        ),
        ("observations".to_owned(), dataset.observations.len()),
        (
            "observationRevisions".to_owned(),
            dataset
                .observations
                .iter()
                .filter(|value| value.revision.is_some())
                .count(),
        ),
        ("measurements".to_owned(), dataset.measurements.len()),
        ("samples".to_owned(), dataset.samples.len()),
        ("attachments".to_owned(), dataset.attachments.len()),
    ])
}

fn unique_keys<'a>(label: &str, values: impl Iterator<Item = &'a str>) -> FixtureResult<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        ensure(
            !value.trim().is_empty(),
            format!("{label} contains an empty key"),
        )?;
        ensure(
            seen.insert(value),
            format!("{label} contains a duplicate key"),
        )?;
    }
    Ok(())
}

fn validate_source_commit(value: &str) -> FixtureResult<()> {
    ensure(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "source commit must be exactly 40 lowercase hexadecimal characters",
    )
}

fn canonical_real_directory(path: &Path, label: &str) -> FixtureResult<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    ensure(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        format!("{label} must be a real directory"),
    )?;
    Ok(fs::canonicalize(path)?)
}

fn read_regular_file(path: &Path, label: &str) -> FixtureResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        format!("{label} must be a regular non-symlink file"),
    )?;
    Ok(fs::read(path)?)
}

fn scan_sensitive_bytes(label: &str, bytes: &[u8]) -> FixtureResult<()> {
    let lower = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    for marker in [
        b"-----begin private key-----".as_slice(),
        b"-----begin rsa private key-----".as_slice(),
        b"-----begin openssh private key-----".as_slice(),
        b"authorization: bearer ".as_slice(),
        b"ghp_".as_slice(),
        b"github_pat_".as_slice(),
    ] {
        ensure(
            !lower.windows(marker.len()).any(|window| window == marker),
            format!("sensitive credential pattern found in {label}"),
        )?;
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn ensure(condition: bool, message: impl Into<String>) -> FixtureResult<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message).into())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/standard-v1")
    }

    #[test]
    fn repository_fixture_is_strict_and_sensitive_free() {
        let bundle = load_fixture(&fixture_root()).unwrap();
        assert_eq!(bundle.dataset.dataset_id, "muriarc-standard-v1");
        assert_eq!(bundle.dataset_sha256.len(), 64);
        assert_eq!(bundle.manifest_sha256.len(), 64);
    }

    #[tokio::test]
    async fn fresh_seed_is_verified_and_second_seed_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("desktop-data");
        let commit = "1111111111111111111111111111111111111111";
        let first = seed_standard_v1(fixture_root(), &output, commit)
            .await
            .unwrap();
        let second = seed_standard_v1(fixture_root(), &output, commit)
            .await
            .unwrap();
        assert_eq!(first.dataset_sha256, second.dataset_sha256);
        assert_eq!(first.generation_id, second.generation_id);
        assert_eq!(first.ids.animals.len(), 24);
        assert_eq!(first.ids.attachments.len(), 7);
    }

    #[tokio::test]
    async fn existing_empty_output_is_rejected_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("desktop-data");
        fs::create_dir(&output).unwrap();
        let error = seed_standard_v1(
            fixture_root(),
            &output,
            "1111111111111111111111111111111111111111",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains(RECEIPT_FILE));
        assert_eq!(fs::read_dir(output).unwrap().count(), 0);
    }
}
