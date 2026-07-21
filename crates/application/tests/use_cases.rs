use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateAlleleCommand, CreateExperimentCommand, CreateGeneLocusCommand, CreateGenotypeCommand,
    CreateMeasurementCommand, CreateParticipationCommand, CreateProjectCommand,
    CreateSampleCommand, create_allele, create_experiment, create_gene_locus, create_genotype,
    create_measurement, create_participation, create_project, create_sample,
};
use muriarc_core::{Animal, AuditContext, Lab, MeasurementValue, MuriArcStore, Sex, WriteSource};
use muriarc_store_sqlite::SqliteStore;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn shared_research_use_cases_build_one_traceable_chain() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Application research test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = create_project(
        &store,
        CreateProjectCommand {
            lab_id: lab.id,
            name: "  PH study  ".to_owned(),
            description: Some("  longitudinal cohort  ".to_owned()),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let experiment = create_experiment(
        &store,
        CreateExperimentCommand {
            lab_id: lab.id,
            project_id: project.id,
            template_version_id: None,
            name: "  intervention  ".to_owned(),
            description: None,
            starts_at: Some(now),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let animal = Animal::new_mouse(lab.id, "M-001", Sex::Female, now).unwrap();
    store.create_animal(&animal, &audit).await.unwrap();
    create_participation(
        &store,
        CreateParticipationCommand {
            experiment_id: experiment.id,
            animal_id: animal.id,
            cohort_id: None,
            enrolled_at: now,
        },
        &audit,
    )
    .await
    .unwrap();

    let measurement = create_measurement(
        &store,
        CreateMeasurementCommand {
            lab_id: lab.id,
            project_id: project.id,
            experiment_id: Some(experiment.id),
            animal_id: animal.id,
            procedure_id: None,
            key: "  weight  ".to_owned(),
            label: "  Body weight  ".to_owned(),
            value: MeasurementValue::Number(24.5),
            unit: Some("  g  ".to_owned()),
            measured_at: now,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(measurement.unit.as_deref(), Some("g"));

    let sample = create_sample(
        &store,
        CreateSampleCommand {
            lab_id: lab.id,
            project_id: project.id,
            experiment_id: Some(experiment.id),
            animal_id: animal.id,
            sample_type: "  plasma  ".to_owned(),
            quantity: Some(100.0),
            unit: Some("  uL  ".to_owned()),
            location: Some("  freezer-1  ".to_owned()),
            collected_at: now,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(sample.sample_type, "plasma");

    let locus = create_gene_locus(
        &store,
        CreateGeneLocusCommand {
            lab_id: lab.id,
            symbol: "  Rosa26  ".to_owned(),
            description: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let allele = create_allele(
        &store,
        CreateAlleleCommand {
            locus_id: locus.id,
            symbol: "  flox  ".to_owned(),
            description: None,
            is_wild_type: false,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let genotype = create_genotype(
        &store,
        CreateGenotypeCommand {
            animal_id: animal.id,
            locus_id: locus.id,
            allele_1_id: Some(allele.id),
            allele_2_id: Some(allele.id),
            assessed_at: Some(now),
            project_id: Some(project.id),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(genotype.allele_1_id, Some(allele.id));
}
