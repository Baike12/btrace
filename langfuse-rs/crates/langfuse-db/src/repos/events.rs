use langfuse_core::Result;
use sqlx::PgPool;

/// 插入事件记录到 events 表（可选）
pub async fn insert_event(
    pool: &PgPool,
    id: &str,
    project_id: &str,
    event_type: &str,
    body: &serde_json::Value,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO events (id, project_id, type, body, timestamp)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (id, project_id) DO NOTHING"#,
    )
    .bind(id)
    .bind(project_id)
    .bind(event_type)
    .bind(body)
    .bind(timestamp)
    .execute(pool)
    .await?;

    Ok(())
}
