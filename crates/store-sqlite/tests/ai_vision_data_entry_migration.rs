use std::borrow::Cow;

use sqlx::{Row, SqlitePool, migrate::Migrator, sqlite::SqlitePoolOptions};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations/sqlite");

struct Phase4SqlFixture {
    lab_id: String,
    user_id: String,
    project_id: String,
    experiment_id: String,
    event_id: String,
    definition_id: String,
    image_id: String,
    attachment_id: String,
    profile_id: String,
}

async fn insert_phase4_draft(
    pool: &SqlitePool,
    fixture: &Phase4SqlFixture,
    draft_id: &str,
    include_cell: bool,
    include_model_trace: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ai_extraction_drafts(
            id,lab_id,user_id,project_id,experiment_id,experiment_event_id,
            private_image_id,attachment_id,image_sha256,provider,model,tool_run_id,
            data_cell_definition_id,data_cell_subject_type,data_cell_subject_id,
            model_profile_id,model_profile_version,model_purpose,
            usage_input_tokens,usage_output_tokens,usage_total_tokens,
            provider_request_id,trace_json,status,items_json,error_code,
            created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(draft_id)
    .bind(&fixture.lab_id)
    .bind(&fixture.user_id)
    .bind(&fixture.project_id)
    .bind(&fixture.experiment_id)
    .bind(&fixture.event_id)
    .bind(&fixture.image_id)
    .bind(&fixture.attachment_id)
    .bind("a".repeat(64))
    .bind("migration-provider")
    .bind("migration-vision")
    .bind(None::<String>)
    .bind(include_cell.then_some(fixture.definition_id.as_str()))
    .bind(include_cell.then_some("experiment"))
    .bind(include_cell.then_some(fixture.experiment_id.as_str()))
    .bind(include_model_trace.then_some(fixture.profile_id.as_str()))
    .bind(include_model_trace.then_some(1_i64))
    .bind(include_model_trace.then_some("vision"))
    .bind(include_model_trace.then_some(10_i64))
    .bind(include_model_trace.then_some(5_i64))
    .bind(include_model_trace.then_some(15_i64))
    .bind(None::<String>)
    .bind(include_model_trace.then_some(r#"{"route":"migration-test"}"#))
    .bind("pending_approval")
    .bind("[]")
    .bind(None::<String>)
    .bind("2026-07-23T00:00:00Z")
    .bind("2026-07-23T00:00:00Z")
    .bind(None::<String>)
    .bind(1_i64)
    .execute(pool)
    .await
    .map(|_| ())
}

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

#[tokio::test]
async fn sqlite_vision_data_entry_migrates_existing_private_ai_input_in_place() {
    let pool = memory_pool().await;
    let through_0023 = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= 23)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    through_0023
        .run(&pool)
        .await
        .expect("migration prefix through 0023 must succeed");

    let lab_id = Uuid::new_v4().to_string();
    let user_id = Uuid::new_v4().to_string();
    let project_id = Uuid::new_v4().to_string();
    let image_id = Uuid::new_v4().to_string();
    let attachment_id = Uuid::new_v4().to_string();
    let derivative_id = Uuid::new_v4().to_string();
    let now = "2026-07-23T00:00:00Z";
    sqlx::query(
        "INSERT INTO labs(id,name,created_at,updated_at,deleted_at,revision)
         VALUES(?, 'phase4 migration lab', ?, ?, NULL, 1)",
    )
    .bind(&lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO users(
            id,lab_id,email,display_name,status,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,'phase4-migration@example.test','phase4 owner','active',?,?,NULL,1)",
    )
    .bind(&user_id)
    .bind(&lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects(
            id,lab_id,name,description,status,created_at,updated_at,deleted_at,revision
         )VALUES(?,?, 'phase4 migration project', NULL, 'active', ?, ?, NULL, 1)",
    )
    .bind(&project_id)
    .bind(&lab_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO attachments(
            id,lab_id,project_id,entity_type,entity_id,file_name,media_type,
            relative_path,size_bytes,sha256,version,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,NULL,'ai_private_image',?,'legacy.png','image/png',
            'objects/legacy',12,?,1,?,?,NULL,1)",
    )
    .bind(&attachment_id)
    .bind(&lab_id)
    .bind(&image_id)
    .bind("a".repeat(64))
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_private_images(
            id,lab_id,user_id,conversation_id,attachment_id,project_id,status,
            last_activity_at,expires_at,archived_at,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,NULL,?,NULL,'active',?,'2026-08-23T00:00:00Z',
            NULL,?,?,NULL,1)",
    )
    .bind(&image_id)
    .bind(&lab_id)
    .bind(&user_id)
    .bind(&attachment_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO attachment_derivatives(
            id,lab_id,project_id,attachment_id,kind,media_type,relative_path,
            size_bytes,sha256,status,error_code,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,?, 'ai_input','image/png','objects/sanitized',10,?,
            'ready',NULL,?,?,NULL,1)",
    )
    .bind(&derivative_id)
    .bind(&lab_id)
    .bind(&project_id)
    .bind(&attachment_id)
    .bind("b".repeat(64))
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    MIGRATOR
        .run(&pool)
        .await
        .expect("0024 upgrade must succeed");
    MIGRATOR
        .run(&pool)
        .await
        .expect("migration ledger replay must remain idempotent");

    let derivative_project: Option<String> =
        sqlx::query_scalar("SELECT project_id FROM attachment_derivatives WHERE id=?")
            .bind(&derivative_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        derivative_project, None,
        "legacy private AiInput derivatives must not remain project scoped"
    );
    let evidence_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master
         WHERE type='table' AND name='ai_extraction_evidence'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(evidence_table, 1);
    let draft_columns = sqlx::query("SELECT name FROM pragma_table_info('ai_extraction_drafts')")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<std::collections::HashSet<_>>();
    for column in [
        "data_cell_definition_id",
        "data_cell_subject_type",
        "data_cell_subject_id",
        "model_profile_id",
        "model_profile_version",
        "model_purpose",
        "usage_input_tokens",
        "usage_output_tokens",
        "usage_total_tokens",
        "provider_request_id",
        "trace_json",
    ] {
        assert!(
            draft_columns.contains(column),
            "missing upgraded column {column}"
        );
    }

    let fixture = Phase4SqlFixture {
        lab_id: lab_id.clone(),
        user_id: user_id.clone(),
        project_id: project_id.clone(),
        experiment_id: Uuid::new_v4().to_string(),
        event_id: Uuid::new_v4().to_string(),
        definition_id: Uuid::new_v4().to_string(),
        image_id: image_id.clone(),
        attachment_id: attachment_id.clone(),
        profile_id: Uuid::new_v4().to_string(),
    };
    sqlx::query(
        "INSERT INTO experiments(
            id,lab_id,project_id,template_version_id,name,description,status,
            starts_at,ends_at,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,NULL,'phase4 SQL constraints',NULL,'draft',NULL,NULL,?,?,NULL,1)",
    )
    .bind(&fixture.experiment_id)
    .bind(&fixture.lab_id)
    .bind(&fixture.project_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO experiment_events(
            id,lab_id,project_id,experiment_id,event_key,label,occurred_at,
            details_json,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,?, 'phase4_sql','Phase 4 SQL',?,'{}',?,?,NULL,1)",
    )
    .bind(&fixture.event_id)
    .bind(&fixture.lab_id)
    .bind(&fixture.project_id)
    .bind(&fixture.experiment_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO observation_definitions(
            id,lab_id,project_id,experiment_id,observation_key,label,value_type,
            unit,categories_json,policy,created_at,updated_at,deleted_at,revision
         )VALUES(?,?,?,?, 'phase4_sql','Phase 4 SQL','number',NULL,'[]',
            'versioned',?,?,NULL,1)",
    )
    .bind(&fixture.definition_id)
    .bind(&fixture.lab_id)
    .bind(&fixture.project_id)
    .bind(&fixture.experiment_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_model_profiles(
            id,lab_id,user_id,name,current_version,created_at,updated_at,
            archived_at,deleted_at,revision
         )VALUES(?,?,?,'phase4 migration vision',1,?,?,NULL,NULL,1)",
    )
    .bind(&fixture.profile_id)
    .bind(&fixture.lab_id)
    .bind(&fixture.user_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_model_profile_versions(
            profile_id,version,protocol,transport,base_url,normalized_base_url,
            model_id,supports_vision,context_window_tokens,max_input_tokens,
            max_output_tokens,history_token_budget,history_turns,temperature,
            timeout_ms,created_at
         )VALUES(?,1,'openai_responses','open_ai_compatible',
            'https://vision.example.test/v1','https://vision.example.test/v1',
            'migration-vision',1,4096,2048,1024,1024,4,0,30000,?)",
    )
    .bind(&fixture.profile_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    for (include_cell, include_model_trace) in [(true, false), (false, true)] {
        let invalid_id = Uuid::new_v4().to_string();
        assert!(
            insert_phase4_draft(
                &pool,
                &fixture,
                &invalid_id,
                include_cell,
                include_model_trace,
            )
            .await
            .is_err(),
            "data-cell and exact model trace bindings must be all-or-none"
        );
    }
    let valid_draft_id = Uuid::new_v4().to_string();
    insert_phase4_draft(&pool, &fixture, &valid_draft_id, true, true)
        .await
        .expect("a complete phase 4 binding must satisfy migration SQL");
    let duplicate_draft_id = Uuid::new_v4().to_string();
    assert!(
        insert_phase4_draft(&pool, &fixture, &duplicate_draft_id, true, true)
            .await
            .is_err(),
        "one user must not have two unresolved drafts for the same data cell"
    );
    sqlx::query(
        "UPDATE ai_extraction_drafts
         SET status='rejected',updated_at='2026-07-23T00:00:01Z',revision=2
         WHERE id=?",
    )
    .bind(&valid_draft_id)
    .execute(&pool)
    .await
    .expect("rejecting the unresolved winner must release the data cell");
    insert_phase4_draft(&pool, &fixture, &duplicate_draft_id, true, true)
        .await
        .expect("a rejected draft must release the partial unique constraint");
    sqlx::query(
        "UPDATE ai_extraction_drafts
         SET status='rejected',updated_at='2026-07-23T00:00:02Z',revision=2
         WHERE id=?",
    )
    .bind(&duplicate_draft_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_extraction_evidence(
            draft_id,display_order,private_image_id,private_attachment_id,
            promoted_attachment_id,original_sha256,sanitized_sha256,
            created_at,updated_at,revision
         )VALUES(?,0,?,?,NULL,?,?,?, ?,1)",
    )
    .bind(&valid_draft_id)
    .bind(&fixture.image_id)
    .bind(&fixture.attachment_id)
    .bind("a".repeat(64))
    .bind("b".repeat(64))
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        sqlx::query(
            "UPDATE ai_extraction_evidence
             SET promoted_attachment_id=private_attachment_id,
                 updated_at='2026-07-23T00:00:01Z'
             WHERE draft_id=? AND display_order=0",
        )
        .bind(&valid_draft_id)
        .execute(&pool)
        .await
        .is_err(),
        "promotion must advance the evidence revision"
    );
    sqlx::query(
        "UPDATE ai_extraction_evidence
         SET promoted_attachment_id=private_attachment_id,
             updated_at='2026-07-23T00:00:01Z',revision=2
         WHERE draft_id=? AND display_order=0",
    )
    .bind(&valid_draft_id)
    .execute(&pool)
    .await
    .expect("the one-time evidence promotion transition must be allowed");
    assert!(
        sqlx::query(
            "UPDATE ai_extraction_evidence
             SET updated_at='2026-07-23T00:00:02Z',revision=3
             WHERE draft_id=? AND display_order=0",
        )
        .bind(&valid_draft_id)
        .execute(&pool)
        .await
        .is_err(),
        "promoted evidence must reject even metadata-only updates"
    );
    assert!(
        sqlx::query(
            "UPDATE ai_extraction_evidence
             SET promoted_attachment_id=NULL,
                 updated_at='2026-07-23T00:00:02Z',revision=3
             WHERE draft_id=? AND display_order=0",
        )
        .bind(&valid_draft_id)
        .execute(&pool)
        .await
        .is_err(),
        "promoted evidence must never be cleared or rewritten"
    );

    let migration_0024: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version=24")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(migration_0024, 1);
}
