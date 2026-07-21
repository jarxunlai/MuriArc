use chrono::{Duration, Utc};

use crate::*;

/// PostgreSQL stores `TIMESTAMPTZ` at microsecond precision. Shared adapter
/// contracts must construct portable timestamps so exact round-trip equality
/// has the same meaning for PostgreSQL and SQLite.
fn contract_now() -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp_micros(Utc::now().timestamp_micros())
        .expect("the current UTC timestamp is representable")
}

async fn assign_project_animal(
    store: &dyn MuriArcStore,
    lab_id: uuid::Uuid,
    project_id: uuid::Uuid,
    animal_id: uuid::Uuid,
    audit: &AuditContext,
    now: chrono::DateTime<Utc>,
) -> ProjectAnimalAssignment {
    let assignment = ProjectAnimalAssignment::new(
        lab_id,
        project_id,
        animal_id,
        audit.actor.user_id,
        Some("store contract fixture".to_owned()),
        now,
    );
    store
        .assign_animals_to_project(std::slice::from_ref(&assignment), audit)
        .await
        .expect("project animal assignment succeeds");
    assignment
}

/// Runs the behavior contract shared by every persistence adapter.
/// The target store must already be connected to an isolated database.
pub async fn run_store_contract(store: &dyn MuriArcStore) {
    store.migrate().await.expect("migration succeeds");
    store.health_check().await.expect("store is healthy");

    let now = contract_now();
    let audit = AuditContext::system(WriteSource::Migration);
    let mut lab = Lab::new(format!("Contract Lab {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    assert_eq!(store.get_lab(lab.id).await.unwrap(), lab);
    let lab_revision = lab.meta.revision;
    lab.rename("Renamed Contract Lab", now + Duration::milliseconds(1))
        .unwrap();
    store.update_lab(&lab, lab_revision, &audit).await.unwrap();
    assert_eq!(store.get_lab(lab.id).await.unwrap(), lab);
    assert!(matches!(
        store.update_lab(&lab, lab_revision, &audit).await,
        Err(StoreError::Conflict(_))
    ));

    let mut user = User::new(
        lab.id,
        format!("{}@example.test", uuid::Uuid::new_v4()),
        "Contract User",
        now,
    )
    .unwrap();
    store.create_user(&user, &audit).await.unwrap();
    assert_eq!(store.get_user(user.id).await.unwrap(), user);
    let user_revision = user.meta.revision;
    user.rename("Renamed Contract User", now + Duration::milliseconds(2))
        .unwrap();
    store
        .update_user(&user, user_revision, &audit)
        .await
        .unwrap();
    assert_eq!(store.get_user(user.id).await.unwrap(), user);
    assert!(matches!(
        store.update_user(&user, user_revision, &audit).await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .list_users(&UserFilter {
                lab_id: lab.id,
                status: Some(UserStatus::Active),
            })
            .await
            .unwrap(),
        vec![user.clone()]
    );
    let active_revision = user.meta.revision;
    user.suspend(now + Duration::milliseconds(3));
    store
        .update_user(&user, active_revision, &audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_users(&UserFilter {
                lab_id: lab.id,
                status: Some(UserStatus::Suspended),
            })
            .await
            .unwrap(),
        vec![user.clone()]
    );
    let suspended_revision = user.meta.revision;
    user.reactivate(now + Duration::milliseconds(4));
    store
        .update_user(&user, suspended_revision, &audit)
        .await
        .unwrap();

    let membership = Membership::lab(lab.id, user.id, LabRole::LabAdmin, now);
    store.create_membership(&membership, &audit).await.unwrap();
    assert_eq!(
        store
            .list_memberships(&MembershipFilter {
                lab_id: lab.id,
                user_id: Some(user.id),
                project_id: None,
            })
            .await
            .unwrap(),
        vec![membership.clone()]
    );
    assert_eq!(
        store.get_membership(membership.id).await.unwrap(),
        membership
    );

    let project = Project::new(lab.id, "DEMO Contract", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();
    assert_eq!(store.get_project(project.id).await.unwrap(), project);
    assert!(
        store
            .list_projects(lab.id)
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == project.id)
    );

    let mut scoped_animal = Animal::new_mouse(
        lab.id,
        format!("PROJECT-SCOPED-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    scoped_animal.identifier_scope = IdentifierScope::Project {
        project_id: project.id,
    };
    store.create_animal(&scoped_animal, &audit).await.unwrap();

    let mut unknown_scope = Animal::new_mouse(
        lab.id,
        format!("UNKNOWN-SCOPE-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    unknown_scope.identifier_scope = IdentifierScope::Project {
        project_id: uuid::Uuid::new_v4(),
    };
    assert!(matches!(
        store.create_animal(&unknown_scope, &audit).await,
        Err(StoreError::NotFound {
            entity: "project",
            ..
        })
    ));

    let other_lab = Lab::new(format!("Other Lab {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&other_lab, &audit).await.unwrap();
    let other_project = Project::new(other_lab.id, "Other project", now).unwrap();
    store.create_project(&other_project, &audit).await.unwrap();
    let mut cross_lab_scope = Animal::new_mouse(
        lab.id,
        format!("CROSS-LAB-SCOPE-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    cross_lab_scope.identifier_scope = IdentifierScope::Project {
        project_id: other_project.id,
    };
    assert!(matches!(
        store.create_animal(&cross_lab_scope, &audit).await,
        Err(StoreError::Validation(_))
    ));

    let removable_assignment =
        assign_project_animal(store, lab.id, project.id, scoped_animal.id, &audit, now).await;
    let removed_assignments = store
        .remove_animals_from_project(
            &[ProjectAnimalAssignmentRemoval {
                assignment_id: removable_assignment.id,
                expected_revision: removable_assignment.meta.revision,
            }],
            now + Duration::milliseconds(1),
            &audit,
        )
        .await
        .unwrap();
    assert_eq!(removed_assignments.len(), 1);
    assert!(removed_assignments[0].meta.deleted_at.is_some());
    assert!(
        store
            .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                animal_id: Some(scoped_animal.id),
            })
            .await
            .unwrap()
            .is_empty()
    );

    let mut project_membership =
        Membership::project(lab.id, project.id, user.id, ProjectRole::Viewer, now);
    store
        .create_membership(&project_membership, &audit)
        .await
        .unwrap();
    let project_membership_revision = project_membership.meta.revision;
    project_membership
        .change_project_role(ProjectRole::Editor, now + Duration::milliseconds(5))
        .unwrap();
    store
        .update_membership(&project_membership, project_membership_revision, &audit)
        .await
        .unwrap();
    assert_eq!(
        store.get_membership(project_membership.id).await.unwrap(),
        project_membership
    );
    assert!(matches!(
        store
            .update_membership(&project_membership, project_membership_revision, &audit,)
            .await,
        Err(StoreError::Conflict(_))
    ));
    let deleted = store
        .soft_delete_membership(
            project_membership.id,
            project_membership.meta.revision,
            now + Duration::milliseconds(6),
            &audit,
        )
        .await
        .unwrap();
    assert!(deleted.meta.deleted_at.is_some());
    assert!(matches!(
        store.get_membership(project_membership.id).await,
        Err(StoreError::NotFound { .. })
    ));

    let cage = Cage::new(lab.id, "A", format!("C-{}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_cage(&cage, &audit).await.unwrap();
    assert_eq!(store.get_cage(cage.id).await.unwrap(), cage);
    assert!(
        store
            .list_cages(lab.id)
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == cage.id)
    );

    let mut animal = Animal::new_mouse(
        lab.id,
        format!("M-{}", uuid::Uuid::new_v4()),
        Sex::Female,
        now,
    )
    .unwrap();
    animal.current_cage_id = Some(cage.id);
    store.create_animal(&animal, &audit).await.unwrap();
    assert_eq!(store.get_animal(animal.id).await.unwrap(), animal);
    let assignment = assign_project_animal(store, lab.id, project.id, animal.id, &audit, now).await;
    assert_eq!(
        store
            .list_project_animal_assignments(&ProjectAnimalAssignmentFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                animal_id: Some(animal.id),
            })
            .await
            .unwrap(),
        vec![assignment.clone()]
    );
    assert_eq!(
        store
            .list_cages_for_project(lab.id, project.id)
            .await
            .unwrap(),
        vec![cage.clone()]
    );

    let event = AnimalEvent::new(
        lab.id,
        animal.id,
        AnimalEventKind::Transferred {
            from_cage_id: Some(cage.id),
            to_cage_id: None,
        },
        now,
        now,
    );
    let projected = store.append_animal_event(&event, &audit).await.unwrap();
    assert_eq!(projected.current_cage_id, None);
    assert_eq!(projected.meta.revision, animal.meta.revision + 1);
    assert!(
        store
            .list_animal_events(animal.id)
            .await
            .unwrap()
            .contains(&event)
    );

    let second_animal = Animal::new_mouse(
        lab.id,
        format!("M-{}", uuid::Uuid::new_v4()),
        Sex::Male,
        now,
    )
    .unwrap();
    store.create_animal(&second_animal, &audit).await.unwrap();

    let mut undersized_target =
        Cage::new(lab.id, "B", format!("C-{}", uuid::Uuid::new_v4()), now).unwrap();
    undersized_target.set_capacity(1).unwrap();
    store.create_cage(&undersized_target, &audit).await.unwrap();
    let oversized_transfer = AnimalTransfer {
        lab_id: lab.id,
        animal_ids: (0..=MAX_TRANSFER_ANIMALS)
            .map(|_| uuid::Uuid::new_v4())
            .collect(),
        target_cage_id: undersized_target.id,
        occurred_at: now,
        recorded_at: now,
        recorded_by: Some(user.id),
        notes: None,
    };
    assert!(matches!(
        store.transfer_animals(&oversized_transfer, &audit).await,
        Err(StoreError::Validation(message)) if message.contains("cannot contain more than")
    ));

    let rejected_transfer = AnimalTransfer::new(
        lab.id,
        vec![animal.id, second_animal.id],
        undersized_target.id,
        now,
        now,
    )
    .unwrap();
    assert!(matches!(
        store.transfer_animals(&rejected_transfer, &audit).await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store.get_animal(animal.id).await.unwrap().current_cage_id,
        None
    );
    assert_eq!(
        store
            .get_animal(second_animal.id)
            .await
            .unwrap()
            .current_cage_id,
        None
    );
    assert_eq!(
        store
            .list_animal_events(second_animal.id)
            .await
            .unwrap()
            .iter()
            .filter(|event| matches!(event.kind, AnimalEventKind::Transferred { .. }))
            .count(),
        0
    );

    let mut target = Cage::new(lab.id, "B", format!("C-{}", uuid::Uuid::new_v4()), now).unwrap();
    target.set_capacity(2).unwrap();
    store.create_cage(&target, &audit).await.unwrap();
    let mut accepted_transfer = AnimalTransfer::new(
        lab.id,
        vec![animal.id, second_animal.id],
        target.id,
        now,
        now,
    )
    .unwrap();
    accepted_transfer.recorded_by = Some(user.id);
    let projected = store
        .transfer_animals(&accepted_transfer, &audit)
        .await
        .unwrap();
    assert_eq!(projected.len(), 2);
    assert!(
        projected
            .iter()
            .all(|animal| animal.current_cage_id == Some(target.id))
    );
    assert!(
        store
            .list_animal_events(second_animal.id)
            .await
            .unwrap()
            .iter()
            .any(|event| {
                event.recorded_by == Some(user.id)
                    && matches!(
                        event.kind,
                        AnimalEventKind::Transferred { to_cage_id: Some(id), .. } if id == target.id
                    )
            })
    );

    let template = ExperimentTemplateVersion::draft(
        lab.id,
        format!("contract-template-{}", uuid::Uuid::new_v4()),
        1,
        "Contract template",
        now,
    )
    .unwrap();
    store
        .create_template_version(&template, &audit)
        .await
        .unwrap();
    let publication_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Web,
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        reason: Some("contract template publication".to_owned()),
    };
    let published_template = store
        .publish_template_version(
            template.id,
            template.meta.revision,
            user.id,
            now + Duration::milliseconds(3),
            &publication_audit,
        )
        .await
        .unwrap();
    assert_eq!(published_template.status, TemplateStatus::Published);
    assert_eq!(published_template.published_by, Some(user.id));
    assert_eq!(published_template.meta.revision, template.meta.revision + 1);
    assert_eq!(
        store.get_template_version(template.id).await.unwrap(),
        published_template
    );
    assert!(matches!(
        store
            .publish_template_version(
                template.id,
                template.meta.revision,
                user.id,
                now + Duration::milliseconds(4),
                &publication_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));

    let mut experiment = Experiment::new(lab.id, project.id, "DEMO", now).unwrap();
    experiment.status = ExperimentStatus::Active;
    experiment.template_version_id = Some(published_template.id);
    store.create_experiment(&experiment, &audit).await.unwrap();
    assert_eq!(
        store.get_experiment(experiment.id).await.unwrap(),
        experiment
    );
    assert_eq!(
        store
            .list_experiments(&ExperimentFilter {
                project_id: project.id,
                status: Some(ExperimentStatus::Active)
            })
            .await
            .unwrap()
            .len(),
        1
    );

    let participation = Participation::enroll(experiment.id, animal.id, now);
    store
        .create_participation(&participation, &audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_participations(&ParticipationFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal.id),
                cohort_id: None,
            })
            .await
            .unwrap(),
        vec![participation.clone()]
    );
    assert_eq!(
        store
            .list_animals(&AnimalFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                ..AnimalFilter::default()
            })
            .await
            .unwrap()
            .len(),
        1
    );

    let enrolled_projection = store.get_animal(animal.id).await.unwrap();
    assert_eq!(
        enrolled_projection.current_status,
        AnimalStatus::InExperiment
    );
    assert!(store.list_animal_events(animal.id).await.unwrap().iter().any(|event| {
        matches!(event.kind, AnimalEventKind::ExperimentEnrolled { participation_id } if participation_id == participation.id)
    }));
    let procedure = Procedure {
        id: uuid::Uuid::new_v4(),
        experiment_id: experiment.id,
        animal_id: Some(animal.id),
        name: "Contract completed procedure".to_owned(),
        scheduled_at: Some(now),
        performed_at: Some(now),
        status: ProcedureStatus::Completed,
        details: serde_json::json!({}),
        meta: RecordMeta::new(now),
    };
    store.create_procedure(&procedure, &audit).await.unwrap();
    assert!(store.list_animal_events(animal.id).await.unwrap().iter().any(|event| {
        matches!(event.kind, AnimalEventKind::ProcedurePerformed { procedure_id } if procedure_id == procedure.id)
    }));

    let mut measurement = Measurement::draft(
        lab.id,
        project.id,
        animal.id,
        "body_weight",
        "Body weight",
        MeasurementValue::Number(22.4),
        now,
        now,
    )
    .unwrap();
    measurement.experiment_id = Some(experiment.id);
    measurement.unit = Some("g".to_owned());
    store
        .create_measurement(&measurement, &audit)
        .await
        .unwrap();
    assert_eq!(
        store.get_measurement(measurement.id).await.unwrap(),
        measurement
    );
    assert_eq!(
        store
            .list_measurements(&MeasurementFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal.id),
            })
            .await
            .unwrap()
            .len(),
        1
    );

    let measurement_revision = measurement.meta.revision;
    measurement
        .sign(user.id, now + Duration::seconds(1))
        .unwrap();
    let signing_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Web,
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        reason: Some("contract measurement signature".to_owned()),
    };
    store
        .update_measurement(&measurement, measurement_revision, &signing_audit)
        .await
        .unwrap();
    assert_eq!(
        store.get_measurement(measurement.id).await.unwrap(),
        measurement
    );
    assert!(matches!(
        store
            .update_measurement(&measurement, measurement_revision, &signing_audit)
            .await,
        Err(StoreError::Conflict(_))
    ));

    let mut sample = Sample::new(lab.id, project.id, animal.id, "lung", now, now).unwrap();
    sample.experiment_id = Some(experiment.id);
    sample.set_quantity(1.0, "piece").unwrap();
    store.create_sample(&sample, &audit).await.unwrap();
    assert_eq!(store.get_sample(sample.id).await.unwrap(), sample);
    assert_eq!(
        store
            .list_samples(&SampleFilter {
                project_id: project.id,
                experiment_id: Some(experiment.id),
                animal_id: Some(animal.id),
            })
            .await
            .unwrap()
            .len(),
        1
    );

    let events = store.list_animal_events(animal.id).await.unwrap();
    assert!(events.iter().any(|event| {
        matches!(event.kind, AnimalEventKind::MeasurementRecorded { measurement_id } if measurement_id == measurement.id)
    }));
    assert!(events.iter().any(|event| {
        matches!(event.kind, AnimalEventKind::SampleCollected { sample_id, terminal: false } if sample_id == sample.id)
    }));
    let attachment = Attachment {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        entity_type: "animal".to_owned(),
        entity_id: animal.id,
        file_name: "contract.txt".to_owned(),
        media_type: Some("text/plain".to_owned()),
        relative_path: format!("attachments/{}.txt", uuid::Uuid::new_v4()),
        size_bytes: 1,
        sha256: "a".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    store.create_attachment(&attachment, &audit).await.unwrap();
    assert_eq!(
        store.get_attachment(attachment.id).await.unwrap(),
        attachment
    );
    for (entity_type, entity_id) in [
        (EntityType::Participation, participation.id),
        (EntityType::Procedure, procedure.id),
        (EntityType::Measurement, measurement.id),
        (EntityType::Sample, sample.id),
        (EntityType::Attachment, attachment.id),
    ] {
        let provenance = store
            .list_provenance(&ProvenanceFilter {
                lab_id: lab.id,
                entity_type: Some(entity_type),
                entity_id: Some(entity_id),
                ..ProvenanceFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(provenance[0].source, ProvenanceSource::Migration);
    }

    let mut second_experiment = Experiment::new(lab.id, project.id, "DEMO second", now).unwrap();
    second_experiment.status = ExperimentStatus::Active;
    second_experiment.template_version_id = Some(published_template.id);
    store
        .create_experiment(&second_experiment, &audit)
        .await
        .unwrap();
    let second_participation = Participation::enroll(second_experiment.id, animal.id, now);
    store
        .create_participation(&second_participation, &audit)
        .await
        .unwrap();

    let completed_participation = store
        .transition_participation(
            participation.id,
            ParticipationStatus::Completed,
            participation.meta.revision,
            now + Duration::seconds(2),
            &audit,
        )
        .await
        .unwrap();
    assert_eq!(
        completed_participation.status,
        ParticipationStatus::Completed
    );
    assert_eq!(
        store.get_participation(participation.id).await.unwrap(),
        completed_participation
    );
    assert_eq!(
        store.get_animal(animal.id).await.unwrap().current_status,
        AnimalStatus::InExperiment,
        "another open enrollment keeps the animal in experiment"
    );
    assert!(matches!(
        store
            .transition_participation(
                participation.id,
                ParticipationStatus::Withdrawn,
                participation.meta.revision,
                now + Duration::seconds(3),
                &audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));

    let cancelled_experiment = store
        .transition_experiment(
            second_experiment.id,
            ExperimentStatus::Cancelled,
            second_experiment.meta.revision,
            now + Duration::seconds(4),
            &audit,
        )
        .await
        .unwrap();
    assert_eq!(cancelled_experiment.status, ExperimentStatus::Cancelled);
    assert_eq!(
        store
            .get_participation(second_participation.id)
            .await
            .unwrap()
            .status,
        ParticipationStatus::Withdrawn
    );
    assert_eq!(
        store.get_animal(animal.id).await.unwrap().current_status,
        AnimalStatus::Alive
    );
    assert!(matches!(
        store
            .transition_experiment(
                second_experiment.id,
                ExperimentStatus::Completed,
                second_experiment.meta.revision,
                now + Duration::seconds(5),
                &audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));

    let lifecycle_animal = Animal::new_mouse(
        lab.id,
        format!("LIFECYCLE-{}", uuid::Uuid::new_v4()),
        Sex::Male,
        now,
    )
    .unwrap();
    store
        .create_animal(&lifecycle_animal, &audit)
        .await
        .unwrap();
    assign_project_animal(store, lab.id, project.id, lifecycle_animal.id, &audit, now).await;
    let mut completed_experiment =
        Experiment::new(lab.id, project.id, "Completed study", now).unwrap();
    completed_experiment.status = ExperimentStatus::Active;
    completed_experiment.template_version_id = Some(published_template.id);
    store
        .create_experiment(&completed_experiment, &audit)
        .await
        .unwrap();
    let auto_completed = Participation::enroll(completed_experiment.id, lifecycle_animal.id, now);
    store
        .create_participation(&auto_completed, &audit)
        .await
        .unwrap();
    store
        .transition_experiment(
            completed_experiment.id,
            ExperimentStatus::Completed,
            completed_experiment.meta.revision,
            now + Duration::seconds(6),
            &audit,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_participation(auto_completed.id)
            .await
            .unwrap()
            .status,
        ParticipationStatus::Completed
    );
    assert_eq!(
        store
            .get_animal(lifecycle_animal.id)
            .await
            .unwrap()
            .current_status,
        AnimalStatus::Alive
    );
    assert!(
        store
            .list_animal_events(lifecycle_animal.id)
            .await
            .unwrap()
            .iter()
            .any(|event| {
                matches!(
                    event.kind,
                    AnimalEventKind::ExperimentParticipationEnded {
                        participation_id,
                        status: ParticipationStatus::Completed
                    } if participation_id == auto_completed.id
                )
            })
    );

    let terminal_animal = Animal::new_mouse(
        lab.id,
        format!("TERMINAL-{}", uuid::Uuid::new_v4()),
        Sex::Female,
        now,
    )
    .unwrap();
    store.create_animal(&terminal_animal, &audit).await.unwrap();
    assign_project_animal(store, lab.id, project.id, terminal_animal.id, &audit, now).await;
    let mut terminal_experiment =
        Experiment::new(lab.id, project.id, "Terminal status study", now).unwrap();
    terminal_experiment.status = ExperimentStatus::Active;
    terminal_experiment.template_version_id = Some(published_template.id);
    store
        .create_experiment(&terminal_experiment, &audit)
        .await
        .unwrap();
    let terminal_participation =
        Participation::enroll(terminal_experiment.id, terminal_animal.id, now);
    store
        .create_participation(&terminal_participation, &audit)
        .await
        .unwrap();
    let terminal_at = now + Duration::seconds(7);
    let mut deceased_event = AnimalEvent::new(
        lab.id,
        terminal_animal.id,
        AnimalEventKind::StatusChanged {
            from: AnimalStatus::InExperiment,
            to: AnimalStatus::Deceased,
        },
        terminal_at,
        terminal_at,
    );
    deceased_event.project_id = Some(project.id);
    store
        .append_animal_event(&deceased_event, &audit)
        .await
        .unwrap();
    store
        .transition_participation(
            terminal_participation.id,
            ParticipationStatus::Completed,
            terminal_participation.meta.revision,
            now + Duration::seconds(8),
            &audit,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_animal(terminal_animal.id)
            .await
            .unwrap()
            .current_status,
        AnimalStatus::Deceased,
        "closing participation must not overwrite a terminal animal status"
    );

    let mut job = Job {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        created_by: user.id,
        kind: JobKind::Import,
        status: JobStatus::Queued,
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        progress_current: 0,
        progress_total: Some(10),
        result: None,
        error_report: None,
        cancellation_requested: false,
        meta: RecordMeta::new(now),
    };
    store.create_job(&job, &audit).await.unwrap();
    assert_eq!(store.get_job(job.id).await.unwrap(), job);
    assert_eq!(
        store
            .find_job_by_idempotency(lab.id, user.id, &job.idempotency_key)
            .await
            .unwrap(),
        Some(job.clone())
    );
    assert_eq!(
        store
            .list_jobs(&JobFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                created_by: Some(user.id),
            })
            .await
            .unwrap(),
        vec![job.clone()]
    );

    let expected_revision = job.meta.revision;
    job.status = JobStatus::Parsing;
    job.progress_current = 1;
    job.meta.touch(now + Duration::seconds(1));
    store
        .update_job(&job, expected_revision, &audit)
        .await
        .unwrap();
    assert_eq!(store.get_job(job.id).await.unwrap(), job);
    assert!(matches!(
        store.update_job(&job, expected_revision, &audit).await,
        Err(StoreError::Conflict(_))
    ));

    run_import_contract(store, now).await;
    run_relationship_contract(store, now).await;

    let audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    assert!(
        audits.len() >= 13,
        "every write should produce an audit entry"
    );
}

/// Runs the shared Genetics v2, Breeding and Observation persistence contract.
/// The target store may already be migrated, but must be connected to a test database.
pub async fn run_research_extensions_contract(store: &dyn MuriArcStore) {
    store.migrate().await.expect("migration succeeds");
    let now = contract_now();
    let setup_audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new(format!("Research Extensions {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&lab, &setup_audit).await.unwrap();
    let user = User::new(
        lab.id,
        format!("{}@research-contract.test", uuid::Uuid::new_v4()),
        "Researcher",
        now,
    )
    .unwrap();
    store.create_user(&user, &setup_audit).await.unwrap();
    let human_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Web,
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        reason: Some("shared research extensions contract".to_owned()),
    };
    let project = Project::new(lab.id, "Research project", now).unwrap();
    store.create_project(&project, &human_audit).await.unwrap();

    let male = Animal::new_mouse(lab.id, "BREED-M", Sex::Male, now).unwrap();
    let female = Animal::new_mouse(lab.id, "BREED-F", Sex::Female, now).unwrap();
    store.create_animal(&male, &human_audit).await.unwrap();
    store.create_animal(&female, &human_audit).await.unwrap();

    let locus = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        symbol: "ContractGene".to_owned(),
        description: None,
        meta: RecordMeta::new(now),
    };
    store.create_gene_locus(&locus, &human_audit).await.unwrap();
    let wild_type = Allele {
        id: uuid::Uuid::new_v4(),
        locus_id: locus.id,
        symbol: "+".to_owned(),
        description: None,
        is_wild_type: true,
        meta: RecordMeta::new(now),
    };
    let conditional = Allele {
        id: uuid::Uuid::new_v4(),
        locus_id: locus.id,
        symbol: "flox".to_owned(),
        description: None,
        is_wild_type: false,
        meta: RecordMeta::new(now),
    };
    store.create_allele(&wild_type, &human_audit).await.unwrap();
    store
        .create_allele(&conditional, &human_audit)
        .await
        .unwrap();

    let mut genotype_definition =
        GenotypeDefinition::new(lab.id, "ContractGene +/flox", now).unwrap();
    genotype_definition
        .replace_components(vec![
            GenotypeComponent::new(
                genotype_definition.id,
                locus.id,
                wild_type.id,
                Some(conditional.id),
                GenotypeComponentMode::Diploid,
                0,
                now,
            )
            .unwrap(),
        ])
        .unwrap();
    store
        .create_genotype_definition(&genotype_definition, &human_audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_genotype_definition(genotype_definition.id)
            .await
            .unwrap(),
        genotype_definition
    );

    let mut genotyping_record = GenotypingRecord::new(
        lab.id,
        female.id,
        genotype_definition.id,
        GenotypingState::Confirmed,
        Some(now),
        now,
    )
    .unwrap();
    genotyping_record.project_id = Some(project.id);
    genotyping_record.method = Some("PCR".to_owned());
    store
        .create_genotyping_record(&genotyping_record, &human_audit)
        .await
        .unwrap();
    assert_eq!(
        store.list_genotyping_records(female.id).await.unwrap(),
        vec![genotyping_record.clone()]
    );

    let mut line = BreedingLine::new(lab.id, "Contract line", now).unwrap();
    line.replace_genotype_definitions(vec![genotype_definition.id])
        .unwrap();
    store
        .create_breeding_line(&line, &human_audit)
        .await
        .unwrap();
    let colony = Colony::new(lab.id, line.id, "Contract colony", now).unwrap();
    store.create_colony(&colony, &human_audit).await.unwrap();
    let mut pair = BreedingPair::new(lab.id, colony.id, "Contract pair", now, now).unwrap();
    pair.replace_members(vec![
        BreedingPairMember::new(pair.id, male.id, BreedingMemberRole::Male, now, now).unwrap(),
        BreedingPairMember::new(pair.id, female.id, BreedingMemberRole::Female, now, now).unwrap(),
    ])
    .unwrap();
    store
        .create_breeding_pair(&pair, &human_audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_breeding_pair(pair.id)
            .await
            .unwrap()
            .members
            .len(),
        2
    );

    let mating = MatingEvent::new(lab.id, pair.id, male.id, female.id, now, now).unwrap();
    store
        .create_mating_event(&mating, &human_audit)
        .await
        .unwrap();
    let birth_date = (now + Duration::days(21)).date_naive();
    let litter = Litter::new(lab.id, mating.id, birth_date, 2, 1, now).unwrap();
    let draft = AnimalDraft::new(lab.id, litter.id, "P1", Sex::Female, birth_date, now).unwrap();
    store
        .create_litter(&litter, std::slice::from_ref(&draft), &human_audit)
        .await
        .unwrap();
    assert_eq!(
        store.list_animal_drafts(litter.id).await.unwrap(),
        vec![draft.clone()]
    );
    let mut offspring = Animal::new_mouse(lab.id, "BREED-P1", draft.sex, now).unwrap();
    offspring.birth_date = Some(draft.birth_date);
    let registered = store
        .register_animal_draft(draft.id, draft.meta.revision, &offspring, &human_audit)
        .await
        .unwrap();
    assert_eq!(registered.status, AnimalDraftStatus::Registered);
    assert_eq!(registered.registered_animal_id, Some(offspring.id));
    assert_eq!(
        store
            .list_related_pedigrees(offspring.id)
            .await
            .unwrap()
            .len(),
        2
    );
    let offspring_events = store.list_animal_events(offspring.id).await.unwrap();
    assert!(
        offspring_events
            .iter()
            .any(|event| matches!(event.kind, AnimalEventKind::Registered))
    );
    assert!(
        offspring_events
            .iter()
            .any(|event| matches!(event.kind, AnimalEventKind::Born { .. }))
    );

    let experiment = Experiment::new(lab.id, project.id, "Observation study", now).unwrap();
    store
        .create_experiment(&experiment, &human_audit)
        .await
        .unwrap();
    assign_project_animal(store, lab.id, project.id, female.id, &human_audit, now).await;
    let enrollment = Participation::enroll(experiment.id, female.id, now);
    let enrollment = store
        .create_participation(&enrollment, &human_audit)
        .await
        .unwrap();
    assert_eq!(
        enrollment.genotype_snapshot,
        vec![GenotypeSnapshotEntry {
            genotyping_record_id: genotyping_record.id,
            genotype_definition_id: genotype_definition.id,
            state: GenotypingState::Confirmed,
            assessed_at: Some(now),
        }]
    );

    let event = ExperimentEvent::new(
        lab.id,
        project.id,
        experiment.id,
        "day_7",
        "Day 7",
        now,
        now,
    )
    .unwrap();
    store
        .create_experiment_event(&event, &human_audit)
        .await
        .unwrap();
    let mut definition = ObservationDefinition::new(
        lab.id,
        project.id,
        experiment.id,
        "body_weight",
        "Body weight",
        ObservationValueType::Number,
        ObservationPolicy::Versioned,
        now,
    )
    .unwrap();
    definition.unit = Some("g".to_owned());
    definition.validate().unwrap();
    store
        .create_observation_definition(&definition, &human_audit)
        .await
        .unwrap();

    let mismatched_observation = Observation::new(
        lab.id,
        project.id,
        experiment.id,
        event.id,
        definition.id,
        ObservationSubjectType::Animal,
        female.id,
        now,
    )
    .unwrap();
    let mut mismatched_value = ObservationValueRecord::new(
        mismatched_observation.id,
        1,
        ObservationValueData::Number(22.0),
        now,
        now,
    )
    .unwrap();
    mismatched_value.recorded_by = Some(uuid::Uuid::new_v4());
    assert!(matches!(
        store
            .create_observation(&mismatched_observation, &mismatched_value, &human_audit)
            .await,
        Err(StoreError::Validation(_))
    ));
    assert!(matches!(
        store.get_observation(mismatched_observation.id).await,
        Err(StoreError::NotFound { .. })
    ));

    let observation = Observation::new(
        lab.id,
        project.id,
        experiment.id,
        event.id,
        definition.id,
        ObservationSubjectType::Animal,
        female.id,
        now,
    )
    .unwrap();
    let mut first_value = ObservationValueRecord::new(
        observation.id,
        1,
        ObservationValueData::Number(22.5),
        now,
        now,
    )
    .unwrap();
    first_value.recorded_by = Some(user.id);
    store
        .create_observation(&observation, &first_value, &human_audit)
        .await
        .unwrap();
    let next_time = now + Duration::minutes(1);
    let mut mismatched_revision = ObservationValueRecord::new(
        observation.id,
        2,
        ObservationValueData::Number(22.6),
        next_time,
        next_time,
    )
    .unwrap();
    mismatched_revision.recorded_by = Some(uuid::Uuid::new_v4());
    assert!(matches!(
        store
            .revise_observation_value(
                observation.id,
                observation.meta.revision,
                &mismatched_revision,
                &human_audit,
            )
            .await,
        Err(StoreError::Validation(_))
    ));
    assert_eq!(
        store
            .get_observation(observation.id)
            .await
            .unwrap()
            .current_value_version,
        1
    );
    let mut second_value = ObservationValueRecord::new(
        observation.id,
        2,
        ObservationValueData::Number(22.7),
        next_time,
        next_time,
    )
    .unwrap();
    second_value.recorded_by = Some(user.id);
    let revised = store
        .revise_observation_value(
            observation.id,
            observation.meta.revision,
            &second_value,
            &human_audit,
        )
        .await
        .unwrap();
    assert_eq!(revised.current_value_version, 2);
    assert_eq!(
        store.list_observation_values(observation.id).await.unwrap(),
        vec![first_value, second_value.clone()]
    );

    // Multimodal approvals are atomic: an AI draft cannot overwrite an existing
    // worksheet cell, and a closed experiment can never receive AI-written data.
    let private_image_id = uuid::Uuid::new_v4();
    let private_attachment = Attachment {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: None,
        entity_type: "ai_private_image".to_owned(),
        entity_id: private_image_id,
        file_name: "workspace-contract.png".to_owned(),
        media_type: Some("image/png".to_owned()),
        relative_path: format!("ai-private/{}.png", uuid::Uuid::new_v4()),
        size_bytes: 24,
        sha256: "b".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    let private_image = PrivateAiImage {
        id: private_image_id,
        lab_id: lab.id,
        user_id: user.id,
        conversation_id: None,
        attachment_id: private_attachment.id,
        project_id: None,
        status: PrivateImageStatus::Active,
        last_activity_at: now,
        expires_at: now + Duration::days(30),
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_private_ai_image(&private_attachment, &private_image, &human_audit)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_private_ai_images(&PrivateImageFilter {
                lab_id: lab.id,
                user_id: Some(user.id),
                ..PrivateImageFilter::default()
            })
            .await
            .unwrap(),
        vec![private_image.clone()]
    );

    let overwrite_observation = Observation::new(
        lab.id,
        project.id,
        experiment.id,
        event.id,
        definition.id,
        ObservationSubjectType::Animal,
        female.id,
        now,
    )
    .unwrap();
    let mut overwrite_value = ObservationValueRecord::new(
        overwrite_observation.id,
        1,
        ObservationValueData::Number(99.0),
        now,
        now,
    )
    .unwrap();
    overwrite_value.recorded_by = Some(user.id);
    let overwrite_draft = AiExtractionDraft {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        project_id: project.id,
        experiment_id: experiment.id,
        experiment_event_id: event.id,
        private_image_id: private_image.id,
        attachment_id: private_attachment.id,
        image_sha256: private_attachment.sha256.clone(),
        provider: "contract-provider".to_owned(),
        model: "contract-vision".to_owned(),
        tool_run_id: None,
        status: AiExtractionStatus::PendingApproval,
        items: vec![AiExtractionItem {
            observation: overwrite_observation.clone(),
            value: overwrite_value,
            confidence: 0.99,
            selected: true,
            source_label: Some("body weight".to_owned()),
        }],
        error_code: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_extraction_draft(&overwrite_draft, &human_audit)
        .await
        .unwrap();
    assert!(matches!(
        store
            .apply_ai_extraction_draft(
                overwrite_draft.id,
                overwrite_draft.meta.revision,
                &[0],
                &human_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .get_ai_extraction_draft(overwrite_draft.id)
            .await
            .unwrap()
            .status,
        AiExtractionStatus::PendingApproval
    );
    assert!(matches!(
        store.get_observation(overwrite_observation.id).await,
        Err(StoreError::NotFound { .. })
    ));

    let completed_experiment =
        Experiment::new(lab.id, project.id, "Closed AI write contract", now).unwrap();
    store
        .create_experiment(&completed_experiment, &human_audit)
        .await
        .unwrap();
    let completed_event = ExperimentEvent::new(
        lab.id,
        project.id,
        completed_experiment.id,
        "closed_event",
        "Closed event",
        now,
        now,
    )
    .unwrap();
    store
        .create_experiment_event(&completed_event, &human_audit)
        .await
        .unwrap();
    let mut completed_definition = ObservationDefinition::new(
        lab.id,
        project.id,
        completed_experiment.id,
        "closed_value",
        "Closed value",
        ObservationValueType::Number,
        ObservationPolicy::Versioned,
        now,
    )
    .unwrap();
    completed_definition.unit = Some("g".to_owned());
    completed_definition.validate().unwrap();
    store
        .create_observation_definition(&completed_definition, &human_audit)
        .await
        .unwrap();
    let completed_experiment = store
        .transition_experiment(
            completed_experiment.id,
            ExperimentStatus::Completed,
            completed_experiment.meta.revision,
            now + Duration::minutes(2),
            &human_audit,
        )
        .await
        .unwrap();

    let closed_image_id = uuid::Uuid::new_v4();
    let closed_attachment = Attachment {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: None,
        entity_type: "ai_private_image".to_owned(),
        entity_id: closed_image_id,
        file_name: "closed-contract.png".to_owned(),
        media_type: Some("image/png".to_owned()),
        relative_path: format!("ai-private/{}.png", uuid::Uuid::new_v4()),
        size_bytes: 24,
        sha256: "c".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    let closed_image = PrivateAiImage {
        id: closed_image_id,
        lab_id: lab.id,
        user_id: user.id,
        conversation_id: None,
        attachment_id: closed_attachment.id,
        project_id: None,
        status: PrivateImageStatus::Active,
        last_activity_at: now,
        expires_at: now + Duration::days(30),
        archived_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_private_ai_image(&closed_attachment, &closed_image, &human_audit)
        .await
        .unwrap();
    let closed_observation = Observation::new(
        lab.id,
        project.id,
        completed_experiment.id,
        completed_event.id,
        completed_definition.id,
        ObservationSubjectType::Experiment,
        completed_experiment.id,
        now,
    )
    .unwrap();
    let mut closed_value = ObservationValueRecord::new(
        closed_observation.id,
        1,
        ObservationValueData::Number(1.0),
        now,
        now,
    )
    .unwrap();
    closed_value.recorded_by = Some(user.id);
    let closed_draft = AiExtractionDraft {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        user_id: user.id,
        project_id: project.id,
        experiment_id: completed_experiment.id,
        experiment_event_id: completed_event.id,
        private_image_id: closed_image.id,
        attachment_id: closed_attachment.id,
        image_sha256: closed_attachment.sha256.clone(),
        provider: "contract-provider".to_owned(),
        model: "contract-vision".to_owned(),
        tool_run_id: None,
        status: AiExtractionStatus::PendingApproval,
        items: vec![AiExtractionItem {
            observation: closed_observation.clone(),
            value: closed_value,
            confidence: 0.95,
            selected: true,
            source_label: Some("closed value".to_owned()),
        }],
        error_code: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_ai_extraction_draft(&closed_draft, &human_audit)
        .await
        .unwrap();
    assert!(matches!(
        store
            .apply_ai_extraction_draft(
                closed_draft.id,
                closed_draft.meta.revision,
                &[0],
                &human_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .get_ai_extraction_draft(closed_draft.id)
            .await
            .unwrap()
            .status,
        AiExtractionStatus::PendingApproval
    );
    assert!(matches!(
        store.get_observation(closed_observation.id).await,
        Err(StoreError::NotFound { .. })
    ));

    let retired = store
        .retire_breeding_pair(pair.id, pair.meta.revision, next_time, &human_audit)
        .await
        .unwrap();
    assert_eq!(retired.status, BreedingPairStatus::Retired);
    assert!(
        retired
            .members
            .iter()
            .all(|member| member.left_at == Some(next_time))
    );

    let audited_ids = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.entity_id)
        .collect::<std::collections::HashSet<_>>();
    for id in [
        genotype_definition.id,
        genotyping_record.id,
        line.id,
        colony.id,
        pair.id,
        mating.id,
        litter.id,
        draft.id,
        event.id,
        definition.id,
        observation.id,
        second_value.id,
    ] {
        assert!(audited_ids.contains(&id), "entity {id} must be audited");
    }
}

/// Runs the AI conversation/message behavior contract shared by SQLite and
/// PostgreSQL adapters. The store may contain unrelated fixtures.
pub async fn run_ai_conversation_contract<S>(store: &S)
where
    S: MuriArcStore + AiOperationStore + ?Sized,
{
    store.migrate().await.expect("migration succeeds");
    let now = contract_now();
    let bootstrap = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new(format!("AI Contract {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&lab, &bootstrap).await.unwrap();
    let user = User::new(
        lab.id,
        format!("{}@example.test", uuid::Uuid::new_v4()),
        "AI Contract User",
        now,
    )
    .unwrap();
    let other_user = User::new(
        lab.id,
        format!("{}@example.test", uuid::Uuid::new_v4()),
        "Other AI User",
        now,
    )
    .unwrap();
    store.create_user(&user, &bootstrap).await.unwrap();
    store.create_user(&other_user, &bootstrap).await.unwrap();
    let project = Project::new(lab.id, "AI Contract Project", now).unwrap();
    store.create_project(&project, &bootstrap).await.unwrap();
    let audit = AuditContext {
        actor: Actor {
            actor_type: ActorType::Ai,
            user_id: Some(user.id),
            display_name: "MuriArc AI for AI Contract User".to_owned(),
        },
        source: WriteSource::Ai,
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        reason: Some("AI conversation contract".to_owned()),
    };

    let conversation = AiConversation {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: user.id,
        title: "Project conversation".to_owned(),
        meta: RecordMeta::new(now),
    };
    let lab_conversation = AiConversation {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: None,
        user_id: user.id,
        title: "Lab conversation".to_owned(),
        meta: RecordMeta::new(now - Duration::seconds(1)),
    };
    let hidden_conversation = AiConversation {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: other_user.id,
        title: "Other user's conversation".to_owned(),
        meta: RecordMeta::new(now),
    };
    for value in [&conversation, &lab_conversation, &hidden_conversation] {
        store.create_ai_conversation(value, &audit).await.unwrap();
    }

    let all = store
        .list_ai_conversations(
            &AiConversationFilter {
                lab_id: lab.id,
                user_id: user.id,
                project_id: None,
            },
            0,
            20,
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|value| value.user_id == user.id));
    let project_only = store
        .list_ai_conversations(
            &AiConversationFilter {
                lab_id: lab.id,
                user_id: user.id,
                project_id: Some(project.id),
            },
            0,
            20,
        )
        .await
        .unwrap();
    assert_eq!(project_only, vec![conversation.clone()]);

    let mut autonomy = AiAutonomyGrant {
        id: uuid::Uuid::new_v4(),
        conversation_id: conversation.id,
        lab_id: lab.id,
        project_id: Some(project.id),
        user_id: user.id,
        session_id: Some(uuid::Uuid::new_v4()),
        mode: AiAutonomyMode::Full,
        allowed_categories: vec![
            AiActionCategory::Read,
            AiActionCategory::Artifact,
            AiActionCategory::ReversibleDraft,
        ],
        batch_limit: 100,
        step_up_verified_at: Some(now),
        last_used_at: now,
        expires_at: Some(now + Duration::minutes(30)),
        revoked_at: None,
        meta: RecordMeta::new(now),
    };
    store
        .save_ai_autonomy_grant(&autonomy, None, &audit)
        .await
        .unwrap();
    assert_eq!(
        store.get_ai_autonomy_grant(conversation.id).await.unwrap(),
        Some(autonomy.clone())
    );
    let autonomy_revision = autonomy.meta.revision;
    autonomy.mode = AiAutonomyMode::Auto;
    autonomy.batch_limit = 20;
    autonomy.session_id = None;
    autonomy.step_up_verified_at = None;
    autonomy.expires_at = None;
    autonomy.meta.touch(now + Duration::seconds(1));
    store
        .save_ai_autonomy_grant(&autonomy, Some(autonomy_revision), &audit)
        .await
        .unwrap();
    assert!(matches!(
        store
            .save_ai_autonomy_grant(&autonomy, Some(autonomy_revision), &audit)
            .await,
        Err(StoreError::Conflict(_))
    ));
    let autonomy_audits = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: Some(project.id),
            entity_id: Some(autonomy.id),
        })
        .await
        .unwrap();
    assert_eq!(autonomy_audits.len(), 2);
    assert!(
        autonomy_audits
            .iter()
            .all(|entry| entry.entity_type == EntityType::AiAutonomyGrant)
    );

    let user_message = AiConversationMessage::new(
        conversation.id,
        lab.id,
        Some(project.id),
        user.id,
        1,
        AiConversationMessageRole::User,
        "Which animals need weighing?",
        None,
        now + Duration::milliseconds(1),
    )
    .unwrap();
    let response = serde_json::json!({
        "conversationId": conversation.id,
        "content": "Two animals need weighing.",
        "citations": [],
        "toolRuns": [],
        "drafts": [],
        "trace": {
            "providerId": "contract",
            "model": "contract-model",
            "usage": {
                "provider_calls": 1,
                "tool_calls": 0,
                "input_tokens": 3,
                "output_tokens": 4,
                "total_tokens": 7
            }
        }
    });
    let assistant_message = AiConversationMessage::new(
        conversation.id,
        lab.id,
        Some(project.id),
        user.id,
        2,
        AiConversationMessageRole::Assistant,
        "Two animals need weighing.",
        Some(response.clone()),
        now + Duration::milliseconds(2),
    )
    .unwrap();
    let updated = store
        .append_ai_turn_messages(&user_message, &assistant_message, 0, &audit)
        .await
        .unwrap();
    assert_eq!(updated.meta.revision, conversation.meta.revision + 1);
    assert_eq!(
        store.get_ai_conversation(conversation.id).await.unwrap(),
        updated
    );

    let messages = store
        .list_ai_conversation_messages(conversation.id, 20)
        .await
        .unwrap();
    assert_eq!(
        messages,
        vec![user_message.clone(), assistant_message.clone()]
    );
    assert_eq!(messages[1].response, Some(response));
    assert_eq!(
        store
            .list_ai_conversation_messages(conversation.id, 1)
            .await
            .unwrap(),
        vec![assistant_message.clone()]
    );

    let stale_user = AiConversationMessage::new(
        conversation.id,
        lab.id,
        Some(project.id),
        user.id,
        3,
        AiConversationMessageRole::User,
        "Stale turn",
        None,
        now + Duration::milliseconds(3),
    )
    .unwrap();
    let stale_assistant = AiConversationMessage::new(
        conversation.id,
        lab.id,
        Some(project.id),
        user.id,
        4,
        AiConversationMessageRole::Assistant,
        "Must not persist",
        Some(serde_json::json!({"content": "Must not persist"})),
        now + Duration::milliseconds(4),
    )
    .unwrap();
    assert!(matches!(
        store
            .append_ai_turn_messages(&stale_user, &stale_assistant, 0, &audit)
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .list_ai_conversation_messages(conversation.id, 20)
            .await
            .unwrap()
            .len(),
        2
    );

    for message in [&user_message, &assistant_message] {
        let entries = store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: Some(project.id),
                entity_id: Some(message.id),
            })
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entity_type, EntityType::AiConversationMessage);
        assert_eq!(entries[0].actor.user_id, Some(user.id));
    }
}

async fn run_import_contract(store: &dyn MuriArcStore, now: chrono::DateTime<Utc>) {
    let fixture_audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new(format!("Import Contract Lab {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&lab, &fixture_audit).await.unwrap();
    let user = User::new(
        lab.id,
        format!("{}@import-contract.test", uuid::Uuid::new_v4()),
        "Import Contract User",
        now,
    )
    .unwrap();
    store.create_user(&user, &fixture_audit).await.unwrap();
    let project = Project::new(lab.id, "Import Contract Project", now).unwrap();
    store
        .create_project(&project, &fixture_audit)
        .await
        .unwrap();
    let experiment =
        Experiment::new(lab.id, project.id, "Import Contract Experiment", now).unwrap();
    store
        .create_experiment(&experiment, &fixture_audit)
        .await
        .unwrap();
    let measurement_animal = Animal::new_mouse(
        lab.id,
        format!("IMPORT-MEASUREMENT-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    store
        .create_animal(&measurement_animal, &fixture_audit)
        .await
        .unwrap();
    assign_project_animal(
        store,
        lab.id,
        project.id,
        measurement_animal.id,
        &fixture_audit,
        now,
    )
    .await;
    let measurement_participation =
        Participation::enroll(experiment.id, measurement_animal.id, now);
    store
        .create_participation(&measurement_participation, &fixture_audit)
        .await
        .unwrap();

    let locus = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab.id,
        symbol: format!("ImportLocus-{}", uuid::Uuid::new_v4()),
        description: None,
        meta: RecordMeta::new(now),
    };
    store
        .create_gene_locus(&locus, &fixture_audit)
        .await
        .unwrap();
    let allele = Allele {
        id: uuid::Uuid::new_v4(),
        locus_id: locus.id,
        symbol: format!("ImportAllele-{}", uuid::Uuid::new_v4()),
        description: None,
        is_wild_type: false,
        meta: RecordMeta::new(now),
    };
    store.create_allele(&allele, &fixture_audit).await.unwrap();

    let import_audit = AuditContext {
        actor: Actor::human(user.id, user.display_name.clone()),
        source: WriteSource::Web,
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        reason: Some("confirm store contract import".to_owned()),
    };
    let imported_parent = Animal::new_mouse(
        lab.id,
        format!("IMPORT-PARENT-{}", uuid::Uuid::new_v4()),
        Sex::Female,
        now,
    )
    .unwrap();
    let imported_child = Animal::new_mouse(
        lab.id,
        format!("IMPORT-CHILD-{}", uuid::Uuid::new_v4()),
        Sex::Male,
        now,
    )
    .unwrap();
    let mut parent_registered = AnimalEvent::new(
        lab.id,
        imported_parent.id,
        AnimalEventKind::Registered,
        now,
        now,
    );
    parent_registered.recorded_by = Some(user.id);
    let mut child_registered = AnimalEvent::new(
        lab.id,
        imported_child.id,
        AnimalEventKind::Registered,
        now,
        now,
    );
    child_registered.recorded_by = Some(user.id);
    let genotype = Genotype {
        id: uuid::Uuid::new_v4(),
        animal_id: imported_child.id,
        locus_id: locus.id,
        allele_1_id: Some(allele.id),
        allele_2_id: Some(allele.id),
        assessed_at: Some(now),
        meta: RecordMeta::new(now),
    };
    let pedigree = Pedigree {
        id: uuid::Uuid::new_v4(),
        animal_id: imported_child.id,
        parent_id: imported_parent.id,
        parent_type: ParentType::Mother,
        meta: RecordMeta::new(now),
    };
    let mut measurement = Measurement::draft(
        lab.id,
        project.id,
        measurement_animal.id,
        "import_body_weight",
        "Imported body weight",
        MeasurementValue::Number(21.5),
        now + Duration::seconds(20),
        now,
    )
    .unwrap();
    measurement.experiment_id = Some(experiment.id);
    measurement.unit = Some("g".to_owned());

    let mut plan = ImportPlan::empty(
        lab.id,
        format!("import-contract-{}", uuid::Uuid::new_v4()),
        "1".repeat(64),
    );
    plan.animals = vec![imported_parent.clone(), imported_child.clone()];
    plan.animal_events = vec![parent_registered.clone(), child_registered.clone()];
    plan.genotypes.push(genotype.clone());
    plan.pedigrees.push(pedigree.clone());
    plan.measurements.push(measurement.clone());
    plan.validate().unwrap();

    let audits_before = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    let result = store
        .commit_import(&plan, ImportCommitOptions::default(), &import_audit)
        .await
        .unwrap();
    assert!(!result.replayed);
    assert_eq!(result.commit_id, plan.commit_id);
    assert_eq!(result.preview_hash, plan.preview_hash);
    assert_eq!(result.counts, plan.entity_counts());
    assert_eq!(
        store.get_animal(imported_parent.id).await.unwrap(),
        imported_parent
    );
    assert_eq!(
        store.get_animal(imported_child.id).await.unwrap(),
        imported_child
    );
    assert_eq!(
        store.list_animal_events(imported_parent.id).await.unwrap(),
        vec![parent_registered.clone()]
    );
    assert_eq!(
        store.list_animal_events(imported_child.id).await.unwrap(),
        vec![child_registered.clone()]
    );
    assert_eq!(store.get_genotype(genotype.id).await.unwrap(), genotype);
    assert_eq!(store.get_pedigree(pedigree.id).await.unwrap(), pedigree);
    assert_eq!(
        store.get_measurement(measurement.id).await.unwrap(),
        measurement
    );

    let audits_after = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    let expected_imported_entities = [
        (EntityType::Animal, imported_parent.id),
        (EntityType::Animal, imported_child.id),
        (EntityType::AnimalEvent, parent_registered.id),
        (EntityType::AnimalEvent, child_registered.id),
        (EntityType::Genotype, genotype.id),
        (EntityType::Pedigree, pedigree.id),
        (EntityType::Measurement, measurement.id),
    ];
    assert_eq!(
        audits_after.len() - audits_before.len(),
        expected_imported_entities.len() + 2,
        "imported entities plus the derived Measurement event/projection must be audited"
    );
    for (entity_type, entity_id) in expected_imported_entities {
        assert_eq!(
            audits_after
                .iter()
                .filter(|entry| {
                    entry.entity_type == entity_type
                        && entry.entity_id == entity_id
                        && entry.action == AuditAction::Import
                })
                .count(),
            1,
            "imported {entity_type:?} {entity_id} must have exactly one Import audit"
        );
    }

    assert!(store.list_animal_events(measurement_animal.id).await.unwrap().iter().any(|event| {
        matches!(event.kind, AnimalEventKind::MeasurementRecorded { measurement_id } if measurement_id == measurement.id)
    }));
    let import_provenance = store
        .list_provenance(&ProvenanceFilter {
            lab_id: lab.id,
            entity_type: Some(EntityType::Measurement),
            entity_id: Some(measurement.id),
            ..ProvenanceFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(import_provenance.len(), 1);
    assert_eq!(import_provenance[0].source, ProvenanceSource::Import);
    assert_eq!(import_provenance[0].import_commit_id, Some(plan.commit_id));

    let animal_count_after = store
        .list_animals(&AnimalFilter {
            lab_id: lab.id,
            ..AnimalFilter::default()
        })
        .await
        .unwrap()
        .len();
    let replay = store
        .commit_import(&plan, ImportCommitOptions::default(), &import_audit)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.commit_id, result.commit_id);
    assert_eq!(replay.counts, result.counts);
    assert_eq!(
        store
            .list_animals(&AnimalFilter {
                lab_id: lab.id,
                ..AnimalFilter::default()
            })
            .await
            .unwrap()
            .len(),
        animal_count_after
    );
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        audits_after.len(),
        "idempotent replay must not repeat audits"
    );

    let mut same_key_different_hash = plan.clone();
    same_key_different_hash.preview_hash = "2".repeat(64);
    assert!(matches!(
        store
            .commit_import(
                &same_key_different_hash,
                ImportCommitOptions::default(),
                &import_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    let mut different_key_same_hash = plan.clone();
    different_key_same_hash.commit_id = uuid::Uuid::new_v4();
    different_key_same_hash.idempotency_key =
        format!("different-import-key-{}", uuid::Uuid::new_v4());
    assert!(matches!(
        store
            .commit_import(
                &different_key_same_hash,
                ImportCommitOptions::default(),
                &import_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        audits_after.len(),
        "idempotency conflicts must not write audits"
    );

    let rollback_first = Animal::new_mouse(
        lab.id,
        format!("ROLLBACK-FIRST-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    let rollback_conflict = Animal::new_mouse(
        lab.id,
        imported_parent.display_id.clone(),
        Sex::Unknown,
        now,
    )
    .unwrap();
    let mut rollback_first_event = AnimalEvent::new(
        lab.id,
        rollback_first.id,
        AnimalEventKind::Registered,
        now,
        now,
    );
    rollback_first_event.recorded_by = Some(user.id);
    let mut rollback_conflict_event = AnimalEvent::new(
        lab.id,
        rollback_conflict.id,
        AnimalEventKind::Registered,
        now,
        now,
    );
    rollback_conflict_event.recorded_by = Some(user.id);
    let mut rollback_plan = ImportPlan::empty(
        lab.id,
        format!("rollback-import-{}", uuid::Uuid::new_v4()),
        "3".repeat(64),
    );
    rollback_plan.animals = vec![rollback_first.clone(), rollback_conflict.clone()];
    rollback_plan.animal_events = vec![rollback_first_event, rollback_conflict_event];
    rollback_plan.validate().unwrap();
    let rollback_audit_count = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap()
        .len();
    assert!(
        store
            .commit_import(
                &rollback_plan,
                ImportCommitOptions::default(),
                &import_audit,
            )
            .await
            .is_err()
    );
    assert!(matches!(
        store.get_animal(rollback_first.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_animal(rollback_conflict.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(
        store
            .list_animals(&AnimalFilter {
                lab_id: lab.id,
                ..AnimalFilter::default()
            })
            .await
            .unwrap()
            .len(),
        animal_count_after,
        "a conflict on the second entity must roll back the first entity"
    );
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        rollback_audit_count,
        "a failed import must roll back all audit entries"
    );

    let cancelled_animal = Animal::new_mouse(
        lab.id,
        format!("CANCELLED-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    let mut cancelled_event = AnimalEvent::new(
        lab.id,
        cancelled_animal.id,
        AnimalEventKind::Registered,
        now,
        now,
    );
    cancelled_event.recorded_by = Some(user.id);
    let mut cancelled_plan = ImportPlan::empty(
        lab.id,
        format!("cancelled-import-{}", uuid::Uuid::new_v4()),
        "4".repeat(64),
    );
    cancelled_plan.animals.push(cancelled_animal.clone());
    cancelled_plan.animal_events.push(cancelled_event);
    let cancelled_audit_count = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap()
        .len();
    assert!(matches!(
        store
            .commit_import(
                &cancelled_plan,
                ImportCommitOptions {
                    cancellation_requested: true,
                    job_id: None,
                },
                &import_audit,
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.get_animal(cancelled_animal.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        cancelled_audit_count,
        "cancelled imports must write neither entities nor audits"
    );
}

async fn run_relationship_contract(store: &dyn MuriArcStore, now: chrono::DateTime<Utc>) {
    let audit = AuditContext::system(WriteSource::Migration);
    let lab_a = Lab::new(format!("Relationship Lab A {}", uuid::Uuid::new_v4()), now).unwrap();
    let lab_b = Lab::new(format!("Relationship Lab B {}", uuid::Uuid::new_v4()), now).unwrap();
    store.create_lab(&lab_a, &audit).await.unwrap();
    store.create_lab(&lab_b, &audit).await.unwrap();
    let project_a = Project::new(lab_a.id, "Relationship Project A", now).unwrap();
    let project_a_other = Project::new(lab_a.id, "Relationship Project A Other", now).unwrap();
    let project_b = Project::new(lab_b.id, "Relationship Project B", now).unwrap();
    store.create_project(&project_a, &audit).await.unwrap();
    store
        .create_project(&project_a_other, &audit)
        .await
        .unwrap();
    store.create_project(&project_b, &audit).await.unwrap();
    let draft_template_a = ExperimentTemplateVersion::draft(
        lab_a.id,
        format!("draft-a-{}", uuid::Uuid::new_v4()),
        1,
        "Draft A",
        now,
    )
    .unwrap();
    store
        .create_template_version(&draft_template_a, &audit)
        .await
        .unwrap();
    let mut published_template_b = ExperimentTemplateVersion::draft(
        lab_b.id,
        format!("published-b-{}", uuid::Uuid::new_v4()),
        1,
        "Published B",
        now,
    )
    .unwrap();
    published_template_b
        .publish(uuid::Uuid::new_v4(), now)
        .unwrap();
    store
        .create_template_version(&published_template_b, &audit)
        .await
        .unwrap();
    let cage_a = Cage::new(
        lab_a.id,
        "A",
        format!("REL-CAGE-A-{}", uuid::Uuid::new_v4()),
        now,
    )
    .unwrap();
    let cage_b = Cage::new(
        lab_b.id,
        "B",
        format!("REL-CAGE-B-{}", uuid::Uuid::new_v4()),
        now,
    )
    .unwrap();
    store.create_cage(&cage_a, &audit).await.unwrap();
    store.create_cage(&cage_b, &audit).await.unwrap();
    let animal_a = Animal::new_mouse(
        lab_a.id,
        format!("REL-A-{}", uuid::Uuid::new_v4()),
        Sex::Female,
        now,
    )
    .unwrap();
    let animal_a_peer = Animal::new_mouse(
        lab_a.id,
        format!("REL-A-PEER-{}", uuid::Uuid::new_v4()),
        Sex::Male,
        now,
    )
    .unwrap();
    let animal_a_unenrolled = Animal::new_mouse(
        lab_a.id,
        format!("REL-A-UNENROLLED-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    let animal_b = Animal::new_mouse(
        lab_b.id,
        format!("REL-B-{}", uuid::Uuid::new_v4()),
        Sex::Male,
        now,
    )
    .unwrap();
    store.create_animal(&animal_a, &audit).await.unwrap();
    store.create_animal(&animal_a_peer, &audit).await.unwrap();
    store
        .create_animal(&animal_a_unenrolled, &audit)
        .await
        .unwrap();
    store.create_animal(&animal_b, &audit).await.unwrap();
    let experiment_a =
        Experiment::new(lab_a.id, project_a.id, "Relationship Experiment A", now).unwrap();
    let experiment_a_other = Experiment::new(
        lab_a.id,
        project_a.id,
        "Relationship Experiment A Other",
        now,
    )
    .unwrap();
    let experiment_a_other_project = Experiment::new(
        lab_a.id,
        project_a_other.id,
        "Relationship Experiment A Other Project",
        now,
    )
    .unwrap();
    let experiment_b =
        Experiment::new(lab_b.id, project_b.id, "Relationship Experiment B", now).unwrap();
    for experiment in [
        &experiment_a,
        &experiment_a_other,
        &experiment_a_other_project,
        &experiment_b,
    ] {
        store.create_experiment(experiment, &audit).await.unwrap();
    }
    for animal_id in [animal_a.id, animal_a_peer.id] {
        assign_project_animal(store, lab_a.id, project_a.id, animal_id, &audit, now).await;
    }
    for animal_id in [animal_a.id, animal_a_peer.id] {
        let participation = Participation::enroll(experiment_a.id, animal_id, now);
        store
            .create_participation(&participation, &audit)
            .await
            .unwrap();
    }
    let cohort_a_other = Cohort {
        id: uuid::Uuid::new_v4(),
        experiment_id: experiment_a_other.id,
        name: format!("Other Cohort {}", uuid::Uuid::new_v4()),
        description: None,
        meta: RecordMeta::new(now),
    };
    store.create_cohort(&cohort_a_other, &audit).await.unwrap();
    let participation_other = Participation::enroll(experiment_a_other.id, animal_a.id, now);
    store
        .create_participation(&participation_other, &audit)
        .await
        .unwrap();

    let procedure_other_experiment = Procedure {
        id: uuid::Uuid::new_v4(),
        experiment_id: experiment_a_other.id,
        animal_id: Some(animal_a.id),
        name: "Procedure in other experiment".to_owned(),
        scheduled_at: Some(now),
        performed_at: None,
        status: ProcedureStatus::Planned,
        details: serde_json::json!({}),
        meta: RecordMeta::new(now),
    };
    let procedure_other_animal = Procedure {
        id: uuid::Uuid::new_v4(),
        experiment_id: experiment_a.id,
        animal_id: Some(animal_a_peer.id),
        name: "Procedure for other animal".to_owned(),
        scheduled_at: Some(now),
        performed_at: None,
        status: ProcedureStatus::Planned,
        details: serde_json::json!({}),
        meta: RecordMeta::new(now),
    };
    store
        .create_procedure(&procedure_other_experiment, &audit)
        .await
        .unwrap();
    store
        .create_procedure(&procedure_other_animal, &audit)
        .await
        .unwrap();

    let mut collection_event = AnimalEvent::new(
        lab_a.id,
        animal_a.id,
        AnimalEventKind::Note {
            body: "collection source event".to_owned(),
        },
        now,
        now,
    );
    collection_event.project_id = Some(project_a.id);
    store
        .append_animal_event(&collection_event, &audit)
        .await
        .unwrap();

    let locus_a = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab_a.id,
        symbol: format!("RelLocusA-{}", uuid::Uuid::new_v4()),
        description: None,
        meta: RecordMeta::new(now),
    };
    let locus_a_other = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab_a.id,
        symbol: format!("RelLocusAOther-{}", uuid::Uuid::new_v4()),
        description: None,
        meta: RecordMeta::new(now),
    };
    let locus_b = GeneLocus {
        id: uuid::Uuid::new_v4(),
        lab_id: lab_b.id,
        symbol: format!("RelLocusB-{}", uuid::Uuid::new_v4()),
        description: None,
        meta: RecordMeta::new(now),
    };
    for locus in [&locus_a, &locus_a_other, &locus_b] {
        store.create_gene_locus(locus, &audit).await.unwrap();
    }
    let allele_a_other = Allele {
        id: uuid::Uuid::new_v4(),
        locus_id: locus_a_other.id,
        symbol: format!("RelAlleleOther-{}", uuid::Uuid::new_v4()),
        description: None,
        is_wild_type: false,
        meta: RecordMeta::new(now),
    };
    store.create_allele(&allele_a_other, &audit).await.unwrap();

    let audits_a_before = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab_a.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap()
        .len();
    let audits_b_before = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab_b.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap()
        .len();

    let mut wrong_cage_animal = Animal::new_mouse(
        lab_a.id,
        format!("WRONG-CAGE-{}", uuid::Uuid::new_v4()),
        Sex::Unknown,
        now,
    )
    .unwrap();
    wrong_cage_animal.current_cage_id = Some(cage_b.id);
    assert!(
        store
            .create_animal(&wrong_cage_animal, &audit)
            .await
            .is_err()
    );

    let wrong_project_experiment =
        Experiment::new(lab_a.id, project_b.id, "Cross-lab project", now).unwrap();
    assert!(
        store
            .create_experiment(&wrong_project_experiment, &audit)
            .await
            .is_err()
    );
    let mut missing_template_experiment =
        Experiment::new(lab_a.id, project_a.id, "Missing template", now).unwrap();
    missing_template_experiment.template_version_id = Some(uuid::Uuid::new_v4());
    let mut draft_template_experiment =
        Experiment::new(lab_a.id, project_a.id, "Draft template", now).unwrap();
    draft_template_experiment.template_version_id = Some(draft_template_a.id);
    let mut cross_lab_template_experiment =
        Experiment::new(lab_a.id, project_a.id, "Cross-lab template", now).unwrap();
    cross_lab_template_experiment.template_version_id = Some(published_template_b.id);
    for experiment in [
        &missing_template_experiment,
        &draft_template_experiment,
        &cross_lab_template_experiment,
    ] {
        assert!(store.create_experiment(experiment, &audit).await.is_err());
    }

    let missing_experiment_id = uuid::Uuid::new_v4();
    let missing_experiment_cohort = Cohort {
        id: uuid::Uuid::new_v4(),
        experiment_id: missing_experiment_id,
        name: "Missing experiment cohort".to_owned(),
        description: None,
        meta: RecordMeta::new(now),
    };
    assert!(
        store
            .create_cohort(&missing_experiment_cohort, &audit)
            .await
            .is_err()
    );

    let cross_lab_participation = Participation::enroll(experiment_a.id, animal_b.id, now);
    assert!(
        store
            .create_participation(&cross_lab_participation, &audit)
            .await
            .is_err()
    );
    let mut wrong_cohort_participation = Participation::enroll(experiment_a.id, animal_a.id, now);
    wrong_cohort_participation.cohort_id = Some(cohort_a_other.id);
    assert!(
        store
            .create_participation(&wrong_cohort_participation, &audit)
            .await
            .is_err()
    );

    let cross_lab_procedure = Procedure {
        id: uuid::Uuid::new_v4(),
        experiment_id: experiment_a.id,
        animal_id: Some(animal_b.id),
        name: "Cross-lab animal procedure".to_owned(),
        scheduled_at: Some(now),
        performed_at: None,
        status: ProcedureStatus::Planned,
        details: serde_json::json!({}),
        meta: RecordMeta::new(now),
    };
    assert!(
        store
            .create_procedure(&cross_lab_procedure, &audit)
            .await
            .is_err()
    );

    let mut cross_lab_animal_measurement = Measurement::draft(
        lab_a.id,
        project_a.id,
        animal_b.id,
        "cross_lab_animal",
        "Cross-lab animal",
        MeasurementValue::Number(1.0),
        now + Duration::seconds(101),
        now,
    )
    .unwrap();
    cross_lab_animal_measurement.experiment_id = Some(experiment_a.id);
    let mut wrong_experiment_measurement = Measurement::draft(
        lab_a.id,
        project_a.id,
        animal_a.id,
        "wrong_experiment",
        "Wrong experiment",
        MeasurementValue::Number(1.0),
        now + Duration::seconds(102),
        now,
    )
    .unwrap();
    wrong_experiment_measurement.experiment_id = Some(experiment_a_other_project.id);
    let mut unenrolled_animal_measurement = Measurement::draft(
        lab_a.id,
        project_a.id,
        animal_a_unenrolled.id,
        "unenrolled_animal",
        "Unenrolled animal",
        MeasurementValue::Number(1.0),
        now + Duration::seconds(105),
        now,
    )
    .unwrap();
    unenrolled_animal_measurement.experiment_id = Some(experiment_a.id);
    let mut wrong_procedure_experiment_measurement = Measurement::draft(
        lab_a.id,
        project_a.id,
        animal_a.id,
        "wrong_procedure_experiment",
        "Wrong procedure experiment",
        MeasurementValue::Number(1.0),
        now + Duration::seconds(103),
        now,
    )
    .unwrap();
    wrong_procedure_experiment_measurement.experiment_id = Some(experiment_a.id);
    wrong_procedure_experiment_measurement.procedure_id = Some(procedure_other_experiment.id);
    let mut wrong_procedure_animal_measurement = Measurement::draft(
        lab_a.id,
        project_a.id,
        animal_a.id,
        "wrong_procedure_animal",
        "Wrong procedure animal",
        MeasurementValue::Number(1.0),
        now + Duration::seconds(104),
        now,
    )
    .unwrap();
    wrong_procedure_animal_measurement.experiment_id = Some(experiment_a.id);
    wrong_procedure_animal_measurement.procedure_id = Some(procedure_other_animal.id);
    for measurement in [
        &cross_lab_animal_measurement,
        &wrong_experiment_measurement,
        &unenrolled_animal_measurement,
        &wrong_procedure_experiment_measurement,
        &wrong_procedure_animal_measurement,
    ] {
        assert!(store.create_measurement(measurement, &audit).await.is_err());
    }

    let mut cross_lab_animal_sample =
        Sample::new(lab_a.id, project_a.id, animal_b.id, "blood", now, now).unwrap();
    cross_lab_animal_sample.experiment_id = Some(experiment_a.id);
    let mut wrong_experiment_sample =
        Sample::new(lab_a.id, project_a.id, animal_a.id, "blood", now, now).unwrap();
    wrong_experiment_sample.experiment_id = Some(experiment_a_other_project.id);
    let mut unenrolled_animal_sample = Sample::new(
        lab_a.id,
        project_a.id,
        animal_a_unenrolled.id,
        "blood",
        now,
        now,
    )
    .unwrap();
    unenrolled_animal_sample.experiment_id = Some(experiment_a.id);
    let mut wrong_collection_event_sample =
        Sample::new(lab_a.id, project_a.id, animal_a_peer.id, "blood", now, now).unwrap();
    wrong_collection_event_sample.experiment_id = Some(experiment_a.id);
    wrong_collection_event_sample.collection_event_id = Some(collection_event.id);
    for sample in [
        &cross_lab_animal_sample,
        &wrong_experiment_sample,
        &unenrolled_animal_sample,
        &wrong_collection_event_sample,
    ] {
        assert!(store.create_sample(sample, &audit).await.is_err());
    }

    let cross_lab_locus_genotype = Genotype {
        id: uuid::Uuid::new_v4(),
        animal_id: animal_a.id,
        locus_id: locus_b.id,
        allele_1_id: None,
        allele_2_id: None,
        assessed_at: Some(now),
        meta: RecordMeta::new(now),
    };
    let wrong_locus_allele_genotype = Genotype {
        id: uuid::Uuid::new_v4(),
        animal_id: animal_a.id,
        locus_id: locus_a.id,
        allele_1_id: Some(allele_a_other.id),
        allele_2_id: None,
        assessed_at: Some(now),
        meta: RecordMeta::new(now),
    };
    assert!(
        store
            .create_genotype(&cross_lab_locus_genotype, Some(project_a.id), &audit)
            .await
            .is_err()
    );
    assert!(
        store
            .create_genotype(&wrong_locus_allele_genotype, Some(project_a.id), &audit)
            .await
            .is_err()
    );

    let cross_lab_pedigree = Pedigree {
        id: uuid::Uuid::new_v4(),
        animal_id: animal_a.id,
        parent_id: animal_b.id,
        parent_type: ParentType::Father,
        meta: RecordMeta::new(now),
    };
    let self_pedigree = Pedigree {
        id: uuid::Uuid::new_v4(),
        animal_id: animal_a.id,
        parent_id: animal_a.id,
        parent_type: ParentType::Unknown,
        meta: RecordMeta::new(now),
    };
    assert!(
        store
            .create_pedigree(&cross_lab_pedigree, &audit)
            .await
            .is_err()
    );
    assert!(store.create_pedigree(&self_pedigree, &audit).await.is_err());

    let wrong_project_attachment = Attachment {
        id: uuid::Uuid::new_v4(),
        lab_id: lab_a.id,
        project_id: Some(project_b.id),
        entity_type: "animal".to_owned(),
        entity_id: animal_a.id,
        file_name: "cross-lab-project.txt".to_owned(),
        media_type: Some("text/plain".to_owned()),
        relative_path: "contract/cross-lab-project.txt".to_owned(),
        size_bytes: 1,
        sha256: "a".repeat(64),
        version: 1,
        meta: RecordMeta::new(now),
    };
    assert!(
        store
            .create_attachment(&wrong_project_attachment, &audit)
            .await
            .is_err()
    );

    assert!(matches!(
        store.get_animal(wrong_cage_animal.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_experiment(wrong_project_experiment.id).await,
        Err(StoreError::NotFound { .. })
    ));
    for experiment in [
        &missing_template_experiment,
        &draft_template_experiment,
        &cross_lab_template_experiment,
    ] {
        assert!(matches!(
            store.get_experiment(experiment.id).await,
            Err(StoreError::NotFound { .. })
        ));
    }
    assert!(
        store
            .list_cohorts(missing_experiment_id)
            .await
            .unwrap()
            .iter()
            .all(|cohort| cohort.id != missing_experiment_cohort.id)
    );
    assert!(
        store
            .list_participations(&ParticipationFilter {
                project_id: project_a.id,
                experiment_id: Some(experiment_a.id),
                animal_id: None,
                cohort_id: None,
            })
            .await
            .unwrap()
            .iter()
            .all(|participation| {
                participation.id != cross_lab_participation.id
                    && participation.id != wrong_cohort_participation.id
            })
    );
    assert!(
        store
            .list_procedures(experiment_a.id, None)
            .await
            .unwrap()
            .iter()
            .all(|procedure| procedure.id != cross_lab_procedure.id)
    );
    for measurement in [
        &cross_lab_animal_measurement,
        &wrong_experiment_measurement,
        &unenrolled_animal_measurement,
        &wrong_procedure_experiment_measurement,
        &wrong_procedure_animal_measurement,
    ] {
        assert!(matches!(
            store.get_measurement(measurement.id).await,
            Err(StoreError::NotFound { .. })
        ));
    }
    for sample in [
        &cross_lab_animal_sample,
        &wrong_experiment_sample,
        &unenrolled_animal_sample,
        &wrong_collection_event_sample,
    ] {
        assert!(matches!(
            store.get_sample(sample.id).await,
            Err(StoreError::NotFound { .. })
        ));
    }
    for genotype in [&cross_lab_locus_genotype, &wrong_locus_allele_genotype] {
        assert!(matches!(
            store.get_genotype(genotype.id).await,
            Err(StoreError::NotFound { .. })
        ));
    }
    for pedigree in [&cross_lab_pedigree, &self_pedigree] {
        assert!(matches!(
            store.get_pedigree(pedigree.id).await,
            Err(StoreError::NotFound { .. })
        ));
    }
    assert!(
        store
            .list_attachments(lab_a.id, "animal", animal_a.id)
            .await
            .unwrap()
            .iter()
            .all(|attachment| attachment.id != wrong_project_attachment.id)
    );
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab_a.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        audits_a_before,
        "rejected relationships must not write Lab A audits"
    );
    assert_eq!(
        store
            .list_audit_entries(&AuditFilter {
                lab_id: lab_b.id,
                project_id: None,
                entity_id: None,
            })
            .await
            .unwrap()
            .len(),
        audits_b_before,
        "rejected relationships must not write Lab B audits"
    );
}
