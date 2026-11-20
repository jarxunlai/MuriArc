use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use muriarc_application::{
    AnimalStatistics, AnimalStatisticsDimension, AnimalStatisticsRequest, BusinessReadAccess,
    BusinessReadError, BusinessReadService, CreateAlleleCommand, CreateGeneLocusCommand,
    CreateGenotypeComponentInput, CreateGenotypeDefinitionCommand, RecordScope, create_allele,
    create_gene_locus, create_genotype_definition,
};
use muriarc_core::{
    Animal, AnimalStatus, AuditContext, Cage, EntityType, GenotypeComponentMode, GenotypingRecord,
    GenotypingState, Lab, MuriArcStore, Project, ProjectAnimalAssignment, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

const COMBO: &str = "Prox1 Cre+/Cre-; Ai9 tdT+/tdT-";
const NEGATIVE_COMBO: &str = "Prox1 Cre-/Cre-; Ai9 tdT+/tdT-";
const PROX1_HET: &str = "Prox1 Cre+/Cre-";
const AI9_HOM: &str = "Ai9 tdT+/tdT+";

struct Fixture {
    store: Arc<SqliteStore>,
    lab_id: Uuid,
    project_id: Uuid,
    other_project_id: Uuid,
    definitions: BTreeMap<&'static str, Uuid>,
}

/// 36 project animals in three cages: every fourth animal carries the combined
/// Prox1/Ai9 definition, one quarter the Cre-negative combination, one quarter
/// two separate single-locus definitions, and one quarter has no genotype.
/// Four genotyped animals outside the project must never be counted.
async fn fixture() -> Fixture {
    let store = Arc::new(SqliteStore::in_memory().await.unwrap());
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Statistics lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "Lymphatic mice", now).unwrap();
    let other_project = Project::new(lab.id, "Other", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();
    store.create_project(&other_project, &audit).await.unwrap();

    let mut alleles = BTreeMap::new();
    let mut loci = BTreeMap::new();
    for (symbol, allele_symbols) in [("Prox1", ["Cre+", "Cre-"]), ("Ai9", ["tdT+", "tdT-"])] {
        let locus = create_gene_locus(
            store.as_ref(),
            CreateGeneLocusCommand {
                lab_id: lab.id,
                symbol: symbol.to_owned(),
                description: None,
                now,
            },
            &audit,
        )
        .await
        .unwrap();
        loci.insert(symbol, locus.id);
        for (index, allele_symbol) in allele_symbols.into_iter().enumerate() {
            let allele = create_allele(
                store.as_ref(),
                CreateAlleleCommand {
                    locus_id: locus.id,
                    symbol: allele_symbol.to_owned(),
                    description: None,
                    is_wild_type: index == 1,
                    now,
                },
                &audit,
            )
            .await
            .unwrap();
            alleles.insert(allele_symbol, allele.id);
        }
    }
    let mut definitions = BTreeMap::new();
    for (name, components) in [
        (
            COMBO,
            vec![("Prox1", "Cre+", "Cre-"), ("Ai9", "tdT+", "tdT-")],
        ),
        (
            NEGATIVE_COMBO,
            vec![("Prox1", "Cre-", "Cre-"), ("Ai9", "tdT+", "tdT-")],
        ),
        (PROX1_HET, vec![("Prox1", "Cre+", "Cre-")]),
        (AI9_HOM, vec![("Ai9", "tdT+", "tdT+")]),
    ] {
        let definition = create_genotype_definition(
            store.as_ref(),
            CreateGenotypeDefinitionCommand {
                lab_id: lab.id,
                name: name.to_owned(),
                description: None,
                components: components
                    .into_iter()
                    .enumerate()
                    .map(
                        |(order, (locus, first, second))| CreateGenotypeComponentInput {
                            locus_id: loci[locus],
                            allele_1_id: alleles[first],
                            allele_2_id: Some(alleles[second]),
                            mode: GenotypeComponentMode::Diploid,
                            display_order: order as i32,
                        },
                    )
                    .collect(),
                now,
            },
            &audit,
        )
        .await
        .unwrap();
        definitions.insert(name, definition.id);
    }

    let cages = ["C-1", "C-2", "C-3"]
        .into_iter()
        .map(|display_id| {
            let mut cage = Cage::new(lab.id, "A", display_id, now).unwrap();
            cage.capacity = 20;
            cage
        })
        .collect::<Vec<_>>();
    for cage in &cages {
        store.create_cage(cage, &audit).await.unwrap();
    }

    let record = |animal_id: Uuid, name: &str, state: GenotypingState| {
        // Confirmed facts require an assessment time.
        let assessed_at = (state == GenotypingState::Confirmed).then_some(now);
        GenotypingRecord::new(
            lab.id,
            animal_id,
            definitions[name],
            state,
            assessed_at,
            now,
        )
        .unwrap()
    };
    let mut assignments = Vec::new();
    for index in 0..40_usize {
        let sex = if index % 2 == 0 {
            Sex::Female
        } else {
            Sex::Male
        };
        let mut animal = Animal::new_mouse(lab.id, format!("M{index:03}"), sex, now).unwrap();
        animal.current_cage_id = Some(cages[index % 3].id);
        if (30..36).contains(&index) {
            animal.current_status = AnimalStatus::Deceased;
        }
        let records = match index % 4 {
            0 => vec![record(animal.id, COMBO, GenotypingState::Expected)],
            1 => vec![record(animal.id, NEGATIVE_COMBO, GenotypingState::Expected)],
            2 => vec![
                record(animal.id, PROX1_HET, GenotypingState::Confirmed),
                record(animal.id, AI9_HOM, GenotypingState::Expected),
            ],
            _ => Vec::new(),
        };
        let outside_project = index >= 36;
        let records = if outside_project {
            vec![record(animal.id, COMBO, GenotypingState::Expected)]
        } else {
            records
        };
        store
            .create_animal_with_genotyping_records(&animal, &records, &audit)
            .await
            .unwrap();
        if !outside_project {
            assignments.push(ProjectAnimalAssignment::new(
                lab.id, project.id, animal.id, None, None, now,
            ));
        }
    }
    store
        .assign_animals_to_project(&assignments, &audit)
        .await
        .unwrap();

    Fixture {
        store,
        lab_id: lab.id,
        project_id: project.id,
        other_project_id: other_project.id,
        definitions,
    }
}

fn project_service(fixture: &Fixture) -> BusinessReadService {
    BusinessReadService::new(
        fixture.store.clone(),
        BusinessReadAccess::new(fixture.lab_id, [fixture.project_id]),
    )
}

fn project_request(fixture: &Fixture) -> AnimalStatisticsRequest {
    AnimalStatisticsRequest {
        project_id: Some(fixture.project_id),
        ..AnimalStatisticsRequest::default()
    }
}

async fn statistics(fixture: &Fixture, request: AnimalStatisticsRequest) -> AnimalStatistics {
    project_service(fixture)
        .animal_statistics(request)
        .await
        .unwrap()
        .data
}

fn counts(result: &AnimalStatistics, dimension: &str) -> BTreeMap<String, usize> {
    result
        .groups
        .iter()
        .map(|group| (group.key[dimension].clone(), group.animals))
        .collect()
}

#[tokio::test]
async fn project_totals_and_genotype_groups_count_distinct_animals_without_listing_them() {
    let fixture = fixture().await;
    let result = project_service(&fixture)
        .animal_statistics(AnimalStatisticsRequest {
            group_by: vec![AnimalStatisticsDimension::GenotypeDefinition],
            ..project_request(&fixture)
        })
        .await
        .unwrap();

    let data = &result.data;
    assert_eq!(
        data.record_scope,
        RecordScope::Project {
            project_id: fixture.project_id
        }
    );
    assert_eq!(data.animals_in_scope, 36);
    assert_eq!(data.matching_animals, 36);
    assert_eq!(
        counts(data, "genotype_definition"),
        BTreeMap::from([
            ("(none)".to_owned(), 9),
            (AI9_HOM.to_owned(), 9),
            (COMBO.to_owned(), 9),
            (NEGATIVE_COMBO.to_owned(), 9),
            (PROX1_HET.to_owned(), 9),
        ])
    );
    assert_eq!(data.groups_total, 5);
    assert!(!data.groups_truncated);
    assert!(data.multi_valued_groups);
    let combo_group = data
        .groups
        .iter()
        .find(|group| group.key["genotype_definition"] == COMBO)
        .unwrap();
    assert_eq!(
        combo_group.ids.get("genotype_definition"),
        Some(&fixture.definitions[COMBO])
    );
    assert!(result.sources.iter().any(|source| {
        source.entity_type == EntityType::Project && source.entity_id == fixture.project_id
    }));
    assert!(result.sources.iter().any(|source| {
        source.entity_type == EntityType::GenotypeDefinition
            && source.entity_id == fixture.definitions[COMBO]
    }));
    assert!(
        !result
            .sources
            .iter()
            .any(|source| source.entity_type == EntityType::Animal),
        "aggregate results must not cite every animal"
    );
    let serialized = serde_json::to_string(data).unwrap();
    assert!(!serialized.contains("M000"), "no animal rows are returned");
}

#[tokio::test]
async fn genotype_definition_filters_are_normalized_and_combined_with_and() {
    let fixture = fixture().await;

    let combo = statistics(
        &fixture,
        AnimalStatisticsRequest {
            genotype_definitions: vec!["  prox1 cre+/cre-;ai9 TDT+/tdt-  ".to_owned()],
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(combo.animals_in_scope, 36);
    assert_eq!(combo.matching_animals, 9);
    assert!(combo.unmatched_genotype_definitions.is_empty());

    let both_single_loci = statistics(
        &fixture,
        AnimalStatisticsRequest {
            genotype_definitions: vec![PROX1_HET.to_owned(), AI9_HOM.to_owned()],
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(both_single_loci.matching_animals, 9);

    let expected_prox1 = statistics(
        &fixture,
        AnimalStatisticsRequest {
            genotype_definitions: vec![PROX1_HET.to_owned()],
            genotyping_state: Some(GenotypingState::Expected),
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(
        expected_prox1.matching_animals, 0,
        "the Prox1 record is confirmed, not expected"
    );
}

#[tokio::test]
async fn unknown_genotype_names_report_available_definitions_instead_of_a_silent_zero() {
    let fixture = fixture().await;
    let result = statistics(
        &fixture,
        AnimalStatisticsRequest {
            genotype_definitions: vec![PROX1_HET.to_owned(), "Ai9 tdT+/tdT-".to_owned()],
            ..project_request(&fixture)
        },
    )
    .await;

    assert_eq!(result.matching_animals, 0);
    assert_eq!(
        result.unmatched_genotype_definitions,
        vec!["Ai9 tdT+/tdT-".to_owned()]
    );
    assert!(
        result
            .available_genotype_definitions
            .contains(&COMBO.to_owned())
    );
    assert!(result.note.as_deref().unwrap().contains("Ai9 tdT+/tdT-"));
}

#[tokio::test]
async fn animal_dimensions_and_filters_cover_status_sex_cage_and_state() {
    let fixture = fixture().await;

    let by_status = statistics(
        &fixture,
        AnimalStatisticsRequest {
            group_by: vec![AnimalStatisticsDimension::Status],
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(
        counts(&by_status, "status"),
        BTreeMap::from([("alive".to_owned(), 30), ("deceased".to_owned(), 6)])
    );
    assert!(!by_status.multi_valued_groups);

    let alive_females = statistics(
        &fixture,
        AnimalStatisticsRequest {
            animal_status: Some(AnimalStatus::Alive),
            sex: Some(Sex::Female),
            group_by: vec![
                AnimalStatisticsDimension::Sex,
                AnimalStatisticsDimension::Cage,
            ],
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(alive_females.animals_in_scope, 36);
    assert_eq!(alive_females.matching_animals, 15);
    assert_eq!(
        alive_females
            .groups
            .iter()
            .map(|group| group.animals)
            .sum::<usize>(),
        15
    );
    assert!(
        alive_females
            .groups
            .iter()
            .all(|group| group.key["sex"] == "female")
    );

    let confirmed = statistics(
        &fixture,
        AnimalStatisticsRequest {
            genotyping_state: Some(GenotypingState::Confirmed),
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(confirmed.matching_animals, 9);

    let limited = statistics(
        &fixture,
        AnimalStatisticsRequest {
            group_by: vec![AnimalStatisticsDimension::Cage],
            max_groups: Some(2),
            ..project_request(&fixture)
        },
    )
    .await;
    assert_eq!(limited.groups.len(), 2);
    assert_eq!(limited.groups_total, 3);
    assert!(limited.groups_truncated);
}

#[tokio::test]
async fn statistics_respect_project_scope_and_bounded_arguments() {
    let fixture = fixture().await;
    let service = project_service(&fixture);
    let rejected = |result: Result<_, BusinessReadError>| match result {
        Err(BusinessReadError::Rejected(code)) => code,
        other => panic!("expected rejection, got {other:?}"),
    };

    assert_eq!(
        rejected(
            service
                .animal_statistics(AnimalStatisticsRequest::default())
                .await
        ),
        "project_required"
    );
    assert_eq!(
        rejected(
            service
                .animal_statistics(AnimalStatisticsRequest {
                    project_id: Some(fixture.other_project_id),
                    ..AnimalStatisticsRequest::default()
                })
                .await
        ),
        "project_forbidden"
    );
    assert_eq!(
        rejected(
            service
                .animal_statistics(AnimalStatisticsRequest {
                    group_by: vec![
                        AnimalStatisticsDimension::Sex,
                        AnimalStatisticsDimension::Status,
                        AnimalStatisticsDimension::Cage,
                    ],
                    ..project_request(&fixture)
                })
                .await
        ),
        "statistics_group_by_invalid"
    );
    assert_eq!(
        rejected(
            service
                .animal_statistics(AnimalStatisticsRequest {
                    max_groups: Some(0),
                    ..project_request(&fixture)
                })
                .await
        ),
        "statistics_max_groups_out_of_range"
    );
    assert_eq!(
        rejected(
            service
                .animal_statistics(AnimalStatisticsRequest {
                    genotype_definitions: vec![" ".to_owned()],
                    ..project_request(&fixture)
                })
                .await
        ),
        "statistics_genotype_definitions_invalid"
    );

    let lab_service = BusinessReadService::new(
        fixture.store.clone(),
        BusinessReadAccess::new(fixture.lab_id, [fixture.project_id]).with_lab_registry_read(true),
    );
    let lab = lab_service
        .animal_statistics(AnimalStatisticsRequest::default())
        .await
        .unwrap()
        .data;
    assert_eq!(lab.record_scope, RecordScope::Lab);
    assert_eq!(lab.animals_in_scope, 40);
}
