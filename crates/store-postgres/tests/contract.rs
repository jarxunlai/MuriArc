use chrono::Utc;
use muriarc_core::{
    ActorType, AuditAction, AuditContext, AuditFilter, EntityType, Lab, MuriArcStore, WriteSource,
    store_contract::{
        run_ai_conversation_contract, run_research_extensions_contract, run_store_contract,
    },
};
use muriarc_store_postgres::PostgresStore;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn postgres_store_obeys_shared_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!("skipping PostgreSQL contract: MURIARC_TEST_DATABASE_URL is not set");
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_store_contract(&store).await;
}

#[tokio::test]
async fn postgres_store_obeys_ai_conversation_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL AI conversation contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_ai_conversation_contract(&store).await;
}

#[tokio::test]
async fn postgres_store_obeys_research_extensions_contract_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL research extensions contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    run_research_extensions_contract(&store).await;
}

#[tokio::test]
async fn postgres_audit_reader_accepts_security_lifecycle_entries_when_configured() {
    let Ok(database_url) = std::env::var("MURIARC_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping PostgreSQL audit compatibility contract: MURIARC_TEST_DATABASE_URL is not set"
        );
        return;
    };
    let store = PostgresStore::connect(&database_url).await.unwrap();
    store.migrate().await.unwrap();
    let audit = AuditContext::system(WriteSource::Migration);
    let now = Utc::now();
    let lab = Lab::new("Audit Compatibility Contract", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();

    let fixtures = [
        (
            "user_credential",
            "create",
            EntityType::UserCredential,
            AuditAction::Create,
        ),
        (
            "auth_session",
            "create",
            EntityType::AuthSession,
            AuditAction::Create,
        ),
        (
            "external_token",
            "revoke",
            EntityType::ExternalToken,
            AuditAction::Revoke,
        ),
    ];
    for (entity_type, action, _, _) in fixtures {
        sqlx::query(
            "INSERT INTO audit_entries (id, lab_id, project_id, entity_type, entity_id, action, actor_type, actor_user_id, actor_display_name, source, request_id, reason, before_json, after_json, occurred_at) VALUES ($1, $2, NULL, $3, $4, $5, 'system', NULL, 'Audit compatibility contract', 'migration', NULL, NULL, NULL, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(lab.id)
        .bind(entity_type)
        .bind(Uuid::new_v4())
        .bind(action)
        .bind(json!({"redacted": true}))
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
    }

    let entries = store
        .list_audit_entries(&AuditFilter {
            lab_id: lab.id,
            project_id: None,
            entity_id: None,
        })
        .await
        .unwrap();
    for (_, _, entity_type, action) in fixtures {
        let entry = entries
            .iter()
            .find(|entry| entry.entity_type == entity_type && entry.action == action)
            .unwrap_or_else(|| panic!("missing {entity_type:?}/{action:?} audit entry"));
        assert_eq!(entry.actor.actor_type, ActorType::System);
        assert_eq!(entry.source, WriteSource::Migration);
    }
}
