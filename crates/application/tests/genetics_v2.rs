use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateAlleleCommand, CreateExperimentCommand, CreateGeneLocusCommand,
    CreateGenotypeComponentInput, CreateGenotypeDefinitionCommand, CreateGenotypingRecordCommand,
    CreateParticipationCommand, CreateProjectCommand, create_allele, create_experiment,
    create_gene_locus, create_genotype_definition, create_genotyping_record, create_participation,
    create_project,
};
use muriarc_core::{
    Animal, AuditContext, GenotypeComponentMode, GenotypingState, Lab, MuriArcStore, Sex,
    WriteSource,
};
use muriarc_store_sqlite::SqliteStore;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T16:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn confirmed_genotyping_record_is_captured_on_experiment_enrollment() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Genetics v2 application test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = create_project(
        &store,
        CreateProjectCommand {
            lab_id: lab.id,
            name: "Snapshot study".to_owned(),
            description: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let animal = Animal::new_mouse(lab.id, "G-001", Sex::Male, now).unwrap();
    store.create_animal(&animal, &audit).await.unwrap();

    let locus = create_gene_locus(
        &store,
        CreateGeneLocusCommand {
            lab_id: lab.id,
            symbol: "Tek".to_owned(),
            description: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let wild_type = create_allele(
        &store,
        CreateAlleleCommand {
            locus_id: locus.id,
            symbol: "+".to_owned(),
            description: None,
            is_wild_type: true,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let cre = create_allele(
        &store,
        CreateAlleleCommand {
            locus_id: locus.id,
            symbol: "Cre".to_owned(),
            description: None,
            is_wild_type: false,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let definition = create_genotype_definition(
        &store,
        CreateGenotypeDefinitionCommand {
            lab_id: lab.id,
            name: "Tek-Cre".to_owned(),
            description: None,
            components: vec![CreateGenotypeComponentInput {
                locus_id: locus.id,
                allele_1_id: wild_type.id,
                allele_2_id: Some(cre.id),
                mode: GenotypeComponentMode::Diploid,
                display_order: 0,
            }],
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let record = create_genotyping_record(
        &store,
        CreateGenotypingRecordCommand {
            lab_id: lab.id,
            project_id: Some(project.id),
            animal_id: animal.id,
            genotype_definition_id: definition.id,
            state: GenotypingState::Confirmed,
            assessed_at: Some(now),
            method: Some("PCR".to_owned()),
            notes: None,
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
            name: "Enrollment snapshot".to_owned(),
            description: None,
            starts_at: Some(now),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let participation = create_participation(
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

    assert_eq!(participation.genotype_snapshot.len(), 1);
    let snapshot = &participation.genotype_snapshot[0];
    assert_eq!(snapshot.genotyping_record_id, record.id);
    assert_eq!(snapshot.genotype_definition_id, definition.id);
    assert_eq!(snapshot.state, GenotypingState::Confirmed);
    assert_eq!(
        store
            .get_participation(participation.id)
            .await
            .unwrap()
            .genotype_snapshot,
        participation.genotype_snapshot
    );
}
