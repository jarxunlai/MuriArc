use chrono::{DateTime, Utc};
use muriarc_application::{
    ApplicationError, ArchiveCageCommand, CreateCageCommand, TransferAnimalsCommand,
    UpdateCageCommand, archive_cage, create_cage, transfer_animals, update_cage,
};
use muriarc_core::{
    Animal, AnimalEventKind, AuditContext, AuditFilter, CageKind, DomainError, EntityType, Lab,
    MuriArcStore, Sex, StoreError, WriteSource,
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

#[tokio::test]
async fn cage_updates_merge_partial_edits_and_archive_only_empty_cages() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = fixed_now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Application cage update test", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let other_lab = Lab::new("Other application lab", now).unwrap();
    store.create_lab(&other_lab, &audit).await.unwrap();
    let cage = create_cage(
        &store,
        CreateCageCommand {
            lab_id: lab.id,
            section: "SPF-A".to_owned(),
            display_id: "A01".to_owned(),
            location: Some("rack-1".to_owned()),
            kind: CageKind::Standard,
            capacity: 5,
            sort_order: 2,
            now,
        },
        &audit,
    )
    .await
    .unwrap();
    let command = |expected_revision: i64| UpdateCageCommand {
        lab_id: lab.id,
        cage_id: cage.id,
        expected_revision,
        location: None,
        capacity: None,
        sort_order: None,
        now,
    };

    assert!(matches!(
        update_cage(&store, command(cage.meta.revision), &audit).await,
        Err(ApplicationError::Validation(_))
    ));
    assert!(matches!(
        update_cage(
            &store,
            UpdateCageCommand {
                capacity: Some(0),
                ..command(cage.meta.revision)
            },
            &audit,
        )
        .await,
        Err(ApplicationError::Domain(DomainError::InvalidCageCapacity))
    ));
    assert!(matches!(
        update_cage(
            &store,
            UpdateCageCommand {
                lab_id: other_lab.id,
                capacity: Some(8),
                ..command(cage.meta.revision)
            },
            &audit,
        )
        .await,
        Err(ApplicationError::Store(StoreError::NotFound { .. }))
    ));
    assert!(matches!(
        update_cage(
            &store,
            UpdateCageCommand {
                location: Some("x".repeat(129)),
                ..command(cage.meta.revision)
            },
            &audit,
        )
        .await,
        Err(ApplicationError::TooLong {
            field: "cage.location",
            ..
        })
    ));

    // Only the supplied field changes; omitted fields keep their stored value.
    let resized = update_cage(
        &store,
        UpdateCageCommand {
            capacity: Some(8),
            ..command(cage.meta.revision)
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(resized.capacity, 8);
    assert_eq!(resized.location.as_deref(), Some("rack-1"));
    assert_eq!(resized.sort_order, 2);

    // A blank location clears it after normalization.
    let relocated = update_cage(
        &store,
        UpdateCageCommand {
            location: Some("   ".to_owned()),
            sort_order: Some(9),
            ..command(resized.meta.revision)
        },
        &audit,
    )
    .await
    .unwrap();
    assert_eq!(relocated.location, None);
    assert_eq!(relocated.capacity, 8);
    assert_eq!(relocated.sort_order, 9);
    assert!(matches!(
        update_cage(
            &store,
            UpdateCageCommand {
                capacity: Some(4),
                ..command(resized.meta.revision)
            },
            &audit,
        )
        .await,
        Err(ApplicationError::Store(StoreError::Conflict(_)))
    ));

    let animal = Animal::new_mouse(lab.id, "M-ARCHIVE", Sex::Female, now).unwrap();
    store.create_animal(&animal, &audit).await.unwrap();
    transfer_animals(
        &store,
        TransferAnimalsCommand {
            lab_id: lab.id,
            animal_ids: vec![animal.id],
            target_cage_id: cage.id,
            occurred_at: now,
            recorded_at: now,
            recorded_by: None,
            notes: None,
        },
        &audit,
    )
    .await
    .unwrap();
    let archive = |expected_revision: i64| ArchiveCageCommand {
        lab_id: lab.id,
        cage_id: cage.id,
        expected_revision,
        now,
    };
    assert!(matches!(
        archive_cage(&store, archive(relocated.meta.revision), &audit).await,
        Err(ApplicationError::Store(StoreError::Conflict(_)))
    ));
    store
        .append_animal_event(
            &muriarc_core::AnimalEvent::new(
                lab.id,
                animal.id,
                AnimalEventKind::Transferred {
                    from_cage_id: Some(cage.id),
                    to_cage_id: None,
                },
                now,
                now,
            ),
            &audit,
        )
        .await
        .unwrap();
    let archived = archive_cage(&store, archive(relocated.meta.revision), &audit)
        .await
        .unwrap();
    assert!(archived.meta.deleted_at.is_some());
    assert!(store.list_cages(lab.id).await.unwrap().is_empty());
}
