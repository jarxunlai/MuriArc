use std::borrow::Cow;

use muriarc_core::{LOCAL_LAB_ID, LOCAL_USER_ID};
use sqlx::{SqlitePool, migrate::Migrator, sqlite::SqlitePoolOptions};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

type LegacyProviderSecretBundle = (Option<i64>, Option<Vec<u8>>, Option<Vec<u8>>, i64);

async fn memory_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("migration test database must open");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys must be enabled");
    pool
}

fn vision_profile_id(user_id: &str) -> String {
    assert_eq!(
        &user_id[14..15],
        "4",
        "compatibility profile fixtures must use UUIDv4 user ids"
    );
    format!("{}f{}", &user_id[..14], &user_id[15..])
}

#[tokio::test]
async fn sqlite_model_profile_migration_supports_fresh_database_and_ledger_replay() {
    let pool = memory_pool().await;

    MIGRATOR
        .run(&pool)
        .await
        .expect("fresh migrations must succeed");
    MIGRATOR
        .run(&pool)
        .await
        .expect("replaying the SQLx migration ledger must be idempotent");

    let ledger: (i64, Option<i64>) =
        sqlx::query_as("SELECT count(*), max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("migration ledger must be readable");
    assert_eq!(
        MIGRATOR.iter().count(),
        29,
        "the merged SQLite migration set must contain 29 files (version 0005 is intentionally absent)"
    );
    assert_eq!(
        MIGRATOR.iter().map(|migration| migration.version).max(),
        Some(30),
        "the merged SQLite migration set must end at 0030"
    );
    assert_eq!(
        ledger,
        (
            i64::try_from(MIGRATOR.iter().count()).unwrap(),
            MIGRATOR.iter().map(|migration| migration.version).max(),
        )
    );

    for table in [
        "ai_model_profiles",
        "ai_model_profile_versions",
        "ai_model_profile_secret_refs",
        "ai_user_model_defaults",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|error| panic!("{table} schema lookup failed: {error}"));
        assert_eq!(exists, 1, "{table} must exist after a fresh migration");
    }

    let secret_ref_columns: Vec<(String, i64)> = sqlx::query_as(
        "SELECT name, pk
         FROM pragma_table_info('ai_model_profile_secret_refs')
         ORDER BY cid",
    )
    .fetch_all(&pool)
    .await
    .expect("Desktop secret reference columns must be readable");
    assert_eq!(
        secret_ref_columns,
        vec![
            ("profile_id".to_owned(), 1),
            ("profile_version".to_owned(), 2),
            ("keyring_account".to_owned(), 0),
            ("credential_state".to_owned(), 0),
            ("created_at".to_owned(), 0),
            ("updated_at".to_owned(), 0),
            ("revision".to_owned(), 0),
        ],
        "Desktop keyring metadata must be profile-version scoped and contain no secret material"
    );
    let secret_ref_foreign_key: Vec<(i64, i64, String, String, String)> = sqlx::query_as(
        "SELECT id, seq, \"table\", \"from\", \"to\"
         FROM pragma_foreign_key_list('ai_model_profile_secret_refs')
         ORDER BY id, seq",
    )
    .fetch_all(&pool)
    .await
    .expect("Desktop secret reference foreign key must be readable");
    assert!(
        secret_ref_foreign_key.windows(2).any(|rows| {
            rows[0].0 == rows[1].0
                && rows[0].2 == "ai_model_profile_versions"
                && rows[1].2 == "ai_model_profile_versions"
                && rows[0].3 == "profile_id"
                && rows[0].4 == "profile_id"
                && rows[1].3 == "profile_version"
                && rows[1].4 == "version"
        }),
        "Desktop keyring metadata must reference an exact immutable profile version"
    );

    for column in [
        "model_profile_id",
        "model_profile_version",
        "legacy_read_only",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('ai_conversations') WHERE name = ?",
        )
        .bind(column)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|error| panic!("{column} schema lookup failed: {error}"));
        assert_eq!(
            exists, 1,
            "ai_conversations.{column} must exist after a fresh migration"
        );
    }

    pool.close().await;
}

