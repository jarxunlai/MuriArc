use chrono::{DateTime, Duration, Utc};
use muriarc_application::{
    CreateAlleleCommand, CreateAnimalDraftInput, CreateAnimalIdentifierScope,
    CreateBreedingLineCommand, CreateBreedingPairCommand, CreateColonyCommand,
    CreateGeneLocusCommand, CreateGenotypeComponentInput, CreateGenotypeDefinitionCommand,
    CreateLitterCommand, CreateMatingEventCommand, RegisterAnimalDraftCommand, breeding_prediction,
    create_allele, create_breeding_line, create_breeding_pair, create_colony, create_gene_locus,
    create_genotype_definition, create_litter, create_mating_event, register_animal_draft,
};
use muriarc_core::{
    Animal, AnimalDraftStatus, AuditContext, GenotypeComponentMode, Lab, MuriArcStore, ParentType,
    Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn breeding_workflow_registers_offspring_and_both_parents_atomically() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Breeding application test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();

    let locus = create_gene_locus(
        &store,
        CreateGeneLocusCommand {
            lab_id: lab.id,
            symbol: "Rosa26".to_owned(),
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
    let transgene = create_allele(
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
            name: "Rosa26 Cre".to_owned(),
            description: None,
            components: vec![CreateGenotypeComponentInput {
                locus_id: locus.id,
                allele_1_id: wild_type.id,
                allele_2_id: Some(transgene.id),
                mode: GenotypeComponentMode::Diploid,
                display_order: 0,
            }],
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    let line = create_breeding_line(
        &store,
        CreateBreedingLineCommand {
            lab_id: lab.id,
            name: "Cre line".to_owned(),
            description: Some("  maintained line  ".to_owned()),
            genotype_definition_ids: vec![definition.id],
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let colony = create_colony(
        &store,
        CreateColonyCommand {
            lab_id: lab.id,
            breeding_line_id: line.id,
            name: "Main colony".to_owned(),
            description: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    let male = Animal::new_mouse(lab.id, "SIRE-1", Sex::Male, now).unwrap();
    let female = Animal::new_mouse(lab.id, "DAM-1", Sex::Female, now).unwrap();
    let second_female = Animal::new_mouse(lab.id, "DAM-2", Sex::Female, now).unwrap();
    store.create_animal(&male, &audit).await.unwrap();
    store.create_animal(&female, &audit).await.unwrap();
    store.create_animal(&second_female, &audit).await.unwrap();

    let pair = create_breeding_pair(
        &store,
        CreateBreedingPairCommand {
            lab_id: lab.id,
            colony_id: colony.id,
            name: "Pair A".to_owned(),
            male_animal_id: male.id,
            female_animal_ids: vec![female.id, second_female.id],
            started_at: now,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(pair.members.len(), 3);

    let mating = create_mating_event(
        &store,
        CreateMatingEventCommand {
            lab_id: lab.id,
            breeding_pair_id: pair.id,
            male_animal_id: male.id,
            female_animal_id: female.id,
            occurred_at: now + Duration::days(1),
            notes: Some(" observed plug ".to_owned()),
            now: now + Duration::days(1),
        },
        &audit,
    )
    .await
    .unwrap();

    let born_on = (now + Duration::days(20)).date_naive();
    let created = create_litter(
        &store,
        CreateLitterCommand {
            lab_id: lab.id,
            mating_event_id: mating.id,
            born_on,
            size_total: 2,
            drafts: vec![
                CreateAnimalDraftInput {
                    temporary_label: "P1".to_owned(),
                    sex: Sex::Female,
                },
                CreateAnimalDraftInput {
                    temporary_label: "P2".to_owned(),
                    sex: Sex::Male,
                },
            ],
            notes: None,
            now: now + Duration::days(20),
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(created.drafts.len(), 2);

    let registered = register_animal_draft(
        &store,
        RegisterAnimalDraftCommand {
            lab_id: lab.id,
            draft_id: created.drafts[0].id,
            expected_revision: 1,
            identifier_scope: CreateAnimalIdentifierScope::Lab,
            display_id: "OFFSPRING-1".to_owned(),
            strain: Some(" C57BL/6J ".to_owned()),
            initial_cage_id: None,
            now: now + Duration::days(21),
        },
        &audit,
    )
    .await
    .unwrap();

    assert_eq!(registered.draft.status, AnimalDraftStatus::Registered);
    assert_eq!(
        registered.draft.registered_animal_id,
        Some(registered.animal.id)
    );
    assert_eq!(registered.animal.birth_date, Some(born_on));
    assert_eq!(registered.animal.strain.as_deref(), Some("C57BL/6J"));

    let pedigree = store.list_pedigrees(registered.animal.id).await.unwrap();
    assert_eq!(pedigree.len(), 2);
    assert!(
        pedigree
            .iter()
            .any(|edge| { edge.parent_id == male.id && edge.parent_type == ParentType::Father })
    );
    assert!(
        pedigree
            .iter()
            .any(|edge| { edge.parent_id == female.id && edge.parent_type == ParentType::Mother })
    );

    let events = store
        .list_animal_events(registered.animal.id)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| { matches!(event.kind, muriarc_core::AnimalEventKind::Registered) })
    );
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            muriarc_core::AnimalEventKind::Born { birth_date } if birth_date == born_on
        )
    }));

    let prediction = breeding_prediction(&store, definition.id, definition.id)
        .await
        .unwrap();
    assert_eq!(prediction.len(), 1);
    let total: f64 = prediction[0]
        .outcomes
        .iter()
        .map(|outcome| outcome.probability)
        .sum();
    assert!((total - 1.0).abs() < f64::EPSILON);
}
