use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use muriarc_core::{
    BackendKind, DeploymentGenerationManifest, GENERATION_MANIFEST_FORMAT, ReleaseIdentity,
};
use muriarc_release_evidence::*;
use tokio::sync::Mutex;
use uuid::Uuid;

fn digest(value: &[u8]) -> Sha256Digest {
    digest_bytes(value)
}

fn identity() -> ReleaseIdentity {
    ReleaseIdentity::parse(
        "0.1.0".to_owned(),
        "preview_epoch_0".to_owned(),
        format!("sha256:{}", "a".repeat(64)),
        "gateway-v1".to_owned(),
    )
    .unwrap()
}

fn synthetic_facts(fixture_id: Uuid, identity: ReleaseIdentity) -> ExpectedFacts {
    let user_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let sire_id = Uuid::new_v4();
    let dam_id = Uuid::new_v4();
    let animal_id = Uuid::new_v4();
    let experiment_id = Uuid::new_v4();
    let observation_id = Uuid::new_v4();
    let measurement_id = Uuid::new_v4();
    let sample_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let user_message_id = Uuid::new_v4();
    let assistant_message_id = Uuid::new_v4();
    let tool_run_id = Uuid::new_v4();
    ExpectedFacts {
        format_version: EXPECTED_FACTS_FORMAT_VERSION,
        fixture_id,
        release_identity: identity,
        accounts: vec![AccountFact {
            user_id,
            normalized_email_digest: digest(b"synthetic@example.invalid"),
            lab_roles: BTreeSet::from(["lab_admin".to_owned()]),
            project_ids: BTreeSet::from([project_id]),
            active: true,
        }],
        projects: vec![ProjectFact {
            project_id,
            name_digest: digest(b"synthetic-project"),
            active: true,
        }],
        animals: vec![
            AnimalFact {
                animal_id: sire_id,
                display_id: "SYN-SIRE".to_owned(),
                status: "alive".to_owned(),
                sire_id: None,
                dam_id: None,
                revision: 1,
            },
            AnimalFact {
                animal_id: dam_id,
                display_id: "SYN-DAM".to_owned(),
                status: "alive".to_owned(),
                sire_id: None,
                dam_id: None,
                revision: 1,
            },
            AnimalFact {
                animal_id,
                display_id: "SYN-001".to_owned(),
                status: "in_experiment".to_owned(),
                sire_id: Some(sire_id),
                dam_id: Some(dam_id),
                revision: 3,
            },
        ],
        breeding: vec![BreedingFact {
            breeding_id: Uuid::new_v4(),
            male_id: sire_id,
            female_ids: BTreeSet::from([dam_id]),
            offspring_ids: BTreeSet::from([animal_id]),
            status: "completed".to_owned(),
        }],
        experiments: vec![ExperimentFact {
            experiment_id,
            project_id,
            animal_ids: BTreeSet::from([animal_id]),
            status: "active".to_owned(),
            revision: 2,
        }],
        observations: vec![ObservationFact {
            observation_id,
            experiment_id,
            animal_id,
            value_digest: digest(b"23.4-g"),
            signed: false,
            revision: 1,
        }],
        measurements: vec![MeasurementFact {
            measurement_id,
            experiment_id,
            animal_id,
            value_digest: digest(b"23.4-g"),
            status: "signed".to_owned(),
            signed: true,
            revision: 2,
        }],
        samples: vec![SampleFact {
            sample_id,
            experiment_id,
            animal_id,
            status: "stored".to_owned(),
            revision: 1,
        }],
        attachments: vec![AttachmentFact {
            attachment_id,
            owner_entity_id: animal_id,
            size_bytes: 4,
            content_sha256: digest(b"file"),
        }],
        ai_history: AiHistoryFact {
            profiles: vec![AiProfileFact {
                profile_id,
                current_version: 1,
                version_digests: BTreeMap::from([(1, digest(b"synthetic-profile-v1"))]),
                archived: false,
            }],
            conversations: vec![AiConversationFact {
                conversation_id,
                project_id: Some(project_id),
                profile_id: Some(profile_id),
                profile_version: Some(1),
                message_ids: BTreeSet::from([user_message_id, assistant_message_id]),
                legacy_read_only: false,
                revision: 2,
            }],
            messages: vec![
                AiMessageFact {
                    message_id: user_message_id,
                    conversation_id,
                    sequence: 1,
                    role: "user".to_owned(),
                    content_digest: digest(b"synthetic question"),
                    response_digest: None,
                    revision: 1,
                },
                AiMessageFact {
                    message_id: assistant_message_id,
                    conversation_id,
                    sequence: 2,
                    role: "assistant".to_owned(),
                    content_digest: digest(b"synthetic answer"),
                    response_digest: Some(digest(b"synthetic response payload")),
                    revision: 1,
                },
            ],
            tool_runs: vec![AiToolRunFact {
                tool_run_id,
                conversation_id: Some(conversation_id),
                status: "completed".to_owned(),
                input_digest: digest(b"synthetic tool input"),
                output_digest: Some(digest(b"synthetic tool output")),
                revision: 2,
            }],
            approvals: vec![AiApprovalFact {
                approval_id: Uuid::new_v4(),
                tool_run_id,
                decision: "approved".to_owned(),
                revision: 2,
            }],
            jobs: vec![AiJobFact {
                job_id: Uuid::new_v4(),
                kind: "snapshot".to_owned(),
                status: "completed".to_owned(),
                revision: 2,
            }],
            conversation_ids: BTreeSet::from([conversation_id]),
            encrypted_envelope_count: 1,
            ciphertext_digests: BTreeSet::from([digest(b"synthetic-ciphertext")]),
            key_versions: BTreeSet::from([1]),
        },
        audit: AuditFact {
            minimum_entry_count: 5,
            entity_ids: BTreeSet::from([animal_id, experiment_id]),
            action_counts: BTreeMap::from([("create".to_owned(), 3), ("update".to_owned(), 2)]),
        },
        provenance: ProvenanceFact {
            minimum_record_count: 2,
            entity_ids: BTreeSet::from([animal_id, observation_id]),
            source_kinds: BTreeSet::from(["manual".to_owned(), "ai_approved".to_owned()]),
        },
        continuation: ContinuationExpectation {
            actor_user_id: user_id,
            animal_id,
            expected_previous_revision: 3,
            write_kind: "animal_note".to_owned(),
            expected_audit_delta: 1,
            expected_provenance_delta: 1,
        },
    }
}

