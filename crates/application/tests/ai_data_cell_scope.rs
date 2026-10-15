use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateExperimentCommand, CreateParticipationCommand, CreateProjectCommand, CreateSampleCommand,
    ai_data_cell_in_experiment_scope, create_experiment, create_participation, create_project,
    create_sample,
};
use muriarc_core::{
    AiObservationDataCell, Animal, AuditContext, Experiment, Lab, MuriArcStore,
    ObservationSubjectType, Project, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T14:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

async fn project(store: &SqliteStore, lab: &Lab, name: &str, audit: &AuditContext) -> Project {
    create_project(
        store,
        CreateProjectCommand {
            lab_id: lab.id,
            name: name.to_owned(),
            description: None,
            now: fixed_now(),
        },
        audit,
    )
    .await
    .unwrap()
}

async fn experiment(
    store: &SqliteStore,
    lab: &Lab,
    project: &Project,
    audit: &AuditContext,
) -> Experiment {
    create_experiment(
        store,
        CreateExperimentCommand {
            lab_id: lab.id,
            project_id: project.id,
            template_version_id: None,
            name: format!("{} experiment", project.name),
            description: None,
            starts_at: Some(fixed_now()),
            now: fixed_now(),
        },
        audit,
    )
    .await
    .unwrap()
}

fn cell(subject_type: ObservationSubjectType, subject_id: Uuid) -> AiObservationDataCell {
    AiObservationDataCell {
        definition_id: Uuid::new_v4(),
        subject_type,
        subject_id,
    }
}

#[tokio::test]
async fn data_cells_must_target_subjects_of_the_selected_experiment() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("AI data cell scope", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let selected_project = project(&store, &lab, "Selected", &audit).await;
    let other_project = project(&store, &lab, "Other", &audit).await;
    let selected = experiment(&store, &lab, &selected_project, &audit).await;
    let other = experiment(&store, &lab, &other_project, &audit).await;

    let enrolled = Animal::new_mouse(lab.id, "CELL-001", Sex::Female, now).unwrap();
    let bystander = Animal::new_mouse(lab.id, "CELL-002", Sex::Male, now).unwrap();
    store.create_animal(&enrolled, &audit).await.unwrap();
    store.create_animal(&bystander, &audit).await.unwrap();
    create_participation(
        &store,
        CreateParticipationCommand {
            experiment_id: selected.id,
            animal_id: enrolled.id,
            cohort_id: None,
            enrolled_at: now,
        },
        &audit,
    )
    .await
    .unwrap();
    let sample_for = |experiment_id: Option<Uuid>| CreateSampleCommand {
        lab_id: lab.id,
        project_id: selected_project.id,
        experiment_id,
        animal_id: enrolled.id,
        sample_type: "plasma".to_owned(),
        quantity: None,
        unit: None,
        location: None,
        collected_at: now,
        now,
    };
    let in_experiment = create_sample(&store, sample_for(Some(selected.id)), &audit)
        .await
        .unwrap();
    let unlinked = create_sample(&store, sample_for(None), &audit)
        .await
        .unwrap();

    let in_scope = |cell: AiObservationDataCell| {
        let store = &store;
        async move {
            ai_data_cell_in_experiment_scope(store, lab.id, selected_project.id, selected.id, &cell)
                .await
                .unwrap()
        }
    };
    assert!(in_scope(cell(ObservationSubjectType::Experiment, selected.id)).await);
    assert!(!in_scope(cell(ObservationSubjectType::Experiment, other.id)).await);
    assert!(in_scope(cell(ObservationSubjectType::Animal, enrolled.id)).await);
    assert!(!in_scope(cell(ObservationSubjectType::Animal, bystander.id)).await);
    assert!(in_scope(cell(ObservationSubjectType::Sample, in_experiment.id)).await);
    assert!(!in_scope(cell(ObservationSubjectType::Sample, unlinked.id)).await);

    // Unknown subjects are out of scope rather than errors, so existence is never revealed.
    assert!(!in_scope(cell(ObservationSubjectType::Sample, Uuid::new_v4())).await);
    assert!(!in_scope(cell(ObservationSubjectType::Artifact, Uuid::new_v4())).await);
}
