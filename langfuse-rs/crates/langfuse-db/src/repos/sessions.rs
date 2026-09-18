use langfuse_core::Result;
use sqlx::{PgPool, Row};

/// 创建或更新 trace session
///
/// `environment` is NOT NULL with no database default, and this fork's `traces`
/// table has no environment column to copy from — so an unset environment falls
/// back to `"default"` (Langfuse's convention for an unspecified environment)
/// rather than binding NULL and failing the insert.
pub async fn upsert_session(
    pool: &PgPool,
    session_id: &str,
    project_id: &str,
    environment: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO trace_sessions (id, project_id, environment)
        VALUES ($1, $2, $3)
        ON CONFLICT (id, project_id) DO UPDATE SET
            environment = COALESCE(EXCLUDED.environment, trace_sessions.environment),
            updated_at = now()"#,
    )
    .bind(session_id)
    .bind(project_id)
    .bind(environment.unwrap_or("default"))
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Read queries
// ============================================================================

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub project_id: String,
    pub environment: Option<String>,
    pub bookmarked: bool,
    pub public: bool,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

pub async fn list_sessions(
    pool: &PgPool,
    project_id: &str,
    limit: i64,
) -> Result<Vec<SessionRow>> {
    let rows = sqlx::query(
        r#"SELECT id, project_id, environment, bookmarked, public, created_at, updated_at
        FROM trace_sessions WHERE project_id = $1
        ORDER BY created_at DESC LIMIT $2"#
    )
    .bind(project_id)
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r: sqlx::postgres::PgRow| SessionRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        environment: r.get("environment"),
        bookmarked: r.get("bookmarked"),
        public: r.get("public"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }).collect())
}

/// Find one session by id.
pub async fn find_session(
    pool: &PgPool,
    project_id: &str,
    session_id: &str,
) -> Result<Option<SessionRow>> {
    let row = sqlx::query(
        r#"SELECT id, project_id, environment, bookmarked, public, created_at, updated_at
        FROM trace_sessions WHERE project_id = $1 AND id = $2"#,
    )
    .bind(project_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| SessionRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        environment: r.get("environment"),
        bookmarked: r.get("bookmarked"),
        public: r.get("public"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }))
}