fn write_component(
    root: &Path,
    path: &str,
    kind: FixtureComponentKind,
    bytes: &[u8],
) -> FixtureFile {
    let absolute = root.join(path);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(&absolute, bytes).unwrap();
    FixtureFile {
        path: path.to_owned(),
        kind,
        size_bytes: u64::try_from(bytes.len()).unwrap(),
        sha256: digest(bytes),
    }
}

fn synthetic_fixture(root: &Path) -> FixtureManifest {
    let fixture_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let identity = identity();
    let facts = synthetic_facts(fixture_id, identity.clone());
    let facts_bytes = serde_json::to_vec(&facts).unwrap();
    let generation = DeploymentGenerationManifest {
        format_version: GENERATION_MANIFEST_FORMAT,
        generation_id,
        data_epoch: identity.data_epoch.clone(),
        backend_state_digest: identity.backend_state_digest.clone(),
    };
    let generation_bytes = serde_json::to_vec(&generation).unwrap();
    let files = vec![
        write_component(
            root,
            "database/postgres.dump",
            FixtureComponentKind::Database,
            b"synthetic-postgres-dump",
        ),
        write_component(
            root,
            "attachments/file.bin",
            FixtureComponentKind::Attachments,
            b"file",
        ),
        write_component(
            root,
            "data/artifact.bin",
            FixtureComponentKind::DataArtifacts,
            b"synthetic-data-artifact",
        ),
        write_component(
            root,
            "config/deployment.json",
            FixtureComponentKind::Configuration,
            b"{\"synthetic\":true}",
        ),
        write_component(
            root,
            "keyset/synthetic.key",
            FixtureComponentKind::Keyset,
            b"synthetic-test-key-not-for-production",
        ),
        write_component(
            root,
            "ai/state.json",
            FixtureComponentKind::AiState,
            b"{\"encryptedEnvelopeCount\":1}",
        ),
        write_component(
            root,
            "data/deployment-generation.json",
            FixtureComponentKind::GenerationManifest,
            &generation_bytes,
        ),
        write_component(
            root,
            "expected-facts.json",
            FixtureComponentKind::ExpectedFacts,
            &facts_bytes,
        ),
    ];
    let manifest = FixtureManifest {
        format_version: FIXTURE_FORMAT_VERSION,
        fixture_id,
        backend: BackendKind::Postgres,
        release_identity: identity,
        generation_id,
        producer: FixtureProducerProvenance {
            generator_application_version: "0.1.0".parse().unwrap(),
            generator_data_epoch: "preview_epoch_0".parse().unwrap(),
            generator_backend_state_digest: format!("sha256:{}", "a".repeat(64)).parse().unwrap(),
            source_release_artifact_digest: digest(b"release-artifact"),
            source_release_provenance_digest: digest(b"release-provenance"),
            generated_at: Utc::now(),
        },
        expected_facts_digest: digest(&facts_bytes),
        files,
    };
    manifest.validate().unwrap();
    fs::write(
        root.join(FIXTURE_MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

#[test]
fn asset_verifier_rejects_extra_tampered_and_wrong_release_assets() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let expected_manifest_digest = fixture_manifest_digest(&manifest).unwrap();
    let (_, facts, result) =
        load_and_verify_fixture(directory.path(), Some(&expected_manifest_digest)).unwrap();
    assert_eq!(facts.fixture_id, manifest.fixture_id);
    assert_eq!(result.verified_file_count, 8);

    fs::write(directory.path().join("unregistered.txt"), b"unexpected").unwrap();
    assert!(matches!(
        load_and_verify_fixture(directory.path(), Some(&expected_manifest_digest)),
        Err(EvidenceError::AssetVerification { .. })
    ));
    fs::remove_file(directory.path().join("unregistered.txt")).unwrap();

    fs::write(directory.path().join("attachments/file.bin"), b"tampered").unwrap();
    assert!(matches!(
        load_and_verify_fixture(directory.path(), Some(&expected_manifest_digest)),
        Err(EvidenceError::AssetVerification { .. })
    ));

    let mut wrong = manifest;
    wrong.producer.generator_application_version = "9.0.0".parse().unwrap();
    assert!(matches!(
        wrong.validate(),
        Err(EvidenceError::WrongProducerRelease)
    ));
}

#[test]
fn fixture_paths_reject_parent_traversal() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest = synthetic_fixture(directory.path());
    manifest.files[0].path = "../outside.dump".to_owned();
    assert!(matches!(
        manifest.validate(),
        Err(EvidenceError::UnsafePath { .. })
    ));
}

#[test]
fn expected_facts_reject_broken_domain_and_ai_relationships() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let (_, facts, _) = load_and_verify_fixture(directory.path(), None).unwrap();

    let mut broken_domain = facts.clone();
    broken_domain.animals[2].sire_id = Some(Uuid::new_v4());
    assert!(matches!(
        broken_domain.validate(&manifest),
        Err(EvidenceError::ExpectedFactsIncomplete)
    ));

    let mut broken_ai = facts;
    broken_ai.ai_history.conversations[0].profile_version = Some(99);
    assert!(matches!(
        broken_ai.validate(&manifest),
        Err(EvidenceError::ExpectedFactsIncomplete)
    ));
}

