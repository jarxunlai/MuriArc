use muriarc_core::store_contract::{
    run_ai_conversation_contract, run_ai_model_profile_contract, run_research_extensions_contract,
    run_store_contract,
};
use muriarc_store_sqlite::SqliteStore;

#[tokio::test]
async fn sqlite_store_obeys_shared_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_store_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_ai_model_profile_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_model_profile_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_ai_conversation_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_ai_conversation_contract(&store).await;
}

#[tokio::test]
async fn sqlite_store_obeys_research_extensions_contract() {
    let store = SqliteStore::in_memory().await.unwrap();
    run_research_extensions_contract(&store).await;
}
