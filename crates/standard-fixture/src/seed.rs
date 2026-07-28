use std::{collections::BTreeMap, fs, path::Path};

use chrono::{DateTime, Utc};
use muriarc_application::{
    AssignAnimalsToProjectCommand, CreateAlleleCommand, CreateAnimalCommand,
    CreateAnimalDraftInput, CreateAnimalIdentifierScope, CreateBreedingLineCommand,
    CreateBreedingPairCommand, CreateCageCommand, CreateCohortCommand, CreateColonyCommand,
    CreateExperimentCommand, CreateExperimentEventCommand, CreateGeneLocusCommand,
    CreateGenotypeComponentInput, CreateGenotypeDefinitionCommand, CreateGenotypingRecordCommand,
    CreateLitterCommand, CreateMatingEventCommand, CreateMeasurementCommand,
    CreateObservationDefinitionCommand, CreateParticipationCommand, CreateProcedureCommand,
    CreateProjectCommand, CreateSampleCommand, CreateTemplateVersionCommand,
    PublishTemplateVersionCommand, RecordObservationCommand, RegisterAnimalDraftCommand,
    ReviseObservationValueCommand, SignMeasurementCommand, TransferAnimalsCommand,
    TransitionExperimentCommand, TransitionParticipationCommand, assign_animals_to_project,
    create_allele, create_animal, create_breeding_line, create_breeding_pair, create_cage,
    create_cohort, create_colony, create_experiment, create_experiment_event, create_gene_locus,
    create_genotype_definition, create_genotyping_record, create_litter, create_mating_event,
    create_measurement, create_observation_definition, create_participation, create_procedure,
    create_project, create_sample, create_template_version, publish_template_version,
    record_observation, register_animal_draft, retire_breeding_pair, revise_observation_value,
    sign_measurement, transfer_animals, transition_experiment, transition_participation,
};
use muriarc_core::{
    Actor, Attachment, AuditContext, DeploymentGenerationManifest, ExperimentStatus,
    GenotypeComponentMode, LOCAL_LAB_ID, LOCAL_OPERATOR_NAME, LOCAL_USER_ID, Lab, MuriArcStore,
    RecordMeta, RecordStatus, User, WriteSource,
};
use muriarc_data::AttachmentFiles;
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    DATABASE_FILE, FixtureBundle, FixtureIds, FixtureResult, GENERATION_MANIFEST_FILE,
    RECEIPT_FILE, SeedReceipt, ensure, invalid, sha256, verify,
};

