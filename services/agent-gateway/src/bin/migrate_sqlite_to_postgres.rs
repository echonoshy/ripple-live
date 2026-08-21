use std::{path::PathBuf, str::FromStr};

use anyhow::Context;
use sqlx::{
    PgPool, Postgres, QueryBuilder, Row, SqlitePool, postgres::PgPoolOptions,
    sqlite::SqliteConnectOptions,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sqlite_path = std::env::var("RIPPLE_SQLITE_PATH")
        .map(PathBuf::from)
        .context("缺少 RIPPLE_SQLITE_PATH")?;
    let database_url = std::env::var("RIPPLE_DATABASE_URL").context("缺少 RIPPLE_DATABASE_URL")?;
    if !sqlite_path.is_file() {
        anyhow::bail!("SQLite 文件不存在: {}", sqlite_path.display());
    }

    let sqlite_options =
        SqliteConnectOptions::from_str(&format!("sqlite://{}", sqlite_path.display()))?
            .read_only(true);
    let sqlite = SqlitePool::connect_with(sqlite_options).await?;
    let postgres = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&postgres).await?;

    ensure_empty_target(&postgres).await?;
    let mut tx = postgres.begin().await?;

    for row in sqlx::query("SELECT id, email, password_hash, created_at FROM users")
        .fetch_all(&sqlite)
        .await?
    {
        sqlx::query(
            "INSERT INTO users(id, email, password_hash, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(row.get::<String, _>("id"))
        .bind(row.get::<String, _>("email"))
        .bind(row.get::<String, _>("password_hash"))
        .bind(row.get::<f64, _>("created_at"))
        .execute(&mut *tx)
        .await?;
    }

    for row in sqlx::query(
        "SELECT code_hash, created_at, used_by, used_at, max_uses, use_count, expires_at
         FROM invitation_codes",
    )
    .fetch_all(&sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO invitation_codes(
                code_hash, created_at, used_by, used_at, max_uses, use_count, expires_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(row.get::<String, _>("code_hash"))
        .bind(row.get::<f64, _>("created_at"))
        .bind(row.get::<Option<String>, _>("used_by"))
        .bind(row.get::<Option<f64>, _>("used_at"))
        .bind(row.get::<i64, _>("max_uses"))
        .bind(row.get::<i64, _>("use_count"))
        .bind(row.get::<Option<f64>, _>("expires_at"))
        .execute(&mut *tx)
        .await?;
    }

    copy_simple_auth_tables(&sqlite, &mut tx).await?;

    for row in sqlx::query(
        "SELECT id, user_id, title, created_at, updated_at, is_pinned, archived_at
         FROM conversations",
    )
    .fetch_all(&sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO conversations(
                id, user_id, project_id, title, created_at, updated_at, is_pinned, archived_at
             ) VALUES ($1, $2, NULL, $3, $4, $5, $6, $7)",
        )
        .bind(row.get::<String, _>("id"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<String, _>("title"))
        .bind(row.get::<f64, _>("created_at"))
        .bind(row.get::<f64, _>("updated_at"))
        .bind(row.get::<i64, _>("is_pinned"))
        .bind(row.get::<Option<f64>, _>("archived_at"))
        .execute(&mut *tx)
        .await?;
    }

    copy_sessions_events_turns(&sqlite, &mut tx).await?;
    copy_assets_and_memories(&sqlite, &mut tx).await?;
    copy_todos_and_attachments(&sqlite, &mut tx).await?;
    tx.commit().await?;

    reset_identity(&postgres, "events").await?;
    reset_identity(&postgres, "turns").await?;
    reset_identity(&postgres, "memories").await?;
    validate_counts(&sqlite, &postgres).await?;
    println!("SQLite -> PostgreSQL migration completed and row counts match");
    Ok(())
}

async fn ensure_empty_target(postgres: &PgPool) -> anyhow::Result<()> {
    for table in [
        "users",
        "conversations",
        "turns",
        "memory_items",
        "assets",
        "todos",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(postgres)
            .await?;
        if count != 0 {
            anyhow::bail!("目标 PostgreSQL 表 {table} 非空，拒绝覆盖");
        }
    }
    Ok(())
}

async fn copy_simple_auth_tables(
    sqlite: &SqlitePool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    for row in sqlx::query("SELECT code_hash, user_id, used_at FROM invitation_redemptions")
        .fetch_all(sqlite)
        .await?
    {
        sqlx::query(
            "INSERT INTO invitation_redemptions(code_hash, user_id, used_at)
             VALUES ($1, $2, $3)",
        )
        .bind(row.get::<String, _>("code_hash"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<f64, _>("used_at"))
        .execute(&mut **tx)
        .await?;
    }
    for row in sqlx::query(
        "SELECT token_hash, user_id, created_at, expires_at, revoked_at FROM auth_sessions",
    )
    .fetch_all(sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO auth_sessions(token_hash, user_id, created_at, expires_at, revoked_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(row.get::<String, _>("token_hash"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<f64, _>("created_at"))
        .bind(row.get::<f64, _>("expires_at"))
        .bind(row.get::<Option<f64>, _>("revoked_at"))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn copy_sessions_events_turns(
    sqlite: &SqlitePool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    for row in sqlx::query("SELECT id, created_at, updated_at FROM sessions")
        .fetch_all(sqlite)
        .await?
    {
        sqlx::query("INSERT INTO sessions(id, created_at, updated_at) VALUES ($1, $2, $3)")
            .bind(row.get::<String, _>("id"))
            .bind(row.get::<f64, _>("created_at"))
            .bind(row.get::<f64, _>("updated_at"))
            .execute(&mut **tx)
            .await?;
    }
    let event_rows = sqlx::query("SELECT id, session_id, kind, payload, created_at FROM events")
        .fetch_all(sqlite)
        .await?;
    for rows in event_rows.chunks(500) {
        let mut insert = QueryBuilder::<Postgres>::new(
            "INSERT INTO events(id, session_id, kind, payload, created_at) ",
        );
        insert.push_values(rows, |mut values, row| {
            values
                .push_bind(row.get::<i64, _>("id"))
                .push_bind(row.get::<String, _>("session_id"))
                .push_bind(row.get::<String, _>("kind"))
                .push_bind(row.get::<String, _>("payload"))
                .push_bind(row.get::<f64, _>("created_at"));
        });
        insert.build().execute(&mut **tx).await?;
    }
    for row in sqlx::query("SELECT id, session_id, role, content, metadata, created_at FROM turns")
        .fetch_all(sqlite)
        .await?
    {
        sqlx::query(
            "INSERT INTO turns(id, session_id, role, content, metadata, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(row.get::<i64, _>("id"))
        .bind(row.get::<String, _>("session_id"))
        .bind(row.get::<String, _>("role"))
        .bind(row.get::<String, _>("content"))
        .bind(row.get::<String, _>("metadata"))
        .bind(row.get::<f64, _>("created_at"))
        .execute(&mut **tx)
        .await?;
    }
    for row in sqlx::query("SELECT id, session_id, content, created_at FROM memories")
        .fetch_all(sqlite)
        .await?
    {
        sqlx::query(
            "INSERT INTO memories(id, session_id, content, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(row.get::<i64, _>("id"))
        .bind(row.get::<String, _>("session_id"))
        .bind(row.get::<String, _>("content"))
        .bind(row.get::<f64, _>("created_at"))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn copy_assets_and_memories(
    sqlite: &SqlitePool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    for row in sqlx::query(
        "SELECT id, user_id, sha256, mime_type, storage_key, width, height,
                size_bytes, captured_at, created_at FROM assets",
    )
    .fetch_all(sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO assets(
                id, user_id, sha256, mime_type, storage_key, width, height,
                size_bytes, captured_at, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(row.get::<String, _>("id"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<String, _>("sha256"))
        .bind(row.get::<String, _>("mime_type"))
        .bind(row.get::<String, _>("storage_key"))
        .bind(row.get::<i64, _>("width"))
        .bind(row.get::<i64, _>("height"))
        .bind(row.get::<i64, _>("size_bytes"))
        .bind(row.get::<Option<f64>, _>("captured_at"))
        .bind(row.get::<f64, _>("created_at"))
        .execute(&mut **tx)
        .await?;
    }
    for row in sqlx::query(
        "SELECT id, user_id, conversation_id, source_turn_id, source_response_id,
                kind, user_note, visual_summary, cover_asset_id, captured_at,
                created_at, updated_at, is_pinned, archived_at FROM memory_items",
    )
    .fetch_all(sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO memory_items(
                id, user_id, scope_type, project_id, conversation_id, source_turn_id,
                source_response_id, kind, user_note, visual_summary, cover_asset_id,
                captured_at, created_at, updated_at, is_pinned, archived_at
             ) VALUES ($1, $2, 'personal', NULL, $3, $4, $5, $6, $7, $8,
                       $9, $10, $11, $12, $13, $14)",
        )
        .bind(row.get::<String, _>("id"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<Option<String>, _>("conversation_id"))
        .bind(row.get::<Option<i64>, _>("source_turn_id"))
        .bind(row.get::<String, _>("source_response_id"))
        .bind(row.get::<String, _>("kind"))
        .bind(row.get::<String, _>("user_note"))
        .bind(row.get::<String, _>("visual_summary"))
        .bind(row.get::<Option<String>, _>("cover_asset_id"))
        .bind(row.get::<Option<f64>, _>("captured_at"))
        .bind(row.get::<f64, _>("created_at"))
        .bind(row.get::<f64, _>("updated_at"))
        .bind(row.get::<i64, _>("is_pinned"))
        .bind(row.get::<Option<f64>, _>("archived_at"))
        .execute(&mut **tx)
        .await?;
    }
    copy_relation(sqlite, tx, "memory_assets").await?;
    copy_relation(sqlite, tx, "memory_tool_executions").await?;
    Ok(())
}

async fn copy_todos_and_attachments(
    sqlite: &SqlitePool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    for row in sqlx::query(
        "SELECT id, user_id, memory_id, conversation_id, source_turn_id,
                source_response_id, title, visual_summary, cover_asset_id,
                due_at, completed_at, created_at, updated_at FROM todos",
    )
    .fetch_all(sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO todos(
                id, user_id, project_id, memory_id, conversation_id, source_turn_id,
                source_response_id, title, visual_summary, cover_asset_id,
                due_at, completed_at, created_at, updated_at
             ) VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, $9,
                       $10, $11, $12, $13)",
        )
        .bind(row.get::<String, _>("id"))
        .bind(row.get::<String, _>("user_id"))
        .bind(row.get::<Option<String>, _>("memory_id"))
        .bind(row.get::<Option<String>, _>("conversation_id"))
        .bind(row.get::<Option<i64>, _>("source_turn_id"))
        .bind(row.get::<Option<String>, _>("source_response_id"))
        .bind(row.get::<String, _>("title"))
        .bind(row.get::<String, _>("visual_summary"))
        .bind(row.get::<Option<String>, _>("cover_asset_id"))
        .bind(row.get::<Option<f64>, _>("due_at"))
        .bind(row.get::<Option<f64>, _>("completed_at"))
        .bind(row.get::<f64, _>("created_at"))
        .bind(row.get::<f64, _>("updated_at"))
        .execute(&mut **tx)
        .await?;
    }
    for row in sqlx::query(
        "SELECT turn_id, asset_id, memory_id, todo_id, caption, ordinal FROM turn_attachments",
    )
    .fetch_all(sqlite)
    .await?
    {
        sqlx::query(
            "INSERT INTO turn_attachments(
                turn_id, asset_id, memory_id, todo_id, caption, ordinal
             ) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(row.get::<i64, _>("turn_id"))
        .bind(row.get::<String, _>("asset_id"))
        .bind(row.get::<Option<String>, _>("memory_id"))
        .bind(row.get::<Option<String>, _>("todo_id"))
        .bind(row.get::<String, _>("caption"))
        .bind(row.get::<i64, _>("ordinal"))
        .execute(&mut **tx)
        .await?;
    }
    copy_relation(sqlite, tx, "todo_tool_executions").await?;
    Ok(())
}

async fn copy_relation(
    sqlite: &SqlitePool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
) -> anyhow::Result<()> {
    match table {
        "memory_assets" => {
            for row in
                sqlx::query("SELECT memory_id, asset_id, ordinal, is_cover FROM memory_assets")
                    .fetch_all(sqlite)
                    .await?
            {
                sqlx::query(
                    "INSERT INTO memory_assets(memory_id, asset_id, ordinal, is_cover)
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(row.get::<String, _>("memory_id"))
                .bind(row.get::<String, _>("asset_id"))
                .bind(row.get::<i64, _>("ordinal"))
                .bind(row.get::<i64, _>("is_cover"))
                .execute(&mut **tx)
                .await?;
            }
        }
        "memory_tool_executions" => {
            for row in sqlx::query(
                "SELECT response_id, tool_call_id, memory_id FROM memory_tool_executions",
            )
            .fetch_all(sqlite)
            .await?
            {
                sqlx::query(
                    "INSERT INTO memory_tool_executions(response_id, tool_call_id, memory_id)
                     VALUES ($1, $2, $3)",
                )
                .bind(row.get::<String, _>("response_id"))
                .bind(row.get::<String, _>("tool_call_id"))
                .bind(row.get::<String, _>("memory_id"))
                .execute(&mut **tx)
                .await?;
            }
        }
        "todo_tool_executions" => {
            for row in
                sqlx::query("SELECT response_id, tool_call_id, todo_id FROM todo_tool_executions")
                    .fetch_all(sqlite)
                    .await?
            {
                sqlx::query(
                    "INSERT INTO todo_tool_executions(response_id, tool_call_id, todo_id)
                     VALUES ($1, $2, $3)",
                )
                .bind(row.get::<String, _>("response_id"))
                .bind(row.get::<String, _>("tool_call_id"))
                .bind(row.get::<String, _>("todo_id"))
                .execute(&mut **tx)
                .await?;
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

async fn reset_identity(postgres: &PgPool, table: &str) -> anyhow::Result<()> {
    sqlx::query(&format!(
        "SELECT setval(pg_get_serial_sequence('{table}', 'id'),
            GREATEST(COALESCE((SELECT MAX(id) FROM {table}), 0), 1),
            EXISTS(SELECT 1 FROM {table}))"
    ))
    .execute(postgres)
    .await?;
    Ok(())
}

async fn validate_counts(sqlite: &SqlitePool, postgres: &PgPool) -> anyhow::Result<()> {
    for table in [
        "users",
        "conversations",
        "sessions",
        "events",
        "turns",
        "memories",
        "memory_items",
        "assets",
        "memory_assets",
        "turn_attachments",
        "todos",
    ] {
        let source: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(sqlite)
            .await?;
        let target: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(postgres)
            .await?;
        if source != target {
            anyhow::bail!("表 {table} 数量不一致: sqlite={source}, postgres={target}");
        }
        println!("{table}: {target}");
    }
    Ok(())
}