#[tokio::test]
async fn sqlite_model_profile_migration_rejects_malformed_or_incomplete_legacy_provider_json() {
    for (case, provider_config) in [
        ("malformed", "{not-json"),
        (
            "missing model",
            r#"{"base_url":"https://provider.example.test/v1"}"#,
        ),
    ] {
        let pool = memory_pool().await;
        let through_0022 = Migrator {
            migrations: Cow::Owned(
                MIGRATOR
                    .iter()
                    .filter(|migration| migration.version <= 22)
                    .cloned()
                    .collect(),
            ),
            ..Migrator::DEFAULT
        };
        through_0022
            .run(&pool)
            .await
            .expect("migration prefix through 0022 must succeed");

        let now = "2026-07-23T00:00:00Z";
        sqlx::query(
            "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
             VALUES (?, 'Local migration lab', ?, ?, NULL, 1)",
        )
        .bind(LOCAL_LAB_ID.to_string())
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("legacy local lab fixture must be inserted");
        sqlx::query(
            "INSERT INTO users (
                id, lab_id, email, display_name, status, created_at, updated_at,
                deleted_at, revision
             ) VALUES (?, ?, 'local-migration@example.test', 'Local migration user',
                'active', ?, ?, NULL, 1)",
        )
        .bind(LOCAL_USER_ID.to_string())
        .bind(LOCAL_LAB_ID.to_string())
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("legacy local user fixture must be inserted");
        sqlx::query(
            "INSERT INTO ai_provider_settings (
                user_id, enabled, provider_config, provider_preset_id,
                supports_vision, vision_model, context_window_tokens,
                max_input_tokens, max_output_tokens, history_token_budget,
                history_turns, temperature, timeout_ms, created_at, updated_at,
                revision
             ) VALUES (?, 1, ?, 'custom-openai-compatible', 0, NULL,
                65536, 32768, 2048, 16384, 12, 0, 120000, ?, ?, 1)",
        )
        .bind(LOCAL_USER_ID.to_string())
        .bind(provider_config)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("invalid legacy provider JSON is allowed by the legacy schema");

        let migration = MIGRATOR.run(&pool).await;
        assert!(
            migration.is_err(),
            "{case} legacy provider JSON must fail migration instead of being silently ignored"
        );
        let migration_0023_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 23")
                .fetch_one(&pool)
                .await
                .expect("migration ledger must remain readable after rollback");
        assert_eq!(
            migration_0023_count, 0,
            "{case} legacy provider JSON must not be recorded as migrated"
        );
        let legacy_settings_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ai_provider_settings WHERE user_id = ?")
                .bind(LOCAL_USER_ID.to_string())
                .fetch_one(&pool)
                .await
                .expect("legacy settings must remain intact after rollback");
        assert_eq!(legacy_settings_count, 1);

        pool.close().await;
    }
}

#[tokio::test]
async fn sqlite_model_profile_migration_uses_collision_free_vision_profile_ids() {
    let pool = memory_pool().await;
    let through_0022 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 22)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0022
        .run(&pool)
        .await
        .expect("migration prefix through 0022 must succeed");

    let lab_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let first_user_id = "11111111-1111-4111-8111-111111111111";
    let paired_user_id = "91111111-1111-4111-8111-111111111111";
    let now = "2026-07-23T00:00:00Z";
    assert_eq!(
        format!("9{}", &first_user_id[1..]),
        paired_user_id,
        "the fixture must collide under the retired first-nibble mapping"
    );

    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
         VALUES (?, 'Collision migration lab', ?, ?, NULL, 1)",
    )
    .bind(lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("collision migration lab fixture must be inserted");

    for (user_id, email, text_model, vision_model) in [
        (
            first_user_id,
            "first-collision@example.test",
            "first-chat",
            "first-vision",
        ),
        (
            paired_user_id,
            "paired-collision@example.test",
            "paired-chat",
            "paired-vision",
        ),
    ] {
        sqlx::query(
            "INSERT INTO users (
                id, lab_id, email, display_name, status, created_at, updated_at,
                deleted_at, revision
             ) VALUES (?, ?, ?, 'Collision migration user', 'active', ?, ?, NULL, 1)",
        )
        .bind(user_id)
        .bind(lab_id)
        .bind(email)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("collision migration user fixture must be inserted");
        sqlx::query(
            "INSERT INTO ai_provider_settings (
                user_id, enabled, provider_config, provider_preset_id,
                supports_vision, vision_model, context_window_tokens,
                max_input_tokens, max_output_tokens, history_token_budget,
                history_turns, temperature, timeout_ms, created_at, updated_at,
                revision
             ) VALUES (?, 1, ?, 'custom-openai-compatible', 1, ?,
                65536, 32768, 2048, 16384, 12, 0, 120000, ?, ?, 1)",
        )
        .bind(user_id)
        .bind(format!(
            r#"{{"kind":"open_ai_compatible","model":"{text_model}","base_url":"https://provider.example.test/v1"}}"#
        ))
        .bind(vision_model)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("collision migration provider settings must be inserted");
    }

    MIGRATOR
        .run(&pool)
        .await
        .expect("paired UUIDv4 users must migrate without profile id collisions");

    let profiles: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT user_id, name, id
         FROM ai_model_profiles
         ORDER BY user_id, name",
    )
    .fetch_all(&pool)
    .await
    .expect("collision migration profiles must be readable");
    assert_eq!(
        profiles,
        vec![
            (
                first_user_id.to_owned(),
                "Migrated default model".to_owned(),
                first_user_id.to_owned(),
            ),
            (
                first_user_id.to_owned(),
                "Migrated vision model".to_owned(),
                vision_profile_id(first_user_id),
            ),
            (
                paired_user_id.to_owned(),
                "Migrated default model".to_owned(),
                paired_user_id.to_owned(),
            ),
            (
                paired_user_id.to_owned(),
                "Migrated vision model".to_owned(),
                vision_profile_id(paired_user_id),
            ),
        ]
    );

    for user_id in [first_user_id, paired_user_id] {
        let defaults: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT default_conversation_profile_id, default_vision_profile_id
             FROM ai_user_model_defaults WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("collision migration defaults must be readable");
        assert_eq!(
            defaults,
            (Some(user_id.to_owned()), Some(vision_profile_id(user_id)))
        );
    }
}

