use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use muriarc_application::{
    AnimalDetailQuery, AnimalPedigreeDirection, ApplicationError, CreatePedigreeCommand,
    CreateSampleCommand, MAX_ANIMAL_DETAIL_PEDIGREE_EDGES, MAX_ANIMAL_DETAIL_PROJECTS,
    animal_project_refs, create_pedigree, create_sample, load_animal_detail,
};
use muriarc_core::{
    Animal, AnimalProjectRef, AuditContext, Lab, MuriArcStore, ParentType, Project,
    ProjectAnimalAssignment, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

struct Fixture {
    store: SqliteStore,
    audit: AuditContext,
    now: DateTime<Utc>,
    lab: Lab,
}

impl Fixture {
    async fn new() -> Self {
        let store = SqliteStore::in_memory().await.unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        let audit = AuditContext::system(WriteSource::Api);
        let lab = Lab::new("Animal detail lab", now).unwrap();
        store.create_lab(&lab, &audit).await.unwrap();
        Self {
            store,
            audit,
            now,
            lab,
        }
    }

    async fn project(&self, name: &str) -> Project {
        let project = Project::new(self.lab.id, name, self.now).unwrap();
        self.store
            .create_project(&project, &self.audit)
            .await
            .unwrap();
        project
    }

    async fn animal(&self, display_id: &str, sex: Sex, projects: &[&Project]) -> Animal {
        let animal = Animal::new_mouse(self.lab.id, display_id, sex, self.now).unwrap();
        self.store
            .create_animal(&animal, &self.audit)
            .await
            .unwrap();
        // Each assignment batch targets exactly one project.
        for project in projects {
            let assignment = ProjectAnimalAssignment::new(
                self.lab.id,
                project.id,
                animal.id,
                None,
                None,
                self.now,
            );
            self.store
                .assign_animals_to_project(&[assignment], &self.audit)
                .await
                .unwrap();
        }
        animal
    }

    async fn sample(&self, project: &Project, animal: &Animal, offset_seconds: i64) {
        create_sample(
            &self.store,
            CreateSampleCommand {
                lab_id: self.lab.id,
                project_id: project.id,
                experiment_id: None,
                animal_id: animal.id,
                sample_type: format!("tissue-{offset_seconds}"),
                quantity: None,
                unit: None,
                location: None,
                collected_at: self.now + Duration::seconds(offset_seconds),
                now: self.now,
            },
            &self.audit,
        )
        .await
        .unwrap();
    }

    async fn pedigree(&self, child: &Animal, parent: &Animal, parent_type: ParentType) {
        create_pedigree(
            &self.store,
            CreatePedigreeCommand {
                animal_id: child.id,
                parent_id: parent.id,
                parent_type,
                now: self.now,
            },
            &self.audit,
        )
        .await
        .unwrap();
    }

    fn query(&self, animal: &Animal, projects: Vec<AnimalProjectRef>) -> AnimalDetailQuery {
        let all = projects
            .iter()
            .map(|project| project.id)
            .collect::<BTreeSet<_>>();
        AnimalDetailQuery {
            lab_id: self.lab.id,
            animal_id: animal.id,
            project_id: None,
            projects,
            measurement_project_ids: all.clone(),
            sample_project_ids: all,
            include_attachments: true,
            include_audit: true,
            limit: 500,
        }
    }
}

fn project_ref(project: &Project) -> AnimalProjectRef {
    AnimalProjectRef {
        id: project.id,
        name: project.name.clone(),
    }
}

#[tokio::test]
async fn detail_projects_come_from_current_assignments() {
    let fixture = Fixture::new().await;
    let first = fixture.project("First").await;
    let second = fixture.project("Second").await;
    let animal = fixture
        .animal("M-001", Sex::Female, &[&first, &second])
        .await;
    let unassigned = fixture.animal("M-002", Sex::Male, &[]).await;

    let projects = animal_project_refs(&fixture.store, fixture.lab.id, &animal)
        .await
        .unwrap()
        .into_iter()
        .map(|project| project.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(projects, BTreeSet::from([first.id, second.id]));
    assert!(
        animal_project_refs(&fixture.store, fixture.lab.id, &unassigned)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn detail_respects_authorized_sections_and_orders_newest_first() {
    let fixture = Fixture::new().await;
    let readable = fixture.project("Readable").await;
    let restricted = fixture.project("Restricted").await;
    let animal = fixture
        .animal("M-001", Sex::Female, &[&readable, &restricted])
        .await;
    let mother = fixture.animal("F-001", Sex::Female, &[&readable]).await;
    fixture.pedigree(&animal, &mother, ParentType::Mother).await;
    fixture.sample(&readable, &animal, 1).await;
    fixture.sample(&readable, &animal, 3).await;
    fixture.sample(&restricted, &animal, 2).await;

    let mut query = fixture.query(
        &animal,
        vec![project_ref(&readable), project_ref(&restricted)],
    );
    let full = load_animal_detail(&fixture.store, &query).await.unwrap();
    let collected = full
        .samples
        .iter()
        .map(|sample| sample.collected_at)
        .collect::<Vec<_>>();
    assert_eq!(collected.len(), 3);
    assert!(collected.windows(2).all(|pair| pair[0] >= pair[1]));
    assert!(!full.audits.is_empty(), "animal creation must be audited");
    assert_eq!(full.pedigree.len(), 1);
    assert_eq!(full.pedigree[0].direction, AnimalPedigreeDirection::Parent);
    assert_eq!(full.pedigree[0].related_animal.id, mother.id);

    query.sample_project_ids = BTreeSet::from([readable.id]);
    query.include_audit = false;
    let restricted_view = load_animal_detail(&fixture.store, &query).await.unwrap();
    assert_eq!(restricted_view.samples.len(), 2);
    assert!(
        restricted_view
            .samples
            .iter()
            .all(|sample| sample.project_id == readable.id)
    );
    assert!(restricted_view.audits.is_empty());
    assert!(restricted_view.provenance.is_empty());

    // A relative outside the selected project context is hidden, not an error.
    query.project_id = Some(restricted.id);
    query.projects = vec![project_ref(&restricted)];
    let scoped = load_animal_detail(&fixture.store, &query).await.unwrap();
    assert!(scoped.pedigree.is_empty());
}

#[tokio::test]
async fn detail_rejects_unbounded_project_scope() {
    let fixture = Fixture::new().await;
    let animal = fixture.animal("M-001", Sex::Female, &[]).await;
    let projects = (0..=MAX_ANIMAL_DETAIL_PROJECTS)
        .map(|index| AnimalProjectRef {
            id: Uuid::new_v4(),
            name: format!("Project {index}"),
        })
        .collect();

    let error = load_animal_detail(&fixture.store, &fixture.query(&animal, projects))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ApplicationError::Validation(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn detail_rejects_pedigree_beyond_edge_limit() {
    let fixture = Fixture::new().await;
    let mother = fixture.animal("F-001", Sex::Female, &[]).await;
    for index in 0..=MAX_ANIMAL_DETAIL_PEDIGREE_EDGES {
        let pup = fixture
            .animal(&format!("P-{index:04}"), Sex::Unknown, &[])
            .await;
        fixture.pedigree(&pup, &mother, ParentType::Mother).await;
    }

    let error = load_animal_detail(&fixture.store, &fixture.query(&mother, Vec::new()))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ApplicationError::Validation(_)),
        "{error:?}"
    );
}
