use chrono::Utc;
use muriarc_core::*;
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

#[tokio::test]
async fn sqlite_persists_research_entities() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("Research Contract", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "Project", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();
    let parent = Animal::new_mouse(lab.id, "P001", Sex::Male, now).unwrap();
    let child = Animal::new_mouse(lab.id, "C001", Sex::Female, now).unwrap();
    store.create_animal(&parent, &audit).await.unwrap();
    store.create_animal(&child, &audit).await.unwrap();

    let locus = GeneLocus {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        symbol: "GeneA".to_owned(),
        description: None,
        meta: RecordMeta::new(now),
    };
    store.create_gene_locus(&locus, &audit).await.unwrap();
    assert_eq!(store.get_gene_locus(locus.id).await.unwrap(), locus);
    let wild_type = Allele {
        id: Uuid::new_v4(),
        locus_id: locus.id,
        symbol: "+".to_owned(),
        description: None,
        is_wild_type: true,
        meta: RecordMeta::new(now),
    };
    let flox = Allele {
        id: Uuid::new_v4(),
        locus_id: locus.id,
        symbol: "flox".to_owned(),
        description: None,
        is_wild_type: false,
        meta: RecordMeta::new(now),
    };
    store.create_allele(&wild_type, &audit).await.unwrap();
    store.create_allele(&flox, &audit).await.unwrap();
    assert_eq!(store.list_alleles(locus.id).await.unwrap().len(), 2);
    let genotype = Genotype {
        id: Uuid::new_v4(),
        animal_id: child.id,
        locus_id: locus.id,
        allele_1_id: Some(wild_type.id),
        allele_2_id: Some(flox.id),
        assessed_at: Some(now),
        meta: RecordMeta::new(now),
    };
    store
        .create_genotype(&genotype, Some(project.id), &audit)
        .await
        .unwrap();
    assert_eq!(
        store.list_genotypes(child.id).await.unwrap(),
        vec![genotype.clone()]
    );
    let mut definition = GenotypeDefinition::new(lab.id, "GeneA +/flox", now).unwrap();
    definition
        .replace_components(vec![
            GenotypeComponent::new(
                definition.id,
                locus.id,
                wild_type.id,
                Some(flox.id),
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
    let mut genotyping_record = GenotypingRecord::new(
        lab.id,
        child.id,
        definition.id,
        GenotypingState::Confirmed,
        Some(now),
        now,
    )
    .unwrap();
    genotyping_record.project_id = Some(project.id);
    store
        .create_genotyping_record(&genotyping_record, &audit)
        .await
        .unwrap();
    assert!(
        store
            .list_animal_events(child.id)
            .await
            .unwrap()
            .iter()
            .any(|event| {
                event.project_id == Some(project.id)
                    && matches!(
                        &event.kind,
                        AnimalEventKind::Genotyped { genotype_ids }
                            if genotype_ids == &vec![genotype.id]
                    )
            })
    );
    let genotype_provenance = store
        .list_provenance(&ProvenanceFilter {
            lab_id: lab.id,
            project_id: Some(project.id),
            entity_type: Some(EntityType::Genotype),
            entity_id: Some(genotype.id),
            source: None,
        })
        .await
        .unwrap();
    assert_eq!(genotype_provenance.len(), 1);
    let pedigree = Pedigree {
        id: Uuid::new_v4(),
        animal_id: child.id,
        parent_id: parent.id,
        parent_type: ParentType::Father,
        meta: RecordMeta::new(now),
    };
    store.create_pedigree(&pedigree, &audit).await.unwrap();
    assert_eq!(
        store.list_pedigrees(child.id).await.unwrap(),
        vec![pedigree]
    );

    let mut template =
        ExperimentTemplateVersion::draft(lab.id, "weight", 1, "Weight", now).unwrap();
    template
        .replace_fields(
            vec![TemplateField {
                key: "body_weight".to_owned(),
                label: "Body weight".to_owned(),
                value_type: FieldValueType::Number,
                unit: Some("g".to_owned()),
                required: true,
                categories: Vec::new(),
                minimum: Some(0.0),
                maximum: None,
                display_order: 0,
                ai_writable: true,
            }],
            now,
        )
        .unwrap();
    store
        .create_template_version(&template, &audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_template_versions(lab.id, Some("weight"))
            .await
            .unwrap()
            .len(),
        1
    );

    let raw_draft_experiment =
        Experiment::new(lab.id, project.id, "Raw draft-template experiment", now).unwrap();
    let raw_insert = sqlx::query(
        "INSERT INTO experiments (id, lab_id, project_id, template_version_id, name, description, status, starts_at, ends_at, created_at, updated_at, deleted_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(raw_draft_experiment.id.to_string())
    .bind(raw_draft_experiment.lab_id.to_string())
    .bind(raw_draft_experiment.project_id.to_string())
    .bind(template.id.to_string())
    .bind(&raw_draft_experiment.name)
    .bind(&raw_draft_experiment.description)
    .bind("draft")
    .bind(raw_draft_experiment.starts_at)
    .bind(raw_draft_experiment.ends_at)
    .bind(raw_draft_experiment.meta.created_at)
    .bind(raw_draft_experiment.meta.updated_at)
    .bind(raw_draft_experiment.meta.deleted_at)
    .bind(raw_draft_experiment.meta.revision)
    .execute(store.pool())
    .await;
    assert!(
        raw_insert.is_err(),
        "the SQLite migration must reject unpublished template references even when Store validation is bypassed"
    );
    assert!(matches!(
        store.get_experiment(raw_draft_experiment.id).await,
        Err(StoreError::NotFound { .. })
    ));

    let experiment = Experiment::new(lab.id, project.id, "Weight study", now).unwrap();
    store.create_experiment(&experiment, &audit).await.unwrap();
    let cohort = Cohort {
        id: Uuid::new_v4(),
        experiment_id: experiment.id,
        name: "Control".to_owned(),
        description: None,
        meta: RecordMeta::new(now),
    };
    store.create_cohort(&cohort, &audit).await.unwrap();
    assert_eq!(
        store.list_cohorts(experiment.id).await.unwrap(),
        vec![cohort]
    );
    let participation = Participation::enroll(experiment.id, child.id, now);
    store
        .create_participation(&participation, &audit)
        .await
        .unwrap();
    let procedure = Procedure {
        id: Uuid::new_v4(),
        experiment_id: experiment.id,
        animal_id: Some(child.id),
        name: "Weigh".to_owned(),
        scheduled_at: Some(now),
        performed_at: None,
        status: ProcedureStatus::Planned,
        details: serde_json::json!({"day": 0}),
        meta: RecordMeta::new(now),
    };
    store.create_procedure(&procedure, &audit).await.unwrap();
    assert_eq!(
        store
            .list_procedures(experiment.id, Some(child.id))
            .await
            .unwrap(),
        vec![procedure]
    );
    let mut weight = Measurement::draft(
        lab.id,
        project.id,
        child.id,
        "body_weight",
        "Body weight",
        MeasurementValue::Number(22.5),
        now,
        now,
    )
    .unwrap();
    weight.experiment_id = Some(experiment.id);
    weight.unit = Some("g".to_owned());
    store.create_measurement(&weight, &audit).await.unwrap();

    let overviews = store
        .list_animal_overviews(
            &AnimalFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                ..AnimalFilter::default()
            },
            0,
            10,
        )
        .await
        .unwrap();
    assert_eq!(overviews.len(), 1);
    assert_eq!(overviews[0].animal.id, child.id);
    assert_eq!(
        overviews[0].genotype_labels,
        vec!["GeneA +/flox [confirmed]"]
    );
    assert_eq!(overviews[0].projects[0].id, project.id);
    assert_eq!(overviews[0].latest_weight.as_ref().unwrap().value, 22.5);
    assert!(matches!(
        store
            .list_animal_overviews(
                &AnimalFilter {
                    lab_id: lab.id,
                    ..AnimalFilter::default()
                },
                0,
                0,
            )
            .await,
        Err(StoreError::Validation(_))
    ));

    assert_eq!(
        store.list_related_pedigrees(parent.id).await.unwrap().len(),
        1
    );
    assert_eq!(
        store.list_related_pedigrees(child.id).await.unwrap().len(),
        1
    );
    let visible_relatives = store
        .list_animals_by_ids(lab.id, Some(project.id), &[parent.id, child.id])
        .await
        .unwrap();
    assert_eq!(
        visible_relatives
            .into_iter()
            .map(|animal| animal.id)
            .collect::<Vec<_>>(),
        vec![child.id]
    );

    let attachment = Attachment {
        id: Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        entity_type: "animal".to_owned(),
        entity_id: child.id,
        file_name: "photo.png".to_owned(),
        media_type: Some("image/png".to_owned()),
        relative_path: "attachments/photo.png".to_owned(),
        size_bytes: 128,
        sha256: "a".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    store.create_attachment(&attachment, &audit).await.unwrap();
    assert_eq!(
        store
            .list_attachments(lab.id, "animal", child.id)
            .await
            .unwrap(),
        vec![attachment]
    );
}