#[tokio::test]
async fn sqlite_model_profile_migration_rejects_legacy_endpoint_identity_collisions() {
    let pool = memory_pool().await;
    let through_0022 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 22)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0022
        .run(&pool)
        .await
        .expect("migration prefix through 0022 must succeed");

    let lab_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let user_id = "11111111-1111-4111-8111-111111111111";
    let now = "2026-07-23T00:00:00Z";
    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
         VALUES (?, 'Endpoint collision lab', ?, ?, NULL, 1)",
    )
    .bind(lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("endpoint collision lab fixture must be inserted");
    sqlx::query(
        "INSERT INTO users (
            id, lab_id, email, display_name, status, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, 'endpoint-collision@example.test', 'Endpoint collision user',
            'active', ?, ?, NULL, 1)",
    )
    .bind(user_id)
    .bind(lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("endpoint collision user fixture must be inserted");

    for (id, provider_kind, label) in [
        (
            "22222222-2222-4222-8222-222222222222",
            "open_ai_compatible",
            "Legacy OpenAI-compatible endpoint",
        ),
        (
            "33333333-3333-4333-8333-333333333333",
            "local_http",
            "Legacy local endpoint",
        ),
    ] {
        sqlx::query(
            "INSERT INTO ai_provider_endpoints (
                id, lab_id, provider_kind, label, base_url, normalized_base_url,
                enabled, builtin, created_by, updated_by, created_at, updated_at,
                revision
             ) VALUES (?, ?, ?, ?, 'https://provider.example.test/v1/',
                'https://provider.example.test/v1', 1, 0, ?, ?, ?, ?, 1)",
        )
        .bind(id)
        .bind(lab_id)
        .bind(provider_kind)
        .bind(label)
        .bind(user_id)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("legacy endpoint pair is valid under the provider-kind identity");
    }

    let migration = MIGRATOR.run(&pool).await;
    assert!(
        migration.is_err(),
        "cross-transport legacy endpoint collisions must abort before table rebuild"
    );
    let migration_0023_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 23")
            .fetch_one(&pool)
            .await
            .expect("migration ledger must remain readable after endpoint preflight failure");
    assert_eq!(migration_0023_count, 0);
    let legacy_rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, provider_kind FROM ai_provider_endpoints ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("legacy endpoint rows must survive a failed migration");
    assert_eq!(
        legacy_rows,
        vec![
            (
                "22222222-2222-4222-8222-222222222222".to_owned(),
                "open_ai_compatible".to_owned(),
            ),
            (
                "33333333-3333-4333-8333-333333333333".to_owned(),
                "local_http".to_owned(),
            ),
        ]
    );
    let protocol_column_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pragma_table_info('ai_provider_endpoints')
         WHERE name = 'protocol'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy endpoint schema must remain readable");
    assert_eq!(
        protocol_column_count, 0,
        "the endpoint rebuild must not begin before conflict preflight succeeds"
    );
}

