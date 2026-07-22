use chrono::Utc;
use muriarc_core::{AuditContext, Lab, MuriArcStore, User, WriteSource};
use muriarc_store_sqlite::SqliteStore;
use sqlx::Row;

#[tokio::test]
async fn sqlite_ai_provider_settings_have_safe_defaults_and_budget_constraints() {
    let store = SqliteStore::in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Migration);
    let lab = Lab::new("SQLite AI settings contract", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let user = User::new(lab.id, "sqlite-ai@example.test", "SQLite AI user", now).unwrap();
    store.create_user(&user, &audit).await.unwrap();

    sqlx::query(
        "INSERT INTO ai_provider_settings (user_id, provider_config, created_at, updated_at, revision) VALUES (?1, ?2, ?3, ?3, 1)",
    )
    .bind(user.id.to_string())
    .bind(r#"{"provider_id":"desktop-user-provider","kind":"open_ai_compatible","model":"deepseek-chat","base_url":"https://api.deepseek.com"}"#)
    .bind(now.to_rfc3339())
    .execute(store.pool())
    .await
    .unwrap();

    let row = sqlx::query(
        "SELECT enabled, provider_preset_id, secret_ciphertext, context_window_tokens, max_input_tokens, max_output_tokens, history_token_budget, history_turns, temperature, timeout_ms FROM ai_provider_settings WHERE user_id = ?1",
    )
    .bind(user.id.to_string())
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<i64, _>("enabled"), 1);
    assert_eq!(row.get::<String, _>("provider_preset_id"), "deepseek");
    assert!(row.get::<Option<Vec<u8>>, _>("secret_ciphertext").is_none());
    assert_eq!(row.get::<i64, _>("context_window_tokens"), 131_072);
    assert_eq!(row.get::<i64, _>("max_input_tokens"), 65_536);
    assert_eq!(row.get::<i64, _>("max_output_tokens"), 4_096);
    assert_eq!(row.get::<i64, _>("history_token_budget"), 32_768);
    assert_eq!(row.get::<i64, _>("history_turns"), 20);
    assert_eq!(row.get::<f64, _>("temperature"), 0.0);
    assert_eq!(row.get::<i64, _>("timeout_ms"), 120_000);

    let invalid = sqlx::query(
        "UPDATE ai_provider_settings SET context_window_tokens = 4096, max_input_tokens = 4096, max_output_tokens = 1 WHERE user_id = ?1",
    )
    .bind(user.id.to_string())
    .execute(store.pool())
    .await;
    assert!(invalid.is_err());
}
