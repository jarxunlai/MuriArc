use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::{
    CreateAnimalCommand, CreateAnimalIdentifierScope, InitialGenotypingRecordInput, create_animal,
};
use muriarc_core::{
    Allele, AnimalEventKind, AnimalFilter, AuditAction, AuditContext, AuditFilter, Cage,
    EntityType, GeneLocus, GenotypeComponent, GenotypeComponentMode, GenotypeDefinition,
    GenotypingState, IdentifierScope, Lab, MuriArcStore, Project, RecordMeta, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T08:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn create_normalizes_and_persists_the_complete_intent() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Application test lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "Application test project", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();
    let cage = Cage::new(lab.id, "SPF", "A01", now).unwrap();
    store.create_cage(&cage, &audit).await.unwrap();
    let locus = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        symbol: "GeneA".to_owned(),
        description: None,
        meta: RecordMeta::new(now),
    };
    store.create_gene_locus(&locus, &audit).await.unwrap();
    let allele = Allele {
        id: uuid::Uuid::new_v4(),
        locus_id: locus.id,
        symbol: "+".to_owned(),
        description: None,
        is_wild_type: true,
        meta: RecordMeta::new(now),
    };
    store.create_allele(&allele, &audit).await.unwrap();
    let mut definition = GenotypeDefinition::new(lab.id, "GeneA +/+", now).unwrap();
    definition
        .replace_components(vec![
            GenotypeComponent::new(
                definition.id,
                locus.id,
                allele.id,
                Some(allele.id),
                GenotypeComponentMode::Diploid,
                0,
                now,
            )
            .unwrap(),
        ])
        .unwrap();
    store
        .create_genotype_definition(&definition, &audit)
        .await
        .unwrap();

    let animal = create_animal(
        &store,
        CreateAnimalCommand {
            lab_id: lab.id,
            identifier_scope: CreateAnimalIdentifierScope::Project(project.id),
            display_id: "  M-001  ".to_owned(),
            sex: Sex::Female,
            strain: Some("  C57BL/6J  ".to_owned()),
            birth_date: Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
            legacy_id: Some("  legacy-001  ".to_owned()),
            initial_cage_id: Some(cage.id),
            initial_genotyping_records: vec![InitialGenotypingRecordInput {
                genotype_definition_id: definition.id,
                state: GenotypingState::Expected,
                assessed_at: None,
                method: Some("  breeding expectation  ".to_owned()),
                notes: None,
            }],
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    assert_eq!(animal.display_id, "M-001");
    assert_eq!(animal.strain.as_deref(), Some("C57BL/6J"));
    assert_eq!(animal.legacy_id.as_deref(), Some("legacy-001"));
    assert_eq!(
        animal.identifier_scope,
        IdentifierScope::Project {
            project_id: project.id
        }
    );
    assert_eq!(animal.current_cage_id, Some(cage.id));
    assert_eq!(store.get_animal(animal.id).await.unwrap(), animal);

    let events = store.list_animal_events(animal.id).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, AnimalEventKind::Registered))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, AnimalEventKind::Born { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, AnimalEventKind::Transferred { .. }))
    );
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            AnimalEventKind::GenotypingRecorded {
                genotype_definition_id,
                state: GenotypingState::Expected,
                ..
            } if genotype_definition_id == definition.id
        )
    }));
    let records = store
        .list_current_genotyping_records(animal.id)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].genotype_definition_id, definition.id);
    assert_eq!(records[0].project_id, Some(project.id));
    assert_eq!(records[0].method.as_deref(), Some("breeding expectation"));

    let audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: Some(animal.id),
        })
        .await
        .unwrap();
    assert!(audits.iter().any(|entry| {
        entry.entity_type == EntityType::Animal
            && entry.action == AuditAction::Create
            && entry.source == WriteSource::Api
    }));
}

#[tokio::test]
async fn invalid_initial_genotyping_record_rolls_back_the_animal() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Atomic animal registration lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();

    let result = create_animal(
        &store,
        CreateAnimalCommand {
            lab_id: lab.id,
            identifier_scope: CreateAnimalIdentifierScope::Lab,
            display_id: "ROLLBACK-001".to_owned(),
            sex: Sex::Unknown,
            strain: None,
            birth_date: None,
            legacy_id: None,
            initial_cage_id: None,
            initial_genotyping_records: vec![InitialGenotypingRecordInput {
                genotype_definition_id: uuid::Uuid::new_v4(),
                state: GenotypingState::Expected,
                assessed_at: None,
                method: None,
                notes: None,
            }],
            now,
        },
        &audit,
    )
    .await;
    assert!(result.is_err());
    assert!(
        store
            .list_animals(&AnimalFilter {
                lab_id: lab.id,
                ..AnimalFilter::default()
            })
            .await
            .unwrap()
            .is_empty()
    );
}
