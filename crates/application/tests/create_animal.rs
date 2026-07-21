use chrono::{DateTime, NaiveDate, Utc};
use muriarc_application::{CreateAnimalCommand, CreateAnimalIdentifierScope, create_animal};
use muriarc_core::{
    AnimalEventKind, AuditAction, AuditContext, AuditFilter, Cage, EntityType, IdentifierScope,
    Lab, MuriArcStore, Project, Sex, WriteSource,
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