/// List sessions with page/limit pagination and a time range, plus the total for
/// the public `meta.totalItems`.
///
/// A session is created when its first trace is ingested, so `created_at` is the
/// natural ordering key and time filter.
pub async fn list_sessions_filtered(
    pool: &PgPool,
    project_id: &str,
    from: Option<chrono::NaiveDateTime>,
    to: Option<chrono::NaiveDateTime>,
    limit: i64,
) -> Result<Vec<SessionRow>> {
    let rows = sqlx::query(
        r#"SELECT id, project_id, environment, bookmarked, public, created_at, updated_at
        FROM trace_sessions
        WHERE project_id = $1
          AND ($2::timestamp IS NULL OR created_at >= $2)
          AND ($3::timestamp IS NULL OR created_at < $3)
        ORDER BY created_at DESC
        LIMIT $4"#,
    )
    .bind(project_id)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SessionRow {
            id: r.get("id"),
            project_id: r.get("project_id"),
            environment: r.get("environment"),
            bookmarked: r.get("bookmarked"),
            public: r.get("public"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Count sessions matching the same time filter [`list_sessions_filtered`] uses.
pub async fn count_sessions(
    pool: &PgPool,
    project_id: &str,
    from: Option<chrono::NaiveDateTime>,
    to: Option<chrono::NaiveDateTime>,
) -> Result<i64> {
    let row = sqlx::query(
        r#"SELECT COUNT(*) AS cnt FROM trace_sessions
        WHERE project_id = $1
          AND ($2::timestamp IS NULL OR created_at >= $2)
          AND ($3::timestamp IS NULL OR created_at < $3)"#,
    )
    .bind(project_id)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    Ok(row.get::<i64, _>("cnt"))
}

/// Whether the project has any session at all.
///
/// The sessions page uses this to choose between its onboarding empty state and
/// the table, so it has to answer even when the project has no sessions — which
/// is why it is an `EXISTS` probe rather than a count of a page of rows.
pub async fn has_any(pool: &PgPool, project_id: &str) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM trace_sessions WHERE project_id = $1)",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

/// Per-session roll-ups for the sessions table's metric columns.
///
/// Upstream computed these in ClickHouse from materialised per-session columns
/// (`trace_count`, `duration`, `session_total_cost`, …). PostgreSQL has no such
/// rollup, so the same numbers are aggregated here from the `traces` and
/// `observations` rows the session owns — which is also why the cost totals sum
/// the stored columns instead of reading a precomputed map.
#[derive(Debug, Clone, Default)]
pub struct SessionMetricRow {
    pub id: String,
    pub min_timestamp: Option<chrono::NaiveDateTime>,
    pub trace_count: i64,
    pub total_observations: i64,
    pub user_ids: Vec<String>,
    pub trace_tags: Vec<String>,
    /// Wall-clock span of the session's observations, in seconds.
    pub duration: f64,
    pub input_cost: rust_decimal::Decimal,
    pub output_cost: rust_decimal::Decimal,
    pub total_cost: rust_decimal::Decimal,
    pub input_usage: i64,
    pub output_usage: i64,
    pub total_usage: i64,
}

/// Aggregate per-session metrics for the given sessions.
///
/// Split into three queries rather than one wide join: the trace-level counts
/// and the observation-level sums multiply against each other under a join, so
/// keeping them apart is what makes each aggregate mean what its name says.
pub async fn session_metrics(
    pool: &PgPool,
    project_id: &str,
    session_ids: &[String],
) -> Result<Vec<SessionMetricRow>> {
    use std::collections::HashMap;

    if session_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut metrics: HashMap<String, SessionMetricRow> = HashMap::new();

    // Trace-level: count, earliest timestamp, distinct user ids.
    for row in sqlx::query(
        r#"SELECT session_id AS id,
                  MIN(timestamp) AS min_timestamp,
                  COUNT(*) AS trace_count,
                  array_agg(DISTINCT user_id) FILTER (WHERE user_id IS NOT NULL) AS user_ids
           FROM traces
           WHERE project_id = $1 AND session_id = ANY($2::text[])
           GROUP BY session_id"#,
    )
    .bind(project_id)
    .bind(session_ids)
    .fetch_all(pool)
    .await?
    {
        let id: String = row.get("id");
        metrics.insert(
            id.clone(),
            SessionMetricRow {
                id,
                min_timestamp: row.get("min_timestamp"),
                trace_count: row.get("trace_count"),
                user_ids: row.try_get("user_ids").unwrap_or_default(),
                ..Default::default()
            },
        );
    }

    // Tags are an array column, so unnesting them is a separate aggregate.
    for row in sqlx::query(
        r#"SELECT t.session_id AS id, array_agg(DISTINCT tag) AS trace_tags
           FROM traces t, unnest(t.tags) AS tag
           WHERE t.project_id = $1 AND t.session_id = ANY($2::text[])
           GROUP BY t.session_id"#,
    )
    .bind(project_id)
    .bind(session_ids)
    .fetch_all(pool)
    .await?
    {
        let id: String = row.get("id");
        if let Some(metric) = metrics.get_mut(&id) {
            metric.trace_tags = row.try_get("trace_tags").unwrap_or_default();
        }
    }

    // Observation-level: counts, span, cost and token sums.
    for row in sqlx::query(
        r#"SELECT t.session_id AS id,
                  COUNT(o.id) AS total_observations,
                  COALESCE(
                      EXTRACT(EPOCH FROM (MAX(o.end_time) - MIN(o.start_time))),
                      0
                  )::double precision AS duration,
                  COALESCE(SUM(COALESCE(o.input_cost, o.calculated_input_cost, 0)), 0) AS input_cost,
                  COALESCE(SUM(COALESCE(o.output_cost, o.calculated_output_cost, 0)), 0) AS output_cost,
                  COALESCE(SUM(COALESCE(o.total_cost, o.calculated_total_cost, 0)), 0) AS total_cost,
                  COALESCE(SUM(o.prompt_tokens), 0)::bigint AS input_usage,
                  COALESCE(SUM(o.completion_tokens), 0)::bigint AS output_usage,
                  COALESCE(SUM(o.total_tokens), 0)::bigint AS total_usage
           FROM traces t
           JOIN observations o ON o.trace_id = t.id AND o.project_id = t.project_id
           WHERE t.project_id = $1 AND t.session_id = ANY($2::text[])
           GROUP BY t.session_id"#,
    )
    .bind(project_id)
    .bind(session_ids)
    .fetch_all(pool)
    .await?
    {
        let id: String = row.get("id");
        if let Some(metric) = metrics.get_mut(&id) {
            metric.total_observations = row.get("total_observations");
            metric.duration = row.get("duration");
            metric.input_cost = row.get("input_cost");
            metric.output_cost = row.get("output_cost");
            metric.total_cost = row.get("total_cost");
            metric.input_usage = row.get("input_usage");
            metric.output_usage = row.get("output_usage");
            metric.total_usage = row.get("total_usage");
        }
    }

    // Return in the caller's order. A session with no traces yet produces no
    // row here, and the table's join simply leaves that session's metric columns
    // blank — which is accurate, rather than a zeroed row implying it ran.
    Ok(session_ids
        .iter()
        .filter_map(|id| metrics.remove(id))
        .collect())
}
