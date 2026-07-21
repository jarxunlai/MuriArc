use chrono::{TimeZone, Utc};
use muriarc_core::*;
use uuid::Uuid;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 8, 0, 0).unwrap()
}

#[test]
fn event_updates_current_projection_and_revision() {
    let mut animal = Animal::new_mouse(Uuid::new_v4(), "M001", Sex::Male, now()).unwrap();
    let cage_id = Uuid::new_v4();
    let event = AnimalEvent::new(
        animal.lab_id,
        animal.id,
        AnimalEventKind::Transferred {
            from_cage_id: None,
            to_cage_id: Some(cage_id),
        },
        now(),
        now(),
    );

    animal.apply_event(&event).unwrap();

    assert_eq!(animal.current_cage_id, Some(cage_id));
    assert_eq!(animal.meta.revision, 2);
}

#[test]
fn terminal_animal_cannot_be_revived() {
    let mut animal = Animal::new_mouse(Uuid::new_v4(), "M002", Sex::Female, now()).unwrap();
    let death = AnimalEvent::new(
        animal.lab_id,
        animal.id,
        AnimalEventKind::StatusChanged {
            from: AnimalStatus::Alive,
            to: AnimalStatus::Deceased,
        },
        now(),
        now(),
    );
    animal.apply_event(&death).unwrap();
    let revival = AnimalEvent::new(
        animal.lab_id,
        animal.id,
        AnimalEventKind::StatusChanged {
            from: AnimalStatus::Deceased,
            to: AnimalStatus::Alive,
        },
        now(),
        now(),
    );
    assert!(matches!(
        animal.apply_event(&revival),
        Err(DomainError::InvalidStatusTransition { .. })
    ));
}

#[test]
fn published_template_is_immutable() {
    let mut template =
        ExperimentTemplateVersion::draft(Uuid::new_v4(), "weight", 1, "Weight", now()).unwrap();
    template.publish(Uuid::new_v4(), now()).unwrap();
    assert_eq!(
        template.replace_fields(Vec::new(), now()),
        Err(DomainError::PublishedTemplateImmutable)
    );
}

#[test]
fn rejects_non_finite_measurement() {
    let result = Measurement::draft(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        "weight",
        "Weight",
        MeasurementValue::Number(f64::NAN),
        now(),
        now(),
    );
    assert!(matches!(result, Err(DomainError::NonFiniteMeasurement)));
}

#[test]
fn a_measurement_can_only_be_signed_once() {
    let mut measurement = Measurement::draft(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        "weight",
        "Weight",
        MeasurementValue::Number(21.5),
        now(),
        now(),
    )
    .unwrap();
    let signer = Uuid::new_v4();

    measurement.sign(signer, now()).unwrap();

    assert_eq!(measurement.status, RecordStatus::Signed);
    assert_eq!(measurement.signed_by, Some(signer));
    assert_eq!(measurement.meta.revision, 2);
    assert_eq!(
        measurement.sign(signer, now()),
        Err(DomainError::MeasurementNotDraft)
    );
}

#[test]
fn draft_measurement_rejects_orphaned_signature_fields() {
    let mut measurement = Measurement::draft(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        "score",
        "Clinical score",
        MeasurementValue::Number(1.0),
        now(),
        now(),
    )
    .unwrap();
    measurement.signed_by = Some(Uuid::new_v4());

    assert_eq!(
        measurement.validate_record(),
        Err(DomainError::InvalidMeasurementSignatureState)
    );
}

#[test]
fn cage_capacity_and_transfer_selection_are_validated_in_the_domain() {
    let lab_id = Uuid::new_v4();
    let mut cage = Cage::new(lab_id, "SPF-A", "A01", now()).unwrap();
    assert_eq!(cage.capacity, 5);
    assert!(matches!(
        cage.set_capacity(0),
        Err(DomainError::InvalidCageCapacity)
    ));
    cage.set_capacity(8).unwrap();
    assert_eq!(cage.capacity, 8);

    let animal_id = Uuid::new_v4();
    assert!(matches!(
        AnimalTransfer::new(lab_id, vec![], cage.id, now(), now()),
        Err(DomainError::EmptyAnimalSelection)
    ));
    assert!(matches!(
        AnimalTransfer::new(lab_id, vec![animal_id, animal_id], cage.id, now(), now()),
        Err(DomainError::DuplicateAnimalSelection)
    ));

    let oversized_selection = (0..=MAX_TRANSFER_ANIMALS).map(|_| Uuid::new_v4()).collect();
    assert_eq!(
        AnimalTransfer::new(lab_id, oversized_selection, cage.id, now(), now()),
        Err(DomainError::TransferSelectionTooLarge {
            maximum: MAX_TRANSFER_ANIMALS
        })
    );
}

#[test]
fn user_identity_is_normalized_and_status_changes_are_revisioned() {
    let lab_id = Uuid::new_v4();
    let mut user = User::new(lab_id, "  Researcher@Example.ORG ", "  Researcher  ", now()).unwrap();

    assert_eq!(user.email, "researcher@example.org");
    assert_eq!(user.display_name, "Researcher");
    user.suspend(now());
    assert_eq!(user.status, UserStatus::Suspended);
    assert_eq!(user.meta.revision, 2);
    user.suspend(now());
    assert_eq!(user.meta.revision, 2);
    user.reactivate(now());
    assert_eq!(user.status, UserStatus::Active);
    assert_eq!(user.meta.revision, 3);
    assert_eq!(
        User::new(lab_id, "not-an-email", "User", now()),
        Err(DomainError::InvalidUserEmail)
    );
    assert_eq!(
        User::new(lab_id, "user@example.org", "bad\nname", now()),
        Err(DomainError::InvalidUserDisplayName)
    );
    assert_eq!(
        User::new(lab_id, "user@example.org", "x".repeat(201), now()),
        Err(DomainError::InvalidUserDisplayName)
    );
}

#[test]
fn membership_role_changes_preserve_scope_and_revision() {
    let lab_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let mut lab_membership = Membership::lab(lab_id, user_id, LabRole::LabAdmin, now());
    lab_membership
        .change_lab_role(LabRole::AnimalManager, now())
        .unwrap();
    assert_eq!(lab_membership.lab_role, Some(LabRole::AnimalManager));
    assert_eq!(lab_membership.meta.revision, 2);
    assert_eq!(
        lab_membership.change_project_role(ProjectRole::Viewer, now()),
        Err(DomainError::InvalidMembershipScope)
    );

    let mut project_membership =
        Membership::project(lab_id, Uuid::new_v4(), user_id, ProjectRole::Viewer, now());
    project_membership
        .change_project_role(ProjectRole::Editor, now())
        .unwrap();
    assert_eq!(project_membership.project_role, Some(ProjectRole::Editor));
    assert_eq!(
        project_membership.change_lab_role(LabRole::LabAdmin, now()),
        Err(DomainError::InvalidMembershipScope)
    );
    project_membership.soft_delete(now());
    assert!(project_membership.meta.deleted_at.is_some());
    assert_eq!(project_membership.meta.revision, 3);
}