#[cfg(unix)]
#[test]
fn asset_verifier_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let expected_manifest_digest = fixture_manifest_digest(&manifest).unwrap();
    let attachment = directory.path().join("attachments/file.bin");
    fs::remove_file(&attachment).unwrap();
    symlink(directory.path().join("data/artifact.bin"), attachment).unwrap();
    assert!(matches!(
        load_and_verify_fixture(directory.path(), Some(&expected_manifest_digest)),
        Err(EvidenceError::AssetVerification { .. })
    ));
}

#[test]
fn catalog_is_self_hashed_digest_pinned_and_append_only() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let artifact_digest = digest(b"oci-artifact");
    let reference = format!("ghcr.io/jarxunlai/muriarc-fixtures@{artifact_digest}");
    let mut catalog = FixtureCatalog::default();
    catalog
        .append(
            &manifest,
            artifact_digest,
            fixture_manifest_digest(&manifest).unwrap(),
            reference,
        )
        .unwrap();
    catalog.validate().unwrap();
    let previous = catalog.clone();
    catalog.entries[0].expected_facts_digest = digest(b"modified");
    assert!(catalog.validate().is_err());
    assert!(catalog.assert_append_only_from(&previous).is_err());
}

struct PassingAdapter {
    expected: Sha256Digest,
    skip: Arc<Mutex<Option<VerificationLayer>>>,
}

