use chrono::{DateTime, Utc};
use muriarc_application::{
    CreateCageCommand, TransferAnimalsCommand, create_cage, transfer_animals,
};
use muriarc_core::{
    Animal, AnimalEventKind, AuditContext, AuditFilter, CageKind, EntityType, Lab, MuriArcStore,
    Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn cage_creation_and_transfer_share_one_application_boundary() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Application cage test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();

    let cage = create_cage(
        &store,
        CreateCageCommand {
            lab_id: lab.id,
            section: "  SPF-A  ".to_owned(),
            display_id: "  A01  ".to_owned(),
            location: Some("  rack-1  ".to_owned()),
            kind: CageKind::Breeding,
            capacity: 2,
            sort_order: 7,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(cage.section, "SPF-A");
    assert_eq!(cage.display_id, "A01");
    assert_eq!(cage.location.as_deref(), Some("rack-1"));
    assert_eq!(cage.kind, CageKind::Breeding);
    assert_eq!(cage.capacity, 2);

    let animal = Animal::new_mouse(lab.id, "M-001", Sex::Female, now).unwrap();
    store.create_animal(&animal, &audit).await.unwrap();
    let actor_id = uuid::Uuid::new_v4();
    let moved = transfer_animals(
        &store,
        TransferAnimalsCommand {
            lab_id: lab.id,
            animal_ids: vec![animal.id],
            target_cage_id: cage.id,
            occurred_at: now,
            recorded_at: now,
            recorded_by: Some(actor_id),
            notes: Some("  move for pairing  ".to_owned()),
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(moved[0].current_cage_id, Some(cage.id));

    let event = store
        .list_animal_events(animal.id)
        .await
        .unwrap()
        .into_iter()
        .find(|event| matches!(event.kind, AnimalEventKind::Transferred { .. }))
        .unwrap();
    assert_eq!(event.recorded_by, Some(actor_id));
    assert_eq!(event.notes.as_deref(), Some("move for pairing"));

    let audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: Some(cage.id),
        })
        .await
        .unwrap();
    assert!(
        audits
            .iter()
            .any(|entry| entry.entity_type == EntityType::Cage)
    );
}
