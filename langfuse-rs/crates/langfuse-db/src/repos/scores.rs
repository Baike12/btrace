use langfuse_core::{Result, ScoreRecord};
use sqlx::{PgPool, Row};

/// UPSERT score — Phase 1: worker 写操作
pub async fn upsert_score(pool: &PgPool, record: &ScoreRecord) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO scores (
            id, project_id, timestamp, name, value, source,
            author_user_id, comment, trace_id, observation_id,
            config_id, string_value, queue_id, data_type
        ) VALUES ($1, $2, $3, $4, $5, $6::"ScoreSource", $7, $8, $9, $10, $11, $12, $13, $14::"ScoreConfigDataType")
        ON CONFLICT (id, project_id) DO UPDATE SET
            timestamp = EXCLUDED.timestamp, name = EXCLUDED.name,
            value = COALESCE(EXCLUDED.value, scores.value), source = EXCLUDED.source,
            author_user_id = COALESCE(EXCLUDED.author_user_id, scores.author_user_id),
            comment = COALESCE(EXCLUDED.comment, scores.comment),
            trace_id = COALESCE(EXCLUDED.trace_id, scores.trace_id),
            observation_id = COALESCE(EXCLUDED.observation_id, scores.observation_id),
            config_id = COALESCE(EXCLUDED.config_id, scores.config_id),
            string_value = COALESCE(EXCLUDED.string_value, scores.string_value),
            queue_id = COALESCE(EXCLUDED.queue_id, scores.queue_id),
            data_type = COALESCE(EXCLUDED.data_type, scores.data_type),
            updated_at = now()"#
    )
    .bind(&record.id)
    .bind(&record.project_id)
    .bind(record.timestamp)
    .bind(&record.name)
    .bind(record.value)
    .bind(record.source.to_string())
    .bind(&record.author_user_id)
    .bind(&record.comment)
    .bind(&record.trace_id)
    .bind(&record.observation_id)
    .bind(&record.config_id)
    .bind(&record.string_value)
    .bind(&record.queue_id)
    .bind(record.data_type.as_ref().map(|dt| dt.to_string()))
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Read queries
// ============================================================================

#[derive(Debug, Clone)]
pub struct ScoreRow {
    pub id: String,
    pub project_id: String,
    pub timestamp: chrono::NaiveDateTime,
    pub name: Option<String>,
    pub value: Option<f64>,
    pub source: String,           // PG ENUM → text
    pub author_user_id: Option<String>,
    pub comment: Option<String>,
    pub trace_id: Option<String>,
    pub observation_id: Option<String>,
    pub config_id: Option<String>,
    pub string_value: Option<String>,
    pub queue_id: Option<String>,
    pub data_type: String,        // PG ENUM → text
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

/// 根据 trace_id 查找所有 scores
pub async fn find_by_trace_id(
    pool: &PgPool,
    trace_id: &str,
    project_id: &str,
) -> Result<Vec<ScoreRow>> {
    let rows = sqlx::query(
        r#"SELECT id, project_id, timestamp, name, value,
           source::text as source, author_user_id, comment,
           trace_id, observation_id, config_id, string_value,
           queue_id, data_type::text as data_type, created_at, updated_at
        FROM scores
        WHERE trace_id = $1 AND project_id = $2
        ORDER BY timestamp DESC"#
    )
    .bind(trace_id)
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_row).collect())
}

/// 列出 scores（带分页）
pub async fn list_scores(
    pool: &PgPool,
    project_id: &str,
    trace_id: Option<&str>,
    cursor: Option<chrono::NaiveDateTime>,
    limit: i64,
) -> Result<Vec<ScoreRow>> {
    let base = r#"SELECT id, project_id, timestamp, name, value,
        source::text as source, author_user_id, comment,
        trace_id, observation_id, config_id, string_value,
        queue_id, data_type::text as data_type, created_at, updated_at
    FROM scores
    WHERE project_id = $1"#;

    let mut extra = String::new();
    let mut idx = 2i32;

    if trace_id.is_some() {
        extra.push_str(&format!(" AND trace_id = ${}", idx));
        idx += 1;
    }
    if cursor.is_some() {
        extra.push_str(&format!(" AND timestamp < ${}", idx));
        idx += 1;
    }

    let full = format!(
        "{} {} ORDER BY timestamp DESC LIMIT ${}",
        base, extra, idx
    );

    let mut query = sqlx::query(&full).bind(project_id);
    if let Some(tid) = trace_id { query = query.bind(tid); }
    if let Some(c) = cursor { query = query.bind(c); }

    let rows = query.bind(limit + 1).fetch_all(pool).await?;
    Ok(rows.into_iter().map(map_row).collect())
}

fn map_row(r: sqlx::postgres::PgRow) -> ScoreRow {
    ScoreRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        timestamp: r.get("timestamp"),
        name: r.get("name"),
        value: r.get("value"),
        source: r.get("source"),
        author_user_id: r.get("author_user_id"),
        comment: r.get("comment"),
        trace_id: r.get("trace_id"),
        observation_id: r.get("observation_id"),
        config_id: r.get("config_id"),
        string_value: r.get("string_value"),
        queue_id: r.get("queue_id"),
        data_type: r.get("data_type"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}
