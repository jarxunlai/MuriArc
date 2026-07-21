use muriarc_core::*;

#[test]
fn ai_permissions_are_human_permissions_intersected_with_scopes() {
    let access = ActorAccess::human([], [ProjectRole::Editor])
        .with_ai_scopes([AiScope::Read, AiScope::WriteDraft]);

    assert!(access.allows(Permission::ReadAnimal));
    assert!(access.allows(Permission::WriteMeasurementDraft));
    assert!(!access.allows(Permission::WriteExperiment));
    assert!(!access.allows(Permission::ImportData));
    assert!(!access.allows(Permission::PublishTemplate));
}

#[test]
fn viewer_cannot_write_import_or_bulk_export() {
    let access = ActorAccess::human([], [ProjectRole::Viewer]);
    assert!(access.allows(Permission::ReadMeasurement));
    assert!(access.allows(Permission::ReadActivity));
    assert!(!access.allows(Permission::ExportData));
    assert!(!access.allows(Permission::WriteMeasurementDraft));
    assert!(!access.allows(Permission::ImportData));
}

#[test]
fn project_admin_manages_the_project_without_lab_animal_or_audit_authority() {
    let access = ActorAccess::human([], [ProjectRole::ProjectAdmin]);
    assert!(access.allows(Permission::ManageProject));
    assert!(access.allows(Permission::WriteExperiment));
    assert!(access.allows(Permission::ReadActivity));
    assert!(!access.allows(Permission::ManageProjectAnimals));
    assert!(!access.allows(Permission::ManageCage));
    assert!(!access.allows(Permission::ReadAudit));
}
