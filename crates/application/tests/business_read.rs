use std::sync::Arc;

use chrono::{Duration, Utc};
use muriarc_application::{
    AnimalContextRequest, BusinessReadAccess, BusinessReadService, BusinessResource,
    CreateAlleleCommand, CreateGeneLocusCommand, CreateGenotypeComponentInput,
    CreateGenotypeDefinitionCommand, CreateGenotypingRecordCommand, CreatePedigreeCommand,
    GenotypingQueryRequest, ProjectContextRequest, ReadPageRequest, ResourceSearchRequest,
    ResourceSearchResult, create_allele, create_gene_locus, create_genotype_definition,
    create_genotyping_record, create_pedigree,
};
use muriarc_core::{
    Animal, AuditContext, Cage, GenotypeComponentMode, GenotypingState, Lab, MuriArcStore,
    ParentType, Project, ProjectAnimalAssignment, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;

fn resource_request(resource: BusinessResource) -> ResourceSearchRequest {
    ResourceSearchRequest {
        resource,
        project_id: None,
        animal_id: None,
        experiment_id: None,
        experiment_event_id: None,
        breeding_line_id: None,
        colony_id: None,
        breeding_pair_id: None,
        mating_event_id: None,
        observation_id: None,
        observation_subject_id: None,
        locus_id: None,
        cohort_id: None,
        litter_id: None,
        cage_id: None,
        animal_status: None,
        project_status: None,
        experiment_status: None,
        genotyping_state: None,
        breeding_pair_status: None,
        procedure_status: None,
        observation_subject_type: None,
        template_status: None,
        job_kind: None,
        job_status: None,
        provenance_source: None,
        entity_type: None,
        entity_id: None,
        query: None,
        measurement_key: None,
        sample_type: None,
        page: ReadPageRequest::default(),
    }
}

#[tokio::test]
async fn business_reads_use_assignments_and_current_genetics_v2_facts() {
    let store = Arc::new(SqliteStore::in_memory().await.unwrap());
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Business read lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "Visible project", now).unwrap();
    let hidden_project = Project::new(lab.id, "Hidden project", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();
    store.create_project(&hidden_project, &audit).await.unwrap();

    let visible_cage = Cage::new(lab.id, "A", "A-01", now).unwrap();
    let hidden_cage = Cage::new(lab.id, "A", "A-02", now).unwrap();
    store.create_cage(&visible_cage, &audit).await.unwrap();
    store.create_cage(&hidden_cage, &audit).await.unwrap();

    let mut animal = Animal::new_mouse(lab.id, "M-001", Sex::Female, now).unwrap();
    animal.current_cage_id = Some(visible_cage.id);
    let mut same_cage_hidden = Animal::new_mouse(lab.id, "M-HIDDEN", Sex::Male, now).unwrap();
    same_cage_hidden.current_cage_id = Some(visible_cage.id);
    let mut other_cage_hidden = Animal::new_mouse(lab.id, "M-OTHER", Sex::Male, now).unwrap();
    other_cage_hidden.current_cage_id = Some(hidden_cage.id);
    store.create_animal(&animal, &audit).await.unwrap();
    store
        .create_animal(&same_cage_hidden, &audit)
        .await
        .unwrap();
    store
        .create_animal(&other_cage_hidden, &audit)
        .await
        .unwrap();
    store
        .assign_animals_to_project(
            &[ProjectAnimalAssignment::new(
                lab.id, project.id, animal.id, None, None, now,
            )],
            &audit,
        )
        .await
        .unwrap();

    let locus = create_gene_locus(
        store.as_ref(),
        CreateGeneLocusCommand {
            lab_id: lab.id,
            symbol: "Cre".to_owned(),
            description: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let allele = create_allele(
        store.as_ref(),
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
    let confirmed_definition = create_genotype_definition(
        store.as_ref(),
        CreateGenotypeDefinitionCommand {
            lab_id: lab.id,
            name: "Cre resolved".to_owned(),
            description: None,
            components: vec![CreateGenotypeComponentInput {
                locus_id: locus.id,
                allele_1_id: allele.id,
                allele_2_id: None,
                mode: GenotypeComponentMode::Hemizygous,
                display_order: 0,
            }],
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let pending_definition = create_genotype_definition(
        store.as_ref(),
        CreateGenotypeDefinitionCommand {
            lab_id: lab.id,
            name: "Cre pending".to_owned(),
            description: None,
            components: vec![CreateGenotypeComponentInput {
                locus_id: locus.id,
                allele_1_id: allele.id,
                allele_2_id: None,
                mode: GenotypeComponentMode::Hemizygous,
                display_order: 0,
            }],
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    let old_expected = create_genotyping_record(
        store.as_ref(),
        CreateGenotypingRecordCommand {
            lab_id: lab.id,
            project_id: Some(project.id),
            animal_id: animal.id,
            genotype_definition_id: confirmed_definition.id,
            state: GenotypingState::Expected,
            assessed_at: None,
            method: None,
            notes: None,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let confirmed = create_genotyping_record(
        store.as_ref(),
        CreateGenotypingRecordCommand {
            lab_id: lab.id,
            project_id: Some(project.id),
            animal_id: animal.id,
            genotype_definition_id: confirmed_definition.id,
            state: GenotypingState::Confirmed,
            assessed_at: Some(now + Duration::seconds(1)),
            method: Some("PCR".to_owned()),
            notes: None,
            now: now + Duration::seconds(1),
        },
        &audit,
    )
    .await
    .unwrap();
    let pending = create_genotyping_record(
        store.as_ref(),
        CreateGenotypingRecordCommand {
            lab_id: lab.id,
            project_id: Some(project.id),
            animal_id: animal.id,
            genotype_definition_id: pending_definition.id,
            state: GenotypingState::Expected,
            assessed_at: None,
            method: None,
            notes: None,
            now: now + Duration::seconds(2),
        },
        &audit,
    )
    .await
    .unwrap();

    let service = BusinessReadService::new(store, BusinessReadAccess::new(lab.id, [project.id]));
    let mut pending_request = resource_request(BusinessResource::GenotypingRecords);
    pending_request.project_id = Some(project.id);
    pending_request.genotyping_state = Some(GenotypingState::Expected);
    let pending_result = service.resource_search(pending_request).await.unwrap();
    let ResourceSearchResult::GenotypingRecords(records) = pending_result.data else {
        panic!("wrong resource result");
    };
    assert_eq!(records.items.len(), 1);
    assert_eq!(records.items[0].record.id, pending.id);
    assert_ne!(records.items[0].record.id, old_expected.id);
    assert_eq!(records.page.returned, 1);
    assert!(records.page.complete);
    let dedicated = service
        .genotyping_query(GenotypingQueryRequest {
            project_id: Some(project.id),
            animal_id: None,
            state: Some(GenotypingState::Expected),
            page: ReadPageRequest::default(),
        })
        .await
        .unwrap();
    assert_eq!(dedicated.data.items.len(), 1);
    assert_eq!(dedicated.data.items[0].record.id, pending.id);
    assert_eq!(dedicated.sources, pending_result.sources);

    let mut loci_request = resource_request(BusinessResource::GeneLoci);
    loci_request.project_id = Some(project.id);
    loci_request.animal_id = Some(animal.id);
    let ResourceSearchResult::GeneLoci(loci) =
        service.resource_search(loci_request).await.unwrap().data
    else {
        panic!("wrong gene loci result");
    };
    assert_eq!(
        loci.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [locus.id]
    );
    assert!(matches!(
        service
            .resource_search(resource_request(BusinessResource::GeneLoci))
            .await,
        Err(muriarc_application::BusinessReadError::Rejected(
            "lab_registry_forbidden"
        ))
    ));
    let mut hidden_animal_registry_request = resource_request(BusinessResource::GeneLoci);
    hidden_animal_registry_request.project_id = Some(project.id);
    hidden_animal_registry_request.animal_id = Some(same_cage_hidden.id);
    assert!(matches!(
        service
            .resource_search(hidden_animal_registry_request)
            .await,
        Err(muriarc_application::BusinessReadError::Rejected(
            "animal_forbidden"
        ))
    ));

    let mut alleles_request = resource_request(BusinessResource::Alleles);
    alleles_request.project_id = Some(project.id);
    alleles_request.animal_id = Some(animal.id);
    alleles_request.locus_id = Some(locus.id);
    let ResourceSearchResult::Alleles(alleles) =
        service.resource_search(alleles_request).await.unwrap().data
    else {
        panic!("wrong alleles result");
    };
    assert_eq!(
        alleles.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [allele.id]
    );

    let mut definitions_request = resource_request(BusinessResource::GenotypeDefinitions);
    definitions_request.project_id = Some(project.id);
    definitions_request.animal_id = Some(animal.id);
    let ResourceSearchResult::GenotypeDefinitions(definitions) = service
        .resource_search(definitions_request)
        .await
        .unwrap()
        .data
    else {
        panic!("wrong genotype definitions result");
    };
    assert_eq!(definitions.items.len(), 2);

    let mut history_request = resource_request(BusinessResource::GenotypingHistory);
    history_request.project_id = Some(project.id);
    history_request.animal_id = Some(animal.id);
    let ResourceSearchResult::GenotypingHistory(history) =
        service.resource_search(history_request).await.unwrap().data
    else {
        panic!("wrong genotyping history result");
    };
    assert_eq!(history.items.len(), 3);
    assert!(history.items.iter().any(|item| item.id == old_expected.id));

    let animal_context = service
        .animal_context(AnimalContextRequest {
            animal_id: animal.id,
            project_id: Some(project.id),
            page: ReadPageRequest::default(),
        })
        .await
        .unwrap();
    assert_eq!(animal_context.data.current_genotyping_records.len(), 2);
    assert_eq!(animal_context.data.genotyping_history_count, 3);
    assert_eq!(
        animal_context.data.cage.as_ref().map(|cage| cage.id),
        Some(visible_cage.id)
    );
    assert!(animal_context.sources.iter().any(|source| {
        source.entity_id == confirmed.id
            && source.entity_type == muriarc_core::EntityType::GenotypingRecord
    }));

    let project_context = service
        .project_context(ProjectContextRequest {
            project_id: project.id,
            page: ReadPageRequest::default(),
        })
        .await
        .unwrap();
    assert_eq!(project_context.data.animals.items.len(), 1);
    assert_eq!(project_context.data.animals.items[0].animal.id, animal.id);
    assert_eq!(project_context.data.cages.items.len(), 1);
    assert_eq!(project_context.data.cages.items[0].id, visible_cage.id);
    assert!(
        project_context
            .data
            .animals
            .items
            .iter()
            .all(|item| item.animal.id != same_cage_hidden.id)
    );
}

#[tokio::test]
async fn project_pedigree_reads_only_return_edges_with_both_animals_in_scope() {
    let store = Arc::new(SqliteStore::in_memory().await.unwrap());
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Pedigree scope lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "Visible project", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();

    let visible_child = Animal::new_mouse(lab.id, "VISIBLE-CHILD", Sex::Female, now).unwrap();
    let visible_parent = Animal::new_mouse(lab.id, "VISIBLE-PARENT", Sex::Male, now).unwrap();
    let hidden_father = Animal::new_mouse(lab.id, "HIDDEN-FATHER", Sex::Male, now).unwrap();
    let hidden_mother = Animal::new_mouse(lab.id, "HIDDEN-MOTHER", Sex::Female, now).unwrap();
    let hidden_child = Animal::new_mouse(lab.id, "HIDDEN-CHILD", Sex::Female, now).unwrap();
    for animal in [
        &visible_child,
        &visible_parent,
        &hidden_father,
        &hidden_mother,
        &hidden_child,
    ] {
        store.create_animal(animal, &audit).await.unwrap();
    }
    store
        .assign_animals_to_project(
            &[
                ProjectAnimalAssignment::new(lab.id, project.id, visible_child.id, None, None, now),
                ProjectAnimalAssignment::new(
                    lab.id,
                    project.id,
                    visible_parent.id,
                    None,
                    None,
                    now,
                ),
            ],
            &audit,
        )
        .await
        .unwrap();

    let visible_edge = create_pedigree(
        store.as_ref(),
        CreatePedigreeCommand {
            animal_id: visible_child.id,
            parent_id: visible_parent.id,
            parent_type: ParentType::Father,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let hidden_father_edge = create_pedigree(
        store.as_ref(),
        CreatePedigreeCommand {
            animal_id: visible_child.id,
            parent_id: hidden_father.id,
            parent_type: ParentType::Father,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let hidden_mother_edge = create_pedigree(
        store.as_ref(),
        CreatePedigreeCommand {
            animal_id: visible_child.id,
            parent_id: hidden_mother.id,
            parent_type: ParentType::Mother,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let hidden_child_edge = create_pedigree(
        store.as_ref(),
        CreatePedigreeCommand {
            animal_id: hidden_child.id,
            parent_id: visible_parent.id,
            parent_type: ParentType::Father,
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    let project_service =
        BusinessReadService::new(store.clone(), BusinessReadAccess::new(lab.id, [project.id]));
    let mut child_request = resource_request(BusinessResource::Pedigrees);
    child_request.project_id = Some(project.id);
    child_request.animal_id = Some(visible_child.id);
    let child_result = project_service
        .resource_search(child_request)
        .await
        .unwrap();
    let ResourceSearchResult::Pedigrees(child_pedigrees) = child_result.data else {
        panic!("wrong resource result");
    };
    assert_eq!(child_pedigrees.items, vec![visible_edge.clone()]);
    assert_eq!(
        child_result
            .sources
            .iter()
            .map(|source| source.entity_id)
            .collect::<Vec<_>>(),
        [visible_edge.id]
    );
    let child_json = serde_json::to_string(&child_pedigrees).unwrap();
    assert!(!child_json.contains(&hidden_father.id.to_string()));
    assert!(!child_json.contains(&hidden_mother.id.to_string()));

    let mut parent_request = resource_request(BusinessResource::Pedigrees);
    parent_request.project_id = Some(project.id);
    parent_request.animal_id = Some(visible_parent.id);
    let parent_result = project_service
        .resource_search(parent_request)
        .await
        .unwrap();
    let ResourceSearchResult::Pedigrees(parent_pedigrees) = parent_result.data else {
        panic!("wrong resource result");
    };
    assert_eq!(parent_pedigrees.items, vec![visible_edge.clone()]);
    assert!(
        !serde_json::to_string(&parent_pedigrees)
            .unwrap()
            .contains(&hidden_child.id.to_string())
    );

    let lab_service = BusinessReadService::new(
        store,
        BusinessReadAccess::new(lab.id, []).with_lab_registry_read(true),
    );
    let mut lab_request = resource_request(BusinessResource::Pedigrees);
    lab_request.animal_id = Some(visible_child.id);
    let ResourceSearchResult::Pedigrees(lab_pedigrees) =
        lab_service.resource_search(lab_request).await.unwrap().data
    else {
        panic!("wrong resource result");
    };
    assert_eq!(lab_pedigrees.items.len(), 3);
    assert!(
        lab_pedigrees
            .items
            .iter()
            .any(|pedigree| pedigree.id == hidden_father_edge.id)
    );
    assert!(
        lab_pedigrees
            .items
            .iter()
            .any(|pedigree| pedigree.id == hidden_mother_edge.id)
    );
    assert!(
        !lab_pedigrees
            .items
            .iter()
            .any(|pedigree| pedigree.id == hidden_child_edge.id)
    );
}