#[tokio::test]
async fn sqlite_model_profile_migration_projects_legacy_settings_and_schema_constraints() {
    let pool = memory_pool().await;
    let through_0022 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 22)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0022
        .run(&pool)
        .await
        .expect("migration prefix through 0022 must succeed");

    let lab_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let vision_user_id = "11111111-1111-4111-8111-111111111111";
    let text_only_user_id = "22222222-2222-4222-8222-222222222222";
    let legacy_conversation_id = "33333333-3333-4333-8333-333333333333";
    let endpoint_id = "44444444-4444-4444-8444-444444444444";
    let now = "2026-07-23T00:00:00Z";

    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
         VALUES (?, 'Migration Lab', ?, ?, NULL, 1)",
    )
    .bind(lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy lab fixture must be inserted");
    for (user_id, email, name) in [
        (
            vision_user_id,
            "vision-migration@example.test",
            "Vision User",
        ),
        (
            text_only_user_id,
            "text-migration@example.test",
            "Text User",
        ),
    ] {
        sqlx::query(
            "INSERT INTO users (
                id, lab_id, email, display_name, status, created_at, updated_at,
                deleted_at, revision
             ) VALUES (?, ?, ?, ?, 'active', ?, ?, NULL, 1)",
        )
        .bind(user_id)
        .bind(lab_id)
        .bind(email)
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("legacy user fixture must be inserted");
    }

    sqlx::query(
        "INSERT INTO ai_provider_settings (
            user_id, enabled, provider_config, provider_preset_id,
            secret_key_version, secret_nonce, secret_ciphertext,
            supports_vision, vision_model, context_window_tokens,
            max_input_tokens, max_output_tokens, history_token_budget,
            history_turns, temperature, timeout_ms, created_at, updated_at,
            revision
         ) VALUES (?, 1, ?, 'custom-openai-compatible', 7, ?, ?, 1, 'legacy-vision',
            131072, 65536, 4096, 32768, 20, 0.25, 90000, ?, ?, 4)",
    )
    .bind(vision_user_id)
    .bind(
        r#"{"provider_id":"legacy-vision-provider","kind":"open_ai_compatible","model":"legacy-chat","base_url":"https://provider.example.test/v1/","timeout_ms":90000,"max_response_bytes":2097152}"#,
    )
    .bind(vec![7_u8; 12])
    .bind(vec![8_u8; 32])
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy vision settings fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_provider_settings (
            user_id, enabled, provider_config, provider_preset_id,
            supports_vision, vision_model, context_window_tokens,
            max_input_tokens, max_output_tokens, history_token_budget,
            history_turns, temperature, timeout_ms, created_at, updated_at,
            revision
         ) VALUES (?, 1, ?, 'custom-openai-compatible', 0, NULL,
            65536, 32768, 2048, 16384, 12, 0, 120000, ?, ?, 2)",
    )
    .bind(text_only_user_id)
    .bind(
        r#"{"provider_id":"legacy-text-provider","kind":"local_http","model":"text-only","base_url":"https://text.example.test/api","timeout_ms":120000,"max_response_bytes":2097152}"#,
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy text settings fixture must be inserted");

    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, NULL, ?, 'Legacy conversation', ?, ?, NULL, 1)",
    )
    .bind(legacy_conversation_id)
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy conversation fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_provider_endpoints (
            id, lab_id, provider_kind, label, base_url, normalized_base_url,
            enabled, builtin, created_by, updated_by, created_at, updated_at,
            revision
         ) VALUES (?, ?, 'local_http', 'Legacy endpoint',
            'https://provider.example.test/v1/',
            'https://provider.example.test/v1', 1, 0, ?, ?, ?, ?, 3)",
    )
    .bind(endpoint_id)
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy endpoint fixture must be inserted");

    MIGRATOR
        .run(&pool)
        .await
        .expect("0022 to current migration must succeed");
    MIGRATOR
        .run(&pool)
        .await
        .expect("incremental migration ledger replay must be idempotent");

    let profile_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ai_model_profiles")
        .fetch_one(&pool)
        .await
        .expect("migrated profile count must be readable");
    assert_eq!(
        profile_count, 3,
        "vision settings produce two profiles and text-only settings produce one"
    );

    let vision_profile_id = vision_profile_id(vision_user_id);
    let text_profile: (String, String, i64, i64, String) = sqlx::query_as(
        "SELECT p.id, v.model_id, v.supports_vision, p.current_version, v.normalized_base_url
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = 1
         WHERE p.user_id = ? AND p.name = 'Migrated default model'",
    )
    .bind(vision_user_id)
    .fetch_one(&pool)
    .await
    .expect("migrated text profile must be readable");
    assert_eq!(
        text_profile,
        (
            vision_user_id.to_owned(),
            "legacy-chat".to_owned(),
            0,
            1,
            "https://provider.example.test/v1".to_owned(),
        )
    );

    let vision_profile: (String, String, i64, String, String) = sqlx::query_as(
        "SELECT p.id, v.model_id, v.supports_vision, v.protocol, v.transport
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = 1
         WHERE p.user_id = ? AND p.name = 'Migrated vision model'",
    )
    .bind(vision_user_id)
    .fetch_one(&pool)
    .await
    .expect("migrated vision profile must be readable");
    assert_eq!(
        vision_profile,
        (
            vision_profile_id.clone(),
            "legacy-vision".to_owned(),
            1,
            "openai_chat_completions".to_owned(),
            "open_ai_compatible".to_owned(),
        )
    );
    let text_only_transport: String = sqlx::query_scalar(
        "SELECT v.transport
         FROM ai_model_profiles p
         JOIN ai_model_profile_versions v
           ON v.profile_id = p.id AND v.version = 1
         WHERE p.user_id = ? AND p.name = 'Migrated default model'",
    )
    .bind(text_only_user_id)
    .fetch_one(&pool)
    .await
    .expect("legacy LocalHttp transport must be migrated");
    assert_eq!(text_only_transport, "local_http");
    sqlx::query(
        "INSERT INTO ai_model_profile_secret_refs (
            profile_id, profile_version, keyring_account, credential_state,
            created_at, updated_at, revision
         ) VALUES (?, 1, 'profile-v1-keyring-item', 'present', ?, ?, 1)",
    )
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("a keyring reference for an existing exact profile version must be accepted");
    let missing_secret_version = sqlx::query(
        "INSERT INTO ai_model_profile_secret_refs (
            profile_id, profile_version, keyring_account, credential_state,
            created_at, updated_at, revision
         ) VALUES (?, 2, 'missing-profile-v2-keyring-item', 'present', ?, ?, 1)",
    )
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        missing_secret_version.is_err(),
        "a keyring reference must not point at a missing profile version"
    );
    sqlx::query(
        "INSERT INTO ai_model_profile_versions (
            profile_id, version, protocol, transport, base_url, normalized_base_url,
            model_id, supports_vision, context_window_tokens, max_input_tokens,
            max_output_tokens, history_token_budget, history_turns,
            temperature, timeout_ms, created_at
         )
         SELECT profile_id, 2, protocol, transport, base_url, normalized_base_url,
            'legacy-chat-v2', supports_vision, context_window_tokens,
            max_input_tokens, max_output_tokens, history_token_budget,
            history_turns, temperature, timeout_ms, created_at
         FROM ai_model_profile_versions
         WHERE profile_id = ? AND version = 1",
    )
    .bind(vision_user_id)
    .execute(&pool)
    .await
    .expect("a second immutable profile version fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_model_profile_secret_refs (
            profile_id, profile_version, keyring_account, credential_state,
            created_at, updated_at, revision
         ) VALUES (?, 2, 'profile-v2-keyring-item', 'present', ?, ?, 1)",
    )
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("the same profile may have a separate reference for each version");
    let secret_ref_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ai_model_profile_secret_refs WHERE profile_id = ?",
    )
    .bind(vision_user_id)
    .fetch_one(&pool)
    .await
    .expect("profile-version keyring references must be readable");
    assert_eq!(secret_ref_count, 2);
    let mutate_version = sqlx::query(
        "UPDATE ai_model_profile_versions
         SET model_id = 'silently-rewritten'
         WHERE profile_id = ? AND version = 1",
    )
    .bind(vision_user_id)
    .execute(&pool)
    .await;
    assert!(
        mutate_version.is_err(),
        "persisted model profile versions must be immutable"
    );

    let defaults: (Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT default_conversation_profile_id, default_vision_profile_id, revision
         FROM ai_user_model_defaults WHERE user_id = ?",
    )
    .bind(vision_user_id)
    .fetch_one(&pool)
    .await
    .expect("migrated vision defaults must be readable");
    assert_eq!(
        defaults,
        (
            Some(vision_user_id.to_owned()),
            Some(vision_profile_id.clone()),
            1,
        )
    );
    let legacy_secret_bundle: LegacyProviderSecretBundle = sqlx::query_as(
        "SELECT secret_key_version, secret_nonce, secret_ciphertext, revision
             FROM ai_provider_settings WHERE user_id = ?",
    )
    .bind(vision_user_id)
    .fetch_one(&pool)
    .await
    .expect("legacy encrypted provider settings must remain readable");
    assert_eq!(
        legacy_secret_bundle,
        (Some(7), Some(vec![7_u8; 12]), Some(vec![8_u8; 32]), 4,),
        "forward migration must preserve the legacy credential bundle and key version"
    );

    let text_only: (i64, Option<String>) = sqlx::query_as(
        "SELECT count(p.id), max(d.default_vision_profile_id)
         FROM ai_model_profiles p
         JOIN ai_user_model_defaults d ON d.user_id = p.user_id
         WHERE p.user_id = ?",
    )
    .bind(text_only_user_id)
    .fetch_one(&pool)
    .await
    .expect("migrated text-only defaults must be readable");
    assert_eq!(text_only, (1, None));

    let legacy_binding: (Option<String>, Option<i64>, i64) = sqlx::query_as(
        "SELECT model_profile_id, model_profile_version, legacy_read_only
         FROM ai_conversations WHERE id = ?",
    )
    .bind(legacy_conversation_id)
    .fetch_one(&pool)
    .await
    .expect("legacy conversation binding must be readable");
    assert_eq!(legacy_binding, (None, None, 1));

    let new_conversation_id = "55555555-5555-4555-8555-555555555555";
    let unbound_writable = sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, NULL, ?, 'New unbound conversation', ?, ?, NULL, 1)",
    )
    .bind(new_conversation_id)
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        unbound_writable.is_err(),
        "new writable conversations must bind an immutable model profile version"
    );

    let incomplete_binding = sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            created_at, updated_at, deleted_at, revision
         ) VALUES (
            '66666666-6666-4666-8666-666666666666', ?, NULL, ?,
            'Incomplete binding', ?, ?, ?, NULL, 1
         )",
    )
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        incomplete_binding.is_err(),
        "a model profile id without a version must be rejected"
    );
    let unknown_binding = sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, created_at, updated_at, deleted_at, revision
         ) VALUES (
            '77777777-7777-4777-8777-777777777777', ?, NULL, ?,
            'Unknown binding', ?, 1, ?, ?, NULL, 1
         )",
    )
    .bind(lab_id)
    .bind(vision_user_id)
    .bind("66666666-6666-4666-8666-666666666666")
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        unknown_binding.is_err(),
        "a conversation must reference an existing immutable profile version"
    );
    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, created_at, updated_at, deleted_at, revision
         ) VALUES (
            '88888888-8888-4888-8888-888888888888', ?, NULL, ?,
            'Bound conversation', ?, 1, ?, ?, NULL, 1
         )",
    )
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("a complete existing profile-version binding must be accepted");
    let rebind = sqlx::query(
        "UPDATE ai_conversations
         SET model_profile_id = ?
         WHERE id = '88888888-8888-4888-8888-888888888888'",
    )
    .bind(&vision_profile_id)
    .execute(&pool)
    .await;
    assert!(
        rebind.is_err(),
        "an existing conversation model binding must be immutable"
    );

    let protocol: String =
        sqlx::query_scalar("SELECT protocol FROM ai_provider_endpoints WHERE id = ?")
            .bind(endpoint_id)
            .fetch_one(&pool)
            .await
            .expect("migrated endpoint protocol must be readable");
    assert_eq!(protocol, "openai_chat_completions");
    assert!(
        sqlx::query("UPDATE ai_provider_endpoints SET protocol = 'invalid_protocol' WHERE id = ?",)
            .bind(endpoint_id)
            .execute(&pool)
            .await
            .is_err(),
        "endpoint protocol must be schema constrained"
    );

    for (id, protocol) in [
        ("aaaaaaaa-7777-4777-8777-777777777777", "openai_responses"),
        ("aaaaaaaa-8888-4888-8888-888888888888", "anthropic_messages"),
    ] {
        sqlx::query(
            "INSERT INTO ai_provider_endpoints (
                id, lab_id, provider_kind, protocol, label, base_url,
                normalized_base_url, enabled, builtin, created_by, updated_by,
                created_at, updated_at, revision
             ) VALUES (?, ?, 'open_ai_compatible', ?, 'Protocol endpoint',
                'https://provider.example.test/v1/',
                'https://provider.example.test/v1', 1, 0, ?, ?, ?, ?, 1)",
        )
        .bind(id)
        .bind(lab_id)
        .bind(protocol)
        .bind(vision_user_id)
        .bind(vision_user_id)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| {
            panic!("distinct protocol identity {protocol} must be accepted: {error}")
        });
    }
    let duplicate_protocol = sqlx::query(
        "INSERT INTO ai_provider_endpoints (
            id, lab_id, provider_kind, protocol, label, base_url,
            normalized_base_url, enabled, builtin, created_by, updated_by,
            created_at, updated_at, revision
         ) VALUES (
            '99999999-9999-4999-8999-999999999999', ?,
            'open_ai_compatible', 'openai_chat_completions',
            'Duplicate endpoint', 'https://provider.example.test/v1/',
            'https://provider.example.test/v1', 1, 0, ?, ?, ?, ?, 1
         )",
    )
    .bind(lab_id)
    .bind(vision_user_id)
    .bind(vision_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        duplicate_protocol.is_err(),
        "the same lab, protocol, and normalized URL must be unique"
    );

    let duplicate_active_name = sqlx::query(
        "INSERT INTO ai_model_profiles (
            id, lab_id, user_id, name, current_version, created_at, updated_at,
            archived_at, deleted_at, revision
         ) VALUES (
            'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', ?, ?,
            'Migrated default model', 1, ?, ?, NULL, NULL, 1
         )",
    )
    .bind(lab_id)
    .bind(text_only_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(
        duplicate_active_name.is_err(),
        "active profile names must be unique per user"
    );
    sqlx::query(
        "UPDATE ai_model_profiles SET archived_at = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(text_only_user_id)
    .execute(&pool)
    .await
    .expect("legacy text profile must be archivable");
    sqlx::query(
        "INSERT INTO ai_model_profiles (
            id, lab_id, user_id, name, current_version, created_at, updated_at,
            archived_at, deleted_at, revision
         ) VALUES (
            'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', ?, ?,
            'Migrated default model', 1, ?, ?, NULL, NULL, 1
         )",
    )
    .bind(lab_id)
    .bind(text_only_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("an archived profile name must be reusable");

    let migration_0023_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 23")
            .fetch_one(&pool)
            .await
            .expect("0023 migration ledger entry must be readable");
    assert_eq!(migration_0023_count, 1);

    pool.close().await;
}

#[tokio::test]
async fn sqlite_compatibility_finalize_repairs_only_invalid_defaults() {
    let pool = memory_pool().await;
    let through_0024 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 24)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0024
        .run(&pool)
        .await
        .expect("migration prefix through 0024 must succeed");

    let lab_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let invalid_user_id = "11111111-1111-4111-8111-111111111111";
    let valid_user_id = "22222222-2222-4222-8222-222222222222";
    let cross_owner_user_id = "33333333-3333-4333-8333-333333333333";
    let archived_profile_id = "44444444-4444-4444-8444-444444444444";
    let non_vision_profile_id = "55555555-5555-4555-8555-555555555555";
    let valid_vision_profile_id = "66666666-6666-4666-8666-666666666666";
    let bound_conversation_id = "77777777-7777-4777-8777-777777777777";
    let legacy_conversation_id = "88888888-8888-4888-8888-888888888888";
    let now = "2026-07-23T00:00:00Z";

    sqlx::query(
        "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
         VALUES (?, 'Compatibility repair lab', ?, ?, NULL, 1)",
    )
    .bind(lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("compatibility lab fixture must be inserted");
    for (user_id, email) in [
        (invalid_user_id, "invalid-default@example.test"),
        (valid_user_id, "valid-default@example.test"),
        (cross_owner_user_id, "cross-owner-default@example.test"),
    ] {
        sqlx::query(
            "INSERT INTO users (
                id, lab_id, email, display_name, status, created_at, updated_at,
                deleted_at, revision
             ) VALUES (?, ?, ?, 'Compatibility user', 'active', ?, ?, NULL, 1)",
        )
        .bind(user_id)
        .bind(lab_id)
        .bind(email)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("compatibility user fixture must be inserted");
    }

    for (profile_id, user_id, name, supports_vision, archived_at) in [
        (
            archived_profile_id,
            invalid_user_id,
            "Archived conversation default",
            0_i64,
            Some(now),
        ),
        (
            non_vision_profile_id,
            invalid_user_id,
            "Non-vision default",
            0_i64,
            None,
        ),
        (
            valid_vision_profile_id,
            valid_user_id,
            "Valid vision default",
            1_i64,
            None,
        ),
    ] {
        sqlx::query(
            "INSERT INTO ai_model_profiles (
                id, lab_id, user_id, name, current_version, created_at,
                updated_at, archived_at, deleted_at, revision
             ) VALUES (?, ?, ?, ?, 1, ?, ?, ?, NULL, 1)",
        )
        .bind(profile_id)
        .bind(lab_id)
        .bind(user_id)
        .bind(name)
        .bind(now)
        .bind(now)
        .bind(archived_at)
        .execute(&pool)
        .await
        .expect("compatibility profile fixture must be inserted");
        sqlx::query(
            "INSERT INTO ai_model_profile_versions (
                profile_id, version, protocol, transport, base_url,
                normalized_base_url, model_id, supports_vision,
                context_window_tokens, max_input_tokens, max_output_tokens,
                history_token_budget, history_turns, temperature, timeout_ms,
                created_at
             ) VALUES (
                ?, 1, 'openai_chat_completions', 'open_ai_compatible',
                'https://provider.example.test/v1',
                'https://provider.example.test/v1', ?, ?,
                16384, 8192, 2048, 4096, 20, 0, 30000, ?
             )",
        )
        .bind(profile_id)
        .bind(format!("model-{profile_id}"))
        .bind(supports_vision)
        .bind(now)
        .execute(&pool)
        .await
        .expect("compatibility profile version fixture must be inserted");
    }

    for (user_id, conversation_profile, vision_profile, revision) in [
        (
            invalid_user_id,
            archived_profile_id,
            non_vision_profile_id,
            4_i64,
        ),
        (
            valid_user_id,
            valid_vision_profile_id,
            valid_vision_profile_id,
            5_i64,
        ),
        (
            cross_owner_user_id,
            valid_vision_profile_id,
            valid_vision_profile_id,
            6_i64,
        ),
    ] {
        sqlx::query(
            "INSERT INTO ai_user_model_defaults (
                user_id, default_conversation_profile_id,
                default_vision_profile_id, created_at, updated_at, deleted_at,
                revision
             ) VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(user_id)
        .bind(conversation_profile)
        .bind(vision_profile)
        .bind(now)
        .bind(now)
        .bind(revision)
        .execute(&pool)
        .await
        .expect("compatibility default fixture must be inserted");
    }

    sqlx::query(
        "INSERT INTO ai_provider_settings (
            user_id, enabled, provider_config, provider_preset_id,
            secret_key_version, secret_nonce, secret_ciphertext,
            supports_vision, vision_model, context_window_tokens,
            max_input_tokens, max_output_tokens, history_token_budget,
            history_turns, temperature, timeout_ms, created_at, updated_at,
            revision
         ) VALUES (
            ?, 1, ?, 'custom-openai-compatible', 11, ?, ?, 0, NULL,
            16384, 8192, 2048, 4096, 20, 0, 30000, ?, ?, 9
         )",
    )
    .bind(invalid_user_id)
    .bind(
        r#"{"kind":"open_ai_compatible","model":"legacy-model","base_url":"https://provider.example.test/v1"}"#,
    )
    .bind(vec![11_u8; 12])
    .bind(vec![12_u8; 32])
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy credential fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_model_profile_secret_refs (
            profile_id, profile_version, keyring_account, credential_state,
            created_at, updated_at, revision
         ) VALUES (?, 1, 'compatibility-profile-v1', 'present', ?, ?, 3)",
    )
    .bind(archived_profile_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("profile-version keyring reference fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, legacy_read_only, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, NULL, ?, 'Archived model history', ?, 1, 0, ?, ?, NULL, 1)",
    )
    .bind(bound_conversation_id)
    .bind(lab_id)
    .bind(invalid_user_id)
    .bind(archived_profile_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("bound historical conversation fixture must be inserted");
    sqlx::query(
        "INSERT INTO ai_conversations (
            id, lab_id, project_id, user_id, title, model_profile_id,
            model_profile_version, legacy_read_only, created_at, updated_at,
            deleted_at, revision
         ) VALUES (?, ?, NULL, ?, 'Legacy read-only history', NULL, NULL, 1, ?, ?, NULL, 1)",
    )
    .bind(legacy_conversation_id)
    .bind(lab_id)
    .bind(invalid_user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("legacy read-only conversation fixture must be inserted");

    MIGRATOR
        .run(&pool)
        .await
        .expect("compatibility finalization migration must succeed");
    MIGRATOR
        .run(&pool)
        .await
        .expect("compatibility finalization ledger replay must be idempotent");

    let invalid_defaults: (Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT default_conversation_profile_id, default_vision_profile_id,
            revision
         FROM ai_user_model_defaults WHERE user_id = ?",
    )
    .bind(invalid_user_id)
    .fetch_one(&pool)
    .await
    .expect("repaired invalid defaults must be readable");
    assert_eq!(invalid_defaults, (None, None, 5));
    let valid_defaults: (Option<String>, Option<String>, i64, String) = sqlx::query_as(
        "SELECT default_conversation_profile_id, default_vision_profile_id,
            revision, updated_at
         FROM ai_user_model_defaults WHERE user_id = ?",
    )
    .bind(valid_user_id)
    .fetch_one(&pool)
    .await
    .expect("valid defaults must be readable");
    assert_eq!(
        valid_defaults,
        (
            Some(valid_vision_profile_id.to_owned()),
            Some(valid_vision_profile_id.to_owned()),
            5,
            now.to_owned(),
        ),
        "valid defaults must remain byte-for-byte unchanged"
    );
    let cross_owner_defaults: (Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT default_conversation_profile_id, default_vision_profile_id,
            revision
         FROM ai_user_model_defaults WHERE user_id = ?",
    )
    .bind(cross_owner_user_id)
    .fetch_one(&pool)
    .await
    .expect("cross-owner defaults must be readable");
    assert_eq!(
        cross_owner_defaults,
        (None, None, 7),
        "both invalid references must be repaired with one revision advance"
    );

    let retained_legacy_secret: LegacyProviderSecretBundle = sqlx::query_as(
        "SELECT secret_key_version, secret_nonce, secret_ciphertext, revision
             FROM ai_provider_settings WHERE user_id = ?",
    )
    .bind(invalid_user_id)
    .fetch_one(&pool)
    .await
    .expect("legacy credential row must remain readable");
    assert_eq!(
        retained_legacy_secret,
        (Some(11), Some(vec![11_u8; 12]), Some(vec![12_u8; 32]), 9,)
    );
    let retained_secret_ref: (i64, String, String, i64) = sqlx::query_as(
        "SELECT profile_version, keyring_account, credential_state, revision
         FROM ai_model_profile_secret_refs WHERE profile_id = ?",
    )
    .bind(archived_profile_id)
    .fetch_one(&pool)
    .await
    .expect("profile-version keyring reference must remain readable");
    assert_eq!(
        retained_secret_ref,
        (
            1,
            "compatibility-profile-v1".to_owned(),
            "present".to_owned(),
            3,
        )
    );
    let retained_bindings: Vec<(String, Option<String>, Option<i64>, i64)> = sqlx::query_as(
        "SELECT id, model_profile_id, model_profile_version, legacy_read_only
         FROM ai_conversations
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind(bound_conversation_id)
    .bind(legacy_conversation_id)
    .fetch_all(&pool)
    .await
    .expect("historical conversation bindings must remain readable");
    assert_eq!(
        retained_bindings,
        vec![
            (
                bound_conversation_id.to_owned(),
                Some(archived_profile_id.to_owned()),
                Some(1),
                0,
            ),
            (legacy_conversation_id.to_owned(), None, None, 1,),
        ]
    );
    let retained_profiles: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ai_model_profiles),
            (SELECT count(*) FROM ai_model_profile_versions)",
    )
    .fetch_one(&pool)
    .await
    .expect("profile compatibility rows must remain readable");
    assert_eq!(retained_profiles, (3, 3));

    pool.close().await;
}