#[async_trait]
impl VerificationAdapter for PassingAdapter {
    async fn verify_layer(
        &self,
        layer: VerificationLayer,
        _context: &VerificationContext<'_>,
    ) -> Result<AdapterLayerEvidence, EvidenceError> {
        let status = if *self.skip.lock().await == Some(layer) {
            LayerStatus::Skip
        } else {
            LayerStatus::Pass
        };
        let state = digest(b"unchanged-state");
        Ok(AdapterLayerEvidence {
            status,
            evidence_digest: Some(digest(format!("{layer:?}").as_bytes())),
            observed_expected_facts_digest: Some(self.expected.clone()),
            state_digest_before: (layer == VerificationLayer::ReadOnlyNoSideEffects)
                .then(|| state.clone()),
            state_digest_after: (layer == VerificationLayer::ReadOnlyNoSideEffects)
                .then_some(state),
            continuation_write_verified: layer == VerificationLayer::ContinueWrite,
            detail_codes: Vec::new(),
            started_at: Utc::now(),
            completed_at: Utc::now() + Duration::milliseconds(1),
        })
    }
}

#[tokio::test]
async fn seven_layer_verifier_blocks_skip_and_exports_upgrade_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let expected = manifest.expected_facts_digest.clone();
    let adapter = PassingAdapter {
        expected: expected.clone(),
        skip: Arc::new(Mutex::new(None)),
    };
    let report = VerifierRunner::new(
        directory.path().to_path_buf(),
        Some(fixture_manifest_digest(&manifest).unwrap()),
        identity(),
        digest(b"final-native-package"),
        VerificationMode::Rc,
        DeliveryProfile::NativeSystem,
        ArtifactExecutionKind::FinalPackage,
        adapter,
    )
    .run()
    .await
    .unwrap();
    report.validate().unwrap();
    let candidate = Uuid::new_v4();
    let upgrade = report.to_upgrade_evidence(candidate).unwrap();
    assert_eq!(upgrade.generation_id, candidate);
    assert_eq!(upgrade.layers.len(), 7);

    let adapter = PassingAdapter {
        expected,
        skip: Arc::new(Mutex::new(Some(VerificationLayer::RemoteUi))),
    };
    assert!(matches!(
        VerifierRunner::new(
            directory.path().to_path_buf(),
            Some(fixture_manifest_digest(&manifest).unwrap()),
            identity(),
            digest(b"final-native-package"),
            VerificationMode::Rc,
            DeliveryProfile::NativeSystem,
            ArtifactExecutionKind::FinalPackage,
            adapter,
        )
        .run()
        .await,
        Err(EvidenceError::LayerFailed { .. })
    ));
}

#[test]
fn rc_matrix_rejects_empty_catalog_and_source_runs() {
    let definition = CompatibilityMatrixDefinition {
        format_version: 1,
        pr_profiles: BTreeSet::from([DeliveryProfile::ManagedCompose]),
        nightly_profiles: BTreeSet::from([
            DeliveryProfile::NativeSystem,
            DeliveryProfile::ManagedCompose,
            DeliveryProfile::DesktopWindows,
        ]),
        rc_profiles: BTreeSet::from([
            DeliveryProfile::NativeSystem,
            DeliveryProfile::ManagedCompose,
            DeliveryProfile::DesktopWindows,
        ]),
        rc_requires_all_catalog_entries: true,
        rc_requires_final_artifacts: true,
    };
    let report = CompatibilityMatrixReport {
        format_version: 1,
        mode: VerificationMode::Rc,
        selected_fixture_ids: BTreeSet::new(),
        runs: Vec::new(),
    };
    assert!(
        report
            .validate(&definition, &FixtureCatalog::default())
            .is_err()
    );

    let directory = tempfile::tempdir().unwrap();
    let manifest = synthetic_fixture(directory.path());
    let artifact_digest = digest(b"oci-artifact");
    let mut catalog = FixtureCatalog::default();
    catalog
        .append(
            &manifest,
            artifact_digest.clone(),
            fixture_manifest_digest(&manifest).unwrap(),
            format!("ghcr.io/jarxunlai/muriarc-fixtures@{artifact_digest}"),
        )
        .unwrap();
    let fixture_id = manifest.fixture_id;
    let source_report = CompatibilityMatrixReport {
        format_version: 1,
        mode: VerificationMode::Rc,
        selected_fixture_ids: BTreeSet::from([fixture_id]),
        runs: definition
            .rc_profiles
            .iter()
            .map(|profile| MatrixRun {
                fixture_id,
                profile: *profile,
                report_digest: Some(digest(format!("{profile:?}").as_bytes())),
                status: LayerStatus::Pass,
                execution_kind: ArtifactExecutionKind::SourceRun,
            })
            .collect(),
    };
    assert!(source_report.validate(&definition, &catalog).is_err());

    let weakened = CompatibilityMatrixDefinition {
        rc_requires_all_catalog_entries: false,
        ..definition
    };
    assert!(source_report.validate(&weakened, &catalog).is_err());
}
