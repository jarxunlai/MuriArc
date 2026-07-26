use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use async_trait::async_trait;
use muriarc_release_evidence::{
    AdapterLayerEvidence, CompatibilityMatrixDefinition, CompatibilityMatrixReport, EvidenceError,
    FixtureCatalog, Sha256Digest, VerificationAdapter, VerificationContext, VerificationLayer,
    VerificationReport, VerifierRunner, load_and_verify_fixture,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug)]
enum Command {
    Help,
    Asset {
        root: PathBuf,
        manifest_digest: Option<Sha256Digest>,
    },
    Run {
        request: PathBuf,
    },
    Report {
        report: PathBuf,
    },
    Matrix {
        report: PathBuf,
        definition: PathBuf,
        catalog: PathBuf,
    },
}

#[derive(Debug)]
struct Parsed {
    output: OutputFormat,
    command: Command,
}

#[derive(Debug, Serialize)]
struct Response<T: Serialize> {
    ok: bool,
    code: &'static str,
    message: String,
    data: T,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    fixture_root: PathBuf,
    expected_manifest_digest: Option<Sha256Digest>,
    target_identity: muriarc_core::ReleaseIdentity,
    target_artifact_digest: Sha256Digest,
    mode: muriarc_release_evidence::VerificationMode,
    profile: muriarc_release_evidence::DeliveryProfile,
    execution_kind: muriarc_release_evidence::ArtifactExecutionKind,
    evidence_directory: PathBuf,
    report_output: PathBuf,
}

struct FileEvidenceAdapter {
    directory: PathBuf,
}

#[async_trait]
impl VerificationAdapter for FileEvidenceAdapter {
    async fn verify_layer(
        &self,
        layer: VerificationLayer,
        _context: &VerificationContext<'_>,
    ) -> Result<AdapterLayerEvidence, EvidenceError> {
        let name = match layer {
            VerificationLayer::AssetRestore => {
                return Err(EvidenceError::InvalidReport {
                    message: "asset restore evidence is produced by the built-in verifier"
                        .to_owned(),
                });
            }
            VerificationLayer::Storage => "storage",
            VerificationLayer::StoreApplication => "store_application",
            VerificationLayer::Api => "api",
            VerificationLayer::RemoteUi => "remote_ui",
            VerificationLayer::ContinueWrite => "continue_write",
            VerificationLayer::ReadOnlyNoSideEffects => "read_only_no_side_effects",
        };
        let path = self.directory.join(format!("{name}.json"));
        let metadata =
            tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|error| EvidenceError::Io {
                    message: error.to_string(),
                })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(EvidenceError::AssetVerification {
                message: format!("{name} evidence must be a regular non-symlink file"),
            });
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| EvidenceError::Io {
                message: error.to_string(),
            })?;
        serde_json::from_slice(&bytes).map_err(|error| EvidenceError::Serialization {
            message: error.to_string(),
        })
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let parsed = match parse_args(env::args_os().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => {
            emit_error(OutputFormat::Human, error);
            return ExitCode::from(2);
        }
    };
    let output = parsed.output;
    match dispatch(parsed).await {
        Ok(data) => {
            emit(
                output,
                &Response {
                    ok: true,
                    code: "ok",
                    message: "verification command completed".to_owned(),
                    data,
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            emit_error(output, error);
            ExitCode::from(2)
        }
    }
}

async fn dispatch(parsed: Parsed) -> Result<Value, EvidenceError> {
    match parsed.command {
        Command::Help => Ok(json!({ "usage": HELP })),
        Command::Asset {
            root,
            manifest_digest,
        } => {
            let (_, _, result) = load_and_verify_fixture(&root, manifest_digest.as_ref())?;
            serde_json::to_value(result).map_err(serialization)
        }
        Command::Run { request } => {
            let request: RunRequest = load_json(&request)?;
            let runner = VerifierRunner::new(
                request.fixture_root,
                request.expected_manifest_digest,
                request.target_identity,
                request.target_artifact_digest,
                request.mode,
                request.profile,
                request.execution_kind,
                FileEvidenceAdapter {
                    directory: request.evidence_directory,
                },
            );
            let report = runner.run().await?;
            report.validate()?;
            write_json_atomic(&request.report_output, &report)?;
            serde_json::to_value(report).map_err(serialization)
        }
        Command::Report { report } => {
            let report: VerificationReport = load_json(&report)?;
            report.validate()?;
            Ok(json!({ "reportDigest": report.digest()? }))
        }
        Command::Matrix {
            report,
            definition,
            catalog,
        } => {
            let report: CompatibilityMatrixReport = load_json(&report)?;
            let definition: CompatibilityMatrixDefinition = load_json(&definition)?;
            let catalog: FixtureCatalog = load_json(&catalog)?;
            report.validate(&definition, &catalog)?;
            Ok(json!({
                "mode": report.mode,
                "selectedFixtureCount": report.selected_fixture_ids.len(),
                "runCount": report.runs.len(),
            }))
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, EvidenceError> {
    let mut args = args
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| EvidenceError::InvalidReport {
                    message: "arguments must be UTF-8".to_owned(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = OutputFormat::Human;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--output" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| EvidenceError::InvalidReport {
                    message: "--output requires human or json".to_owned(),
                })?;
            output = match value.as_str() {
                "human" => OutputFormat::Human,
                "json" => OutputFormat::Json,
                _ => {
                    return Err(EvidenceError::InvalidReport {
                        message: "--output requires human or json".to_owned(),
                    });
                }
            };
            args.drain(index..=index + 1);
        } else {
            index += 1;
        }
    }
    let command = match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") if args.len() <= 1 => Command::Help,
        Some("asset") => Command::Asset {
            root: required_path(&args, "--root")?,
            manifest_digest: optional_value(&args, "--manifest-digest")
                .map(str::parse)
                .transpose()?,
        },
        Some("run") => Command::Run {
            request: required_path(&args, "--request")?,
        },
        Some("report") => Command::Report {
            report: required_path(&args, "--report")?,
        },
        Some("matrix") => Command::Matrix {
            report: required_path(&args, "--report")?,
            definition: required_path(&args, "--definition")?,
            catalog: required_path(&args, "--catalog")?,
        },
        _ => {
            return Err(EvidenceError::InvalidReport {
                message: "unknown command; run muriarc-verifier help".to_owned(),
            });
        }
    };
    Ok(Parsed { output, command })
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf, EvidenceError> {
    optional_value(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| EvidenceError::InvalidReport {
            message: format!("{name} is required"),
        })
}

fn optional_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].as_str())
}

fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, EvidenceError> {
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EvidenceError::AssetVerification {
            message: "JSON input must be a regular non-symlink file".to_owned(),
        });
    }
    serde_json::from_slice(&fs::read(path).map_err(io)?).map_err(serialization)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), EvidenceError> {
    let parent = path.parent().ok_or_else(|| EvidenceError::Io {
        message: "report output has no parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut bytes = serde_json::to_vec_pretty(value).map_err(serialization)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).map_err(io)?;
    fs::rename(&temporary, path).map_err(io)
}

fn emit_error(output: OutputFormat, error: EvidenceError) {
    emit(
        output,
        &Response {
            ok: false,
            code: error_code(&error),
            message: error.to_string(),
            data: Value::Null,
        },
    );
}

fn emit<T: Serialize>(output: OutputFormat, response: &T) {
    match output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string(response).expect("response serializes")
            );
        }
        OutputFormat::Human => {
            let value = serde_json::to_value(response).expect("response serializes");
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let code = value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            println!("{} [{code}] {message}", if ok { "OK" } else { "ERROR" });
        }
    }
}

fn error_code(error: &EvidenceError) -> &'static str {
    match error {
        EvidenceError::InvalidDigest => "invalid_digest",
        EvidenceError::UnsafePath { .. } => "unsafe_path",
        EvidenceError::InvalidFixture { .. } => "invalid_fixture",
        EvidenceError::IncompleteRecoverySet => "incomplete_recovery_set",
        EvidenceError::WrongProducerRelease => "wrong_producer_release",
        EvidenceError::ExpectedFactsMismatch => "expected_facts_mismatch",
        EvidenceError::ExpectedFactsIncomplete => "expected_facts_incomplete",
        EvidenceError::DuplicateFactId { .. } => "duplicate_fact_id",
        EvidenceError::CatalogNotAppendOnly => "catalog_not_append_only",
        EvidenceError::InvalidCatalogEntry { .. } => "invalid_catalog_entry",
        EvidenceError::AssetVerification { .. } => "asset_verification_failed",
        EvidenceError::LayerFailed { .. } => "verification_layer_failed",
        EvidenceError::InvalidReport { .. } => "invalid_report",
        EvidenceError::Io { .. } => "io_failed",
        EvidenceError::Serialization { .. } => "serialization_failed",
    }
}

fn io(error: std::io::Error) -> EvidenceError {
    EvidenceError::Io {
        message: error.to_string(),
    }
}

fn serialization(error: serde_json::Error) -> EvidenceError {
    EvidenceError::Serialization {
        message: error.to_string(),
    }
}

const HELP: &str = "\
MuriArc immutable compatibility verifier

Usage:
  muriarc-verifier asset --root <fixture-dir> [--manifest-digest sha256:...]
  muriarc-verifier run --request <run-request.json>
  muriarc-verifier report --report <verification-report.json>
  muriarc-verifier matrix --report <matrix-report.json> --definition <matrix.json> --catalog <catalog.json>
  add --output json to any command for stable machine output
";
