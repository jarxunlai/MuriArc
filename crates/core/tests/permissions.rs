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
fn viewer_cannot_write_or_import() {
    let access = ActorAccess::human([], [ProjectRole::Viewer]);
    assert!(access.allows(Permission::ReadMeasurement));
    assert!(access.allows(Permission::ExportData));
    assert!(!access.allows(Permission::WriteMeasurementDraft));
    assert!(!access.allows(Permission::ImportData));
}
