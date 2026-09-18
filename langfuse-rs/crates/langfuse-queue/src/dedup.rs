use langfuse_core::Result;
use sqlx::PgPool;

/// 检查并标记已处理事件，防止重复处理
/// Key 格式：{projectId}:{eventType}:{eventBodyId}:{fileKey}
/// 返回 true 表示已处理过（重复），false 表示是新事件
pub async fn check_and_mark_seen(
    pool: &PgPool,
    project_id: &str,
    event_type: &str,
    event_body_id: &str,
    file_key: &str,
) -> Result<bool> {
    let event_key = format!("{}:{}:{}:{}", project_id, event_type, event_body_id, file_key);

    let result = sqlx::query(
        r#"INSERT INTO pg_seen_events (event_key, created_at)
        VALUES ($1, now())
        ON CONFLICT (event_key) DO NOTHING"#,
    )
    .bind(&event_key)
    .execute(pool)
    .await?;

    // rows_affected=0 说明冲突了，即已处理过（duplicate）
    Ok(result.rows_affected() == 0)
}

/// 清理过期的已处理事件记录
pub async fn clean_seen_events(pool: &PgPool) -> Result<u64> {
    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(10);
    let result = sqlx::query("DELETE FROM pg_seen_events WHERE created_at < $1")
        .bind(cutoff)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}