pub(super) async fn seed_into(
    bundle: &FixtureBundle,
    root: &Path,
    source_commit: &str,
) -> FixtureResult<SeedReceipt> {
    let database = root.join(DATABASE_FILE);
    let attachments = AttachmentFiles::new(root.join("attachments"));
    attachments.initialize().await?;
    fs::create_dir(root.join("data"))?;

    let store = SqliteStore::connect_path(&database).await?;
    let result = async {
        store.migrate().await?;
        let migration_report = store.compatibility_report().await?;
        let deployment = migration_report
            .require_compatible()
            .map_err(invalid)?
            .clone();
        let generation_id = deployment.generation_id;
        write_json_atomic(
            &root.join(GENERATION_MANIFEST_FILE),
            &DeploymentGenerationManifest::from_state(&deployment),
        )?;

        bootstrap_local_identity(&store, bundle.dataset.fixed_timeline.starts_at).await?;
        let audit = AuditContext {
            actor: Actor::human(LOCAL_USER_ID, LOCAL_OPERATOR_NAME),
            source: WriteSource::Desktop,
            request_id: Some(format!("standard-v1-{}", &bundle.dataset_sha256[..16])),
            reason: Some("desktop-standard-v1-fixture".to_owned()),
        };
        let mut ids = FixtureIds::default();
        seed_projects_and_cages(bundle, &store, &audit, &mut ids).await?;
        seed_direct_animals(bundle, &store, &audit, &mut ids).await?;
        seed_genetics_and_breeding(bundle, &store, &audit, &mut ids).await?;
        seed_experiments(bundle, &store, &audit, &mut ids).await?;
        seed_records(bundle, &store, &audit, &mut ids).await?;
        seed_attachments(bundle, &store, &attachments, &audit, &mut ids).await?;
        apply_terminal_states(bundle, &store, &audit, &ids).await?;
        assert_id_counts(bundle, &ids)?;

        let report = store.compatibility_report().await?;
        ensure(
            report.is_compatible(),
            format!(
                "seeded database is not compatible: {}",
                report
                    .issues
                    .iter()
                    .map(|issue| issue.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )?;
        let receipt = SeedReceipt {
            schema_version: 1,
            status: "PASS".to_owned(),
            dataset_id: bundle.dataset.dataset_id.clone(),
            dataset_version: bundle.manifest.dataset_version.clone(),
            dataset_sha256: bundle.dataset_sha256.clone(),
            manifest_sha256: bundle.manifest_sha256.clone(),
            source_commit: source_commit.to_owned(),
            application_version: deployment.identity.application_version.as_str().to_owned(),
            data_epoch: deployment.identity.data_epoch.as_str().to_owned(),
            backend: "sqlite".to_owned(),
            generation_id,
            expected_counts: bundle.dataset.expected_counts.clone(),
            attachment_files: bundle.manifest.files.clone(),
            ids,
        };
        write_json_atomic(&root.join(RECEIPT_FILE), &receipt)?;
        verify::verify(bundle, root, source_commit).await?;
        Ok(receipt)
    }
    .await;
    // Dropping SqlitePool closes connections in the background. Windows keeps
    // the database file handle open long enough to make the caller's atomic
    // staging-directory rename fail with ERROR_ACCESS_DENIED. Await the close
    // on every return path instead of relying on asynchronous Drop cleanup.
    store.pool().close().await;
    result
}

async fn bootstrap_local_identity(store: &SqliteStore, now: DateTime<Utc>) -> FixtureResult<()> {
    let audit = AuditContext::system(WriteSource::Desktop);
    let mut lab = Lab::new("MuriArc standard-v1 合成实验室", now)?;
    lab.id = LOCAL_LAB_ID;
    store.create_lab(&lab, &audit).await?;
    let mut user = User::new(
        LOCAL_LAB_ID,
        "standard.operator@muriarc.invalid",
        LOCAL_OPERATOR_NAME,
        now,
    )?;
    user.id = LOCAL_USER_ID;
    store.create_user(&user, &audit).await?;
    Ok(())
}

async fn seed_projects_and_cages(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.starts_at;
    for spec in &bundle.dataset.projects {
        let project = create_project(
            store,
            CreateProjectCommand {
                lab_id: LOCAL_LAB_ID,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                now,
            },
            audit,
        )
        .await?;
        ids.projects.insert(spec.key.clone(), project.id);
    }
    for spec in &bundle.dataset.cages {
        let cage = create_cage(
            store,
            CreateCageCommand {
                lab_id: LOCAL_LAB_ID,
                section: spec.section.clone(),
                display_id: spec.display_id.clone(),
                location: Some(spec.location.clone()),
                kind: spec.kind,
                capacity: spec.capacity,
                sort_order: spec.sort_order,
                now,
            },
            audit,
        )
        .await?;
        ids.cages.insert(spec.key.clone(), cage.id);
    }
    Ok(())
}

async fn seed_direct_animals(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.starts_at;
    let transferred_at = DateTime::parse_from_rfc3339("2025-01-01T02:00:00Z")?.with_timezone(&Utc);
    for spec in bundle
        .dataset
        .animals
        .iter()
        .filter(|value| value.source != "litter")
    {
        ensure(
            matches!(spec.source.as_str(), "direct" | "minimal"),
            "direct animal has an unknown source",
        )?;
        let animal = create_animal(
            store,
            CreateAnimalCommand {
                lab_id: LOCAL_LAB_ID,
                identifier_scope: CreateAnimalIdentifierScope::Lab,
                display_id: spec.display_id.clone(),
                sex: spec.sex,
                strain: Some(spec.strain.clone()),
                birth_date: Some(spec.birth_date),
                legacy_id: spec.legacy_id.clone(),
                initial_cage_id: None,
                initial_genotyping_records: Vec::new(),
                now,
            },
            audit,
        )
        .await?;
        ids.animals.insert(spec.key.clone(), animal.id);
        assign_animal(bundle, store, audit, ids, &spec.key, &spec.project, now).await?;
        let cage_id = lookup(&ids.cages, &spec.cage, "cage")?;
        transfer_animals(
            store,
            TransferAnimalsCommand {
                lab_id: LOCAL_LAB_ID,
                animal_ids: vec![animal.id],
                target_cage_id: cage_id,
                occurred_at: transferred_at,
                recorded_at: transferred_at,
                recorded_by: Some(LOCAL_USER_ID),
                notes: Some("standard-v1 初始分笼".to_owned()),
            },
            audit,
        )
        .await?;
    }
    Ok(())
}

async fn seed_genetics_and_breeding(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.starts_at;
    let locus_spec = &bundle.dataset.genetics.locus;
    let locus = create_gene_locus(
        store,
        CreateGeneLocusCommand {
            lab_id: LOCAL_LAB_ID,
            symbol: locus_spec.symbol.clone(),
            description: Some(locus_spec.description.clone()),
            now,
        },
        audit,
    )
    .await?;
    ids.loci.insert(locus_spec.key.clone(), locus.id);
    for spec in &bundle.dataset.genetics.alleles {
        let allele = create_allele(
            store,
            CreateAlleleCommand {
                locus_id: locus.id,
                symbol: spec.symbol.clone(),
                description: Some(spec.description.clone()),
                is_wild_type: spec.is_wild_type,
                now,
            },
            audit,
        )
        .await?;
        ids.alleles.insert(spec.key.clone(), allele.id);
    }
    for spec in &bundle.dataset.genetics.definitions {
        ensure(
            spec.alleles.len() == 2,
            "standard genotype definition must be diploid",
        )?;
        let definition = create_genotype_definition(
            store,
            CreateGenotypeDefinitionCommand {
                lab_id: LOCAL_LAB_ID,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                components: vec![CreateGenotypeComponentInput {
                    locus_id: locus.id,
                    allele_1_id: lookup(&ids.alleles, &spec.alleles[0], "allele")?,
                    allele_2_id: Some(lookup(&ids.alleles, &spec.alleles[1], "allele")?),
                    mode: GenotypeComponentMode::Diploid,
                    display_order: 0,
                }],
                now,
            },
            audit,
        )
        .await?;
        ids.genotype_definitions
            .insert(spec.key.clone(), definition.id);
    }

    let line_spec = &bundle.dataset.breeding.line;
    let line = create_breeding_line(
        store,
        CreateBreedingLineCommand {
            lab_id: LOCAL_LAB_ID,
            name: line_spec.name.clone(),
            description: Some(line_spec.description.clone()),
            genotype_definition_ids: line_spec
                .definitions
                .iter()
                .map(|key| lookup(&ids.genotype_definitions, key, "genotype definition"))
                .collect::<FixtureResult<Vec<_>>>()?,
            now,
        },
        audit,
    )
    .await?;
    ids.breeding_lines.insert(line_spec.key.clone(), line.id);
    for spec in &bundle.dataset.breeding.colonies {
        let colony = create_colony(
            store,
            CreateColonyCommand {
                lab_id: LOCAL_LAB_ID,
                breeding_line_id: line.id,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                now,
            },
            audit,
        )
        .await?;
        ids.colonies.insert(spec.key.clone(), colony.id);
    }
    for spec in &bundle.dataset.breeding.pairs {
        let pair = create_breeding_pair(
            store,
            CreateBreedingPairCommand {
                lab_id: LOCAL_LAB_ID,
                colony_id: lookup(&ids.colonies, &spec.colony, "colony")?,
                name: spec.name.clone(),
                male_animal_id: lookup(&ids.animals, &spec.male, "male animal")?,
                female_animal_ids: spec
                    .females
                    .iter()
                    .map(|key| lookup(&ids.animals, key, "female animal"))
                    .collect::<FixtureResult<Vec<_>>>()?,
                started_at: spec.started_at,
                now,
            },
            audit,
        )
        .await?;
        ids.breeding_pairs.insert(spec.key.clone(), pair.id);
    }
    let mating_spec = &bundle.dataset.breeding.mating;
    let mating = create_mating_event(
        store,
        CreateMatingEventCommand {
            lab_id: LOCAL_LAB_ID,
            breeding_pair_id: lookup(&ids.breeding_pairs, &mating_spec.pair, "breeding pair")?,
            male_animal_id: lookup(&ids.animals, &mating_spec.male, "male animal")?,
            female_animal_id: lookup(&ids.animals, &mating_spec.female, "female animal")?,
            occurred_at: mating_spec.occurred_at,
            notes: Some(mating_spec.notes.clone()),
            now,
        },
        audit,
    )
    .await?;
    ids.mating_events.insert(mating_spec.key.clone(), mating.id);

    let litter_spec = &bundle.dataset.breeding.litter;
    let created = create_litter(
        store,
        CreateLitterCommand {
            lab_id: LOCAL_LAB_ID,
            mating_event_id: lookup(&ids.mating_events, &litter_spec.mating, "mating event")?,
            born_on: litter_spec.born_on,
            size_total: litter_spec.size_total,
            drafts: litter_spec
                .drafts
                .iter()
                .map(|draft| CreateAnimalDraftInput {
                    temporary_label: draft.temporary_label.clone(),
                    sex: draft.sex,
                })
                .collect(),
            notes: Some(litter_spec.notes.clone()),
            now,
        },
        audit,
    )
    .await?;
    ids.litters
        .insert(litter_spec.key.clone(), created.litter.id);
    let drafts = created
        .drafts
        .into_iter()
        .map(|draft| (draft.temporary_label.clone(), draft))
        .collect::<BTreeMap<_, _>>();
    for draft_spec in &litter_spec.drafts {
        let draft = drafts
            .get(&draft_spec.temporary_label)
            .ok_or_else(|| invalid("created litter is missing a draft"))?;
        ids.animal_drafts
            .insert(draft_spec.temporary_label.clone(), draft.id);
        let Some(animal_key) = &draft_spec.animal else {
            continue;
        };
        let animal_spec = bundle
            .dataset
            .animals
            .iter()
            .find(|value| &value.key == animal_key)
            .ok_or_else(|| invalid("litter draft references an unknown animal"))?;
        ensure(
            animal_spec.source == "litter"
                && animal_spec.temporary_label.as_deref() == Some(&draft_spec.temporary_label),
            "litter animal source metadata differs",
        )?;
        let project_id = lookup(&ids.projects, &animal_spec.project, "project")?;
        let registered = register_animal_draft(
            store,
            RegisterAnimalDraftCommand {
                lab_id: LOCAL_LAB_ID,
                draft_id: draft.id,
                expected_revision: draft.meta.revision,
                identifier_scope: CreateAnimalIdentifierScope::Project(project_id),
                display_id: animal_spec.display_id.clone(),
                strain: Some(animal_spec.strain.clone()),
                initial_cage_id: Some(lookup(&ids.cages, &animal_spec.cage, "cage")?),
                now,
            },
            audit,
        )
        .await?;
        ensure(
            registered.animal.birth_date == Some(animal_spec.birth_date),
            "registered litter animal birth date differs",
        )?;
        ids.animals
            .insert(animal_spec.key.clone(), registered.animal.id);
        assign_animal(
            bundle,
            store,
            audit,
            ids,
            &animal_spec.key,
            &animal_spec.project,
            now,
        )
        .await?;
    }
    ensure(
        ids.animals.len() == bundle.dataset.animals.len(),
        "litter registration did not materialize every animal",
    )?;

    for spec in &bundle.dataset.genetics.records {
        let animal_spec = bundle
            .dataset
            .animals
            .iter()
            .find(|value| value.key == spec.animal)
            .ok_or_else(|| invalid("genotyping record references an unknown animal"))?;
        let record = create_genotyping_record(
            store,
            CreateGenotypingRecordCommand {
                lab_id: LOCAL_LAB_ID,
                project_id: Some(lookup(&ids.projects, &animal_spec.project, "project")?),
                animal_id: lookup(&ids.animals, &spec.animal, "animal")?,
                genotype_definition_id: lookup(
                    &ids.genotype_definitions,
                    &spec.definition,
                    "genotype definition",
                )?,
                state: spec.state,
                assessed_at: spec.assessed_at,
                method: Some(spec.method.clone()),
                notes: Some(spec.notes.clone()),
                now,
            },
            audit,
        )
        .await?;
        ids.genotyping_records.insert(spec.key.clone(), record.id);
    }
    Ok(())
}

async fn seed_experiments(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.starts_at;
    for spec in &bundle.dataset.templates {
        let template = create_template_version(
            store,
            CreateTemplateVersionCommand {
                lab_id: LOCAL_LAB_ID,
                template_key: spec.template_key.clone(),
                version: spec.version,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                fields: spec.fields.clone(),
                now,
            },
            audit,
        )
        .await?;
        let template = if spec.status == muriarc_core::TemplateStatus::Published {
            publish_template_version(
                store,
                PublishTemplateVersionCommand {
                    id: template.id,
                    expected_revision: template.meta.revision,
                    published_by: LOCAL_USER_ID,
                    published_at: now,
                },
                audit,
            )
            .await?
        } else {
            template
        };
        ensure(
            template.status == spec.status,
            "template terminal status differs",
        )?;
        ids.templates.insert(spec.key.clone(), template.id);
        let _ = lookup(&ids.projects, &spec.project, "template project")?;
    }
    for spec in &bundle.dataset.experiments {
        let experiment = create_experiment(
            store,
            CreateExperimentCommand {
                lab_id: LOCAL_LAB_ID,
                project_id: lookup(&ids.projects, &spec.project, "project")?,
                template_version_id: spec
                    .template
                    .as_ref()
                    .map(|key| lookup(&ids.templates, key, "template"))
                    .transpose()?,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                starts_at: None,
                now,
            },
            audit,
        )
        .await?;
        ids.experiments.insert(spec.key.clone(), experiment.id);
    }
    for spec in &bundle.dataset.cohorts {
        let cohort = create_cohort(
            store,
            CreateCohortCommand {
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                name: spec.name.clone(),
                description: Some(spec.description.clone()),
                now,
            },
            audit,
        )
        .await?;
        ids.cohorts.insert(spec.key.clone(), cohort.id);
    }
    for spec in &bundle.dataset.participations {
        let participation = create_participation(
            store,
            CreateParticipationCommand {
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                animal_id: lookup(&ids.animals, &spec.animal, "animal")?,
                cohort_id: Some(lookup(&ids.cohorts, &spec.cohort, "cohort")?),
                enrolled_at: spec.enrolled_at,
            },
            audit,
        )
        .await?;
        ids.participations
            .insert(spec.key.clone(), participation.id);
    }
    for spec in &bundle.dataset.procedures {
        let procedure = create_procedure(
            store,
            CreateProcedureCommand {
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                animal_id: spec
                    .animal
                    .as_ref()
                    .map(|key| lookup(&ids.animals, key, "animal"))
                    .transpose()?,
                name: spec.name.clone(),
                scheduled_at: Some(spec.scheduled_at),
                performed_at: spec.performed_at,
                status: spec.status,
                details: spec.details.clone(),
                now,
            },
            audit,
        )
        .await?;
        ids.procedures.insert(spec.key.clone(), procedure.id);
    }
    Ok(())
}

async fn seed_records(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.starts_at;
    for spec in &bundle.dataset.events {
        let project_id = experiment_project(bundle, ids, &spec.experiment)?;
        let event = create_experiment_event(
            store,
            CreateExperimentEventCommand {
                lab_id: LOCAL_LAB_ID,
                project_id,
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                event_key: spec.event_key.clone(),
                label: spec.label.clone(),
                occurred_at: spec.occurred_at,
                details: spec.details.clone(),
                now,
            },
            audit,
        )
        .await?;
        ids.events.insert(spec.key.clone(), event.id);
    }
    for spec in &bundle.dataset.observation_definitions {
        let project_id = experiment_project(bundle, ids, &spec.experiment)?;
        let definition = create_observation_definition(
            store,
            CreateObservationDefinitionCommand {
                lab_id: LOCAL_LAB_ID,
                project_id,
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                key: spec.definition_key.clone(),
                label: spec.label.clone(),
                value_type: spec.value_type,
                unit: spec.unit.clone(),
                categories: spec.categories.clone(),
                policy: spec.policy,
                now,
            },
            audit,
        )
        .await?;
        ids.observation_definitions
            .insert(spec.key.clone(), definition.id);
    }
    for spec in &bundle.dataset.observations {
        let project_id = experiment_project(bundle, ids, &spec.experiment)?;
        let recorded = record_observation(
            store,
            RecordObservationCommand {
                lab_id: LOCAL_LAB_ID,
                project_id,
                experiment_id: lookup(&ids.experiments, &spec.experiment, "experiment")?,
                experiment_event_id: lookup(&ids.events, &spec.event, "experiment event")?,
                definition_id: lookup(
                    &ids.observation_definitions,
                    &spec.definition,
                    "observation definition",
                )?,
                subject_type: spec.subject_type,
                subject_id: lookup(&ids.animals, &spec.animal, "animal")?,
                context: spec.context.clone(),
                value: spec.value.clone(),
                recorded_at: spec.recorded_at,
                recorded_by: Some(LOCAL_USER_ID),
                notes: Some(spec.notes.clone()),
                now,
            },
            audit,
        )
        .await?;
        let observation = if let Some(revision) = &spec.revision {
            revise_observation_value(
                store,
                ReviseObservationValueCommand {
                    observation_id: recorded.observation.id,
                    expected_revision: recorded.observation.meta.revision,
                    value: revision.value.clone(),
                    recorded_at: revision.recorded_at,
                    recorded_by: Some(LOCAL_USER_ID),
                    notes: Some(revision.notes.clone()),
                    now: revision.recorded_at,
                },
                audit,
            )
            .await?
            .observation
        } else {
            recorded.observation
        };
        ids.observations.insert(spec.key.clone(), observation.id);
    }
    for spec in &bundle.dataset.measurements {
        let measurement = create_measurement(
            store,
            CreateMeasurementCommand {
                lab_id: LOCAL_LAB_ID,
                project_id: lookup(&ids.projects, &spec.project, "project")?,
                experiment_id: Some(lookup(&ids.experiments, &spec.experiment, "experiment")?),
                animal_id: lookup(&ids.animals, &spec.animal, "animal")?,
                procedure_id: None,
                key: spec.measurement_key.clone(),
                label: spec.label.clone(),
                value: spec.value.clone(),
                unit: spec.unit.clone(),
                measured_at: spec.measured_at,
                now,
            },
            audit,
        )
        .await?;
        let measurement = if spec.status == RecordStatus::Signed {
            sign_measurement(
                store,
                SignMeasurementCommand {
                    id: measurement.id,
                    expected_revision: measurement.meta.revision,
                    signed_by: LOCAL_USER_ID,
                    signed_at: bundle.dataset.fixed_timeline.ends_at,
                },
                audit,
            )
            .await?
        } else {
            measurement
        };
        ensure(
            measurement.status == spec.status,
            "measurement status differs",
        )?;
        ids.measurements.insert(spec.key.clone(), measurement.id);
    }
    for spec in &bundle.dataset.samples {
        let sample = create_sample(
            store,
            CreateSampleCommand {
                lab_id: LOCAL_LAB_ID,
                project_id: lookup(&ids.projects, &spec.project, "project")?,
                experiment_id: Some(lookup(&ids.experiments, &spec.experiment, "experiment")?),
                animal_id: lookup(&ids.animals, &spec.animal, "animal")?,
                sample_type: spec.sample_type.clone(),
                quantity: Some(spec.quantity),
                unit: Some(spec.unit.clone()),
                location: Some(spec.location.clone()),
                collected_at: spec.collected_at,
                now,
            },
            audit,
        )
        .await?;
        ids.samples.insert(spec.key.clone(), sample.id);
    }
    Ok(())
}

async fn seed_attachments(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    files: &AttachmentFiles,
    audit: &AuditContext,
    ids: &mut FixtureIds,
) -> FixtureResult<()> {
    let now = bundle.dataset.fixed_timeline.ends_at;
    for spec in &bundle.dataset.attachments {
        let bytes = fs::read(bundle.root.join("files").join(&spec.file))?;
        let expected = bundle
            .manifest
            .files
            .get(&spec.file)
            .ok_or_else(|| invalid("attachment is missing from manifest"))?;
        ensure(
            sha256(&bytes) == *expected,
            "attachment digest changed before write",
        )?;
        let id = Uuid::new_v4();
        let object = files.write_bytes(id, &bytes).await?;
        let (target_type, key) = spec
            .target
            .split_once(':')
            .ok_or_else(|| invalid("attachment target must be kind:key"))?;
        ensure(
            target_type == spec.target_type,
            "attachment target type differs",
        )?;
        let entity_id = match target_type {
            "project" => lookup(&ids.projects, key, "attachment project target")?,
            "animal" => lookup(&ids.animals, key, "attachment animal target")?,
            "experiment" => lookup(&ids.experiments, key, "attachment experiment target")?,
            "measurement" => lookup(&ids.measurements, key, "attachment measurement target")?,
            "sample" => lookup(&ids.samples, key, "attachment sample target")?,
            _ => return Err(invalid("unsupported attachment target type").into()),
        };
        let attachment = Attachment {
            id,
            lab_id: LOCAL_LAB_ID,
            project_id: Some(lookup(&ids.projects, &spec.project, "attachment project")?),
            entity_type: target_type.to_owned(),
            entity_id,
            file_name: spec.file.clone(),
            media_type: Some(spec.media_type.clone()),
            relative_path: object.relative_path.clone(),
            size_bytes: object.size_bytes,
            sha256: object.sha256.clone(),
            version: 1,
            meta: RecordMeta::new(now),
        };
        if let Err(error) = store.create_attachment(&attachment, audit).await {
            files.remove_installed_object(&object).await?;
            return Err(error.into());
        }
        ids.attachments.insert(spec.key.clone(), id);
    }
    Ok(())
}

async fn apply_terminal_states(
    bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &FixtureIds,
) -> FixtureResult<()> {
    let occurred_at = bundle.dataset.fixed_timeline.ends_at;
    for spec in &bundle.dataset.participations {
        if spec.status == muriarc_core::ParticipationStatus::Enrolled {
            continue;
        }
        let current = store
            .get_participation(lookup(&ids.participations, &spec.key, "participation")?)
            .await?;
        let closed = transition_participation(
            store,
            TransitionParticipationCommand {
                id: current.id,
                target: spec.status,
                expected_revision: current.meta.revision,
                occurred_at,
            },
            audit,
        )
        .await?;
        ensure(
            closed.status == spec.status,
            "participation terminal status differs",
        )?;
    }
    for spec in &bundle.dataset.experiments {
        let target = match spec.status.as_str() {
            "draft" => continue,
            "completed" => ExperimentStatus::Completed,
            "cancelled" => ExperimentStatus::Cancelled,
            _ => return Err(invalid("unsupported standard experiment status").into()),
        };
        let current = store
            .get_experiment(lookup(&ids.experiments, &spec.key, "experiment")?)
            .await?;
        let closed = transition_experiment(
            store,
            TransitionExperimentCommand {
                id: current.id,
                target,
                expected_revision: current.meta.revision,
                occurred_at,
            },
            audit,
        )
        .await?;
        ensure(
            closed.status == target,
            "experiment terminal status differs",
        )?;
    }
    for spec in &bundle.dataset.breeding.pairs {
        match spec.status.as_str() {
            "active" => ensure(spec.ended_at.is_none(), "active pair must not have endedAt")?,
            "retired" => {
                let current = store
                    .get_breeding_pair(lookup(&ids.breeding_pairs, &spec.key, "breeding pair")?)
                    .await?;
                let ended_at = spec
                    .ended_at
                    .ok_or_else(|| invalid("retired pair is missing endedAt"))?;
                let retired =
                    retire_breeding_pair(store, current.id, current.meta.revision, ended_at, audit)
                        .await?;
                ensure(
                    retired.status == muriarc_core::BreedingPairStatus::Retired,
                    "breeding pair terminal status differs",
                )?;
            }
            _ => return Err(invalid("unsupported breeding pair status").into()),
        }
    }
    Ok(())
}

async fn assign_animal(
    _bundle: &FixtureBundle,
    store: &SqliteStore,
    audit: &AuditContext,
    ids: &mut FixtureIds,
    animal_key: &str,
    project_key: &str,
    now: DateTime<Utc>,
) -> FixtureResult<()> {
    let assignment = assign_animals_to_project(
        store,
        AssignAnimalsToProjectCommand {
            lab_id: LOCAL_LAB_ID,
            project_id: lookup(&ids.projects, project_key, "project")?,
            animal_ids: vec![lookup(&ids.animals, animal_key, "animal")?],
            assigned_by: Some(LOCAL_USER_ID),
            reason: Some(format!("standard-v1:{project_key}")),
            now,
        },
        audit,
    )
    .await?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("project assignment was not created"))?;
    ids.assignments.insert(animal_key.to_owned(), assignment.id);
    Ok(())
}

fn experiment_project(
    bundle: &FixtureBundle,
    ids: &FixtureIds,
    experiment_key: &str,
) -> FixtureResult<Uuid> {
    let project_key = bundle
        .dataset
        .experiments
        .iter()
        .find(|value| value.key == experiment_key)
        .map(|value| value.project.as_str())
        .ok_or_else(|| invalid("unknown experiment key"))?;
    lookup(&ids.projects, project_key, "experiment project")
}

fn assert_id_counts(bundle: &FixtureBundle, ids: &FixtureIds) -> FixtureResult<()> {
    let checks = [
        ("projects", ids.projects.len()),
        ("cages", ids.cages.len()),
        ("animals", ids.animals.len()),
        ("genotypingRecords", ids.genotyping_records.len()),
        ("breedingLines", ids.breeding_lines.len()),
        ("colonies", ids.colonies.len()),
        ("breedingPairs", ids.breeding_pairs.len()),
        ("matingEvents", ids.mating_events.len()),
        ("litters", ids.litters.len()),
        ("animalDrafts", ids.animal_drafts.len()),
        ("templateVersions", ids.templates.len()),
        ("experiments", ids.experiments.len()),
        ("cohorts", ids.cohorts.len()),
        ("participations", ids.participations.len()),
        ("procedures", ids.procedures.len()),
        ("experimentEvents", ids.events.len()),
        ("observationDefinitions", ids.observation_definitions.len()),
        ("observations", ids.observations.len()),
        ("measurements", ids.measurements.len()),
        ("samples", ids.samples.len()),
        ("attachments", ids.attachments.len()),
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
            format!("seeded id count differs for {key}: expected {expected}, got {actual}"),
        )?;
    }
    ensure(
        ids.assignments.len() == bundle.dataset.animals.len(),
        "every fixture animal must have exactly one project assignment",
    )?;
    ensure(
        ids.loci.len() == 1,
        "fixture must contain exactly one gene locus",
    )?;
    ensure(
        ids.alleles.len() == bundle.dataset.genetics.alleles.len(),
        "allele id count differs",
    )?;
    ensure(
        ids.genotype_definitions.len() == bundle.dataset.genetics.definitions.len(),
        "genotype definition id count differs",
    )?;
    Ok(())
}

fn lookup(map: &BTreeMap<String, Uuid>, key: &str, label: &str) -> FixtureResult<Uuid> {
    map.get(key)
        .copied()
        .ok_or_else(|| invalid(format!("unknown {label} key: {key}")).into())
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> FixtureResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("JSON output path has no parent"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("json"),
        Uuid::new_v4()
    ));
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}
