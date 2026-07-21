use chrono::{DateTime, Duration, Utc};
use muriarc_application::{
    CreateExperimentCommand, CreateExperimentEventCommand, CreateObservationDefinitionCommand,
    CreateParticipationCommand, CreateProjectCommand, RecordObservationCommand,
    ReviseObservationValueCommand, create_experiment, create_experiment_event,
    create_observation_definition, create_participation, create_project, record_observation,
    revise_observation_value,
};
use muriarc_core::{
    Animal, AuditContext, Lab, MuriArcStore, ObservationPolicy, ObservationSubjectType,
    ObservationValueData, ObservationValueType, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use serde_json::json;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T14:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn observation_values_are_scoped_and_versioned() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Observation application test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = create_project(
        &store,
        CreateProjectCommand {
            lab_id: lab.id,
            name: "Longitudinal study".to_owned(),
            description: None,
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
            name: "PH model".to_owned(),
            description: None,
            starts_at: Some(now),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let animal = Animal::new_mouse(lab.id, "OBS-001", Sex::Female, now).unwrap();
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

    let event = create_experiment_event(
        &store,
        CreateExperimentEventCommand {
            lab_id: lab.id,
            project_id: project.id,
            experiment_id: experiment.id,
            event_key: "day_7".to_owned(),
            label: "Day 7 assessment".to_owned(),
            occurred_at: now + Duration::days(7),
            details: json!({"phase": "follow_up"}),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let definition = create_observation_definition(
        &store,
        CreateObservationDefinitionCommand {
            lab_id: lab.id,
            project_id: project.id,
            experiment_id: experiment.id,
            key: "rvsp".to_owned(),
            label: "RV systolic pressure".to_owned(),
            value_type: ObservationValueType::Number,
            unit: Some("mmHg".to_owned()),
            categories: Vec::new(),
            policy: ObservationPolicy::Versioned,
            now,
        },
        &audit,
    )
    .await
    .unwrap();

    let recorded = record_observation(
        &store,
        RecordObservationCommand {
            lab_id: lab.id,
            project_id: project.id,
            experiment_id: experiment.id,
            experiment_event_id: event.id,
            definition_id: definition.id,
            subject_type: ObservationSubjectType::Animal,
            subject_id: animal.id,
            context: json!({"instrument": "Millar"}),
            value: ObservationValueData::Number(35.2),
            recorded_at: now + Duration::days(7),
            recorded_by: None,
            notes: Some("first pass".to_owned()),
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(recorded.observation.current_value_version, 1);

    let revised = revise_observation_value(
        &store,
        ReviseObservationValueCommand {
            observation_id: recorded.observation.id,
            expected_revision: 1,
            value: ObservationValueData::Number(34.8),
            recorded_at: now + Duration::days(7) + Duration::minutes(5),
            recorded_by: None,
            notes: Some("calibrated".to_owned()),
            now: now + Duration::minutes(5),
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(revised.observation.current_value_version, 2);
    assert_eq!(revised.observation.meta.revision, 2);
    let values = store
        .list_observation_values(recorded.observation.id)
        .await
        .unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].version, 1);
    assert_eq!(values[1].version, 2);

    let stale = revise_observation_value(
        &store,
        ReviseObservationValueCommand {
            observation_id: recorded.observation.id,
            expected_revision: 1,
            value: ObservationValueData::Number(34.7),
            recorded_at: now + Duration::days(7) + Duration::minutes(6),
            recorded_by: None,
            notes: None,
            now: now + Duration::minutes(6),
        },
        &audit,
    )
    .await
    .unwrap_err();
    assert!(stale.to_string().contains("revision"));
}
