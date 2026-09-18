use langfuse_core::{Result, TraceRecord};
use sqlx::{PgPool, Row};

/// UPSERT trace
pub async fn upsert_trace(pool: &PgPool, record: &TraceRecord) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO traces (
            id, project_id, external_id, timestamp, name, user_id,
            metadata, release, version, public, bookmarked,
            tags, input, output, session_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT (id) DO UPDATE SET
            timestamp = EXCLUDED.timestamp,
            name = COALESCE(EXCLUDED.name, traces.name),
            user_id = COALESCE(EXCLUDED.user_id, traces.user_id),
            metadata = COALESCE(EXCLUDED.metadata, traces.metadata),
            release = COALESCE(EXCLUDED.release, traces.release),
            version = COALESCE(EXCLUDED.version, traces.version),
            public = COALESCE(EXCLUDED.public, traces.public),
            bookmarked = COALESCE(EXCLUDED.bookmarked, traces.bookmarked),
            tags = COALESCE(EXCLUDED.tags, traces.tags),
            input = COALESCE(EXCLUDED.input, traces.input),
            output = COALESCE(EXCLUDED.output, traces.output),
            session_id = COALESCE(EXCLUDED.session_id, traces.session_id),
            updated_at = now()"#
    )
    .bind(&record.id)
    .bind(&record.project_id)
    .bind(&record.external_id)
    .bind(record.timestamp)
    .bind(&record.name)
    .bind(&record.user_id)
    .bind(&record.metadata)
    .bind(&record.release)
    .bind(&record.version)
    .bind(record.public)
    .bind(record.bookmarked)
    .bind(&record.tags)
    .bind(&record.input)
    .bind(&record.output)
    .bind(&record.session_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Insert a trace row only if it does not exist yet, never touching an existing
/// row.
///
/// Used to keep `traces` consistent when an observation arrives without its
/// root span: OTLP batches are per-process and NOT ordered, so LexQA's HTTP
/// root span and the asynq worker spans it parents can land in either order,
/// and a trace list that reads only `traces` would hide the whole tree until
/// the root eventually shows up. `upsert_trace` is the opposite semantic — it
/// overwrites `timestamp` — so it cannot be reused here.
pub async fn ensure_trace(pool: &PgPool, record: &TraceRecord) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO traces (
            id, project_id, external_id, timestamp, name, user_id,
            metadata, release, version, public, bookmarked,
            tags, input, output, session_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(&record.id)
    .bind(&record.project_id)
    .bind(&record.external_id)
    .bind(record.timestamp)
    .bind(&record.name)
    .bind(&record.user_id)
    .bind(&record.metadata)
    .bind(&record.release)
    .bind(&record.version)
    .bind(record.public)
    .bind(record.bookmarked)
    .bind(&record.tags)
    .bind(&record.input)
    .bind(&record.output)
    .bind(&record.session_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert trace session
/// Upsert trace session — see [`crate::repos::sessions::upsert_session`].
///
/// Re-exported rather than reimplemented: this module used to carry a second
/// copy that bound `environment` as NULL and so violated the NOT NULL
/// constraint on `trace_sessions`.
pub use crate::repos::sessions::upsert_session;

// ============================================================================
// Read queries
// ============================================================================

/// Trace 行（从 PG 读取的原始数据）
#[derive(Debug, Clone)]
pub struct TraceRow {
    pub id: String,
    pub project_id: String,
    pub external_id: Option<String>,
    pub timestamp: chrono::NaiveDateTime,
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub release: Option<String>,
    pub version: Option<String>,
    pub public: bool,
    pub bookmarked: bool,
    pub tags: Vec<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub session_id: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

/// 根据 ID 查找 trace
/// 注: traces 表无 environment 列 (environment 存储在 trace_sessions 中)
pub async fn find_by_id(pool: &PgPool, id: &str, project_id: &str) -> Result<Option<TraceRow>> {
    let row = sqlx::query(
        r#"SELECT id, project_id, external_id, timestamp, name, user_id,
           metadata, release, version, public, bookmarked,
           tags, input, output, session_id, created_at, updated_at
        FROM traces
        WHERE id = $1 AND project_id = $2"#
    )
    .bind(id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r: sqlx::postgres::PgRow| TraceRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        external_id: r.get("external_id"),
        timestamp: r.get("timestamp"),
        name: r.get("name"),
        user_id: r.get("user_id"),
        metadata: r.get("metadata"),
        release: r.get("release"),
        version: r.get("version"),
        public: r.get("public"),
        bookmarked: r.get("bookmarked"),
        tags: r.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
        input: r.get("input"),
        output: r.get("output"),
        session_id: r.get("session_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }))
}

/// 列出 traces（带分页 + 排序）
///
/// `cursor` 是上一页最后一条 trace 的 timestamp（作为游标）
/// `limit` 是每页大小，默认 50
pub async fn list_traces(
    pool: &PgPool,
    project_id: &str,
    cursor: Option<chrono::NaiveDateTime>,
    limit: i64,
) -> Result<Vec<TraceRow>> {
    let rows = if let Some(cursor_ts) = cursor {
        sqlx::query(
            r#"SELECT id, project_id, external_id, timestamp, name, user_id,
               metadata, release, version, public, bookmarked,
               tags, input, output, session_id, created_at, updated_at
            FROM traces
            WHERE project_id = $1 AND timestamp < $2
            ORDER BY timestamp DESC
            LIMIT $3"#
        )
        .bind(project_id)
        .bind(cursor_ts)
        .bind(limit + 1) // 多取一条用于判断 has_more
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"SELECT id, project_id, external_id, timestamp, name, user_id,
               metadata, release, version, public, bookmarked,
               tags, input, output, session_id, created_at, updated_at
            FROM traces
            WHERE project_id = $1
            ORDER BY timestamp DESC
            LIMIT $2"#
        )
        .bind(project_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(|r: sqlx::postgres::PgRow| TraceRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        external_id: r.get("external_id"),
        timestamp: r.get("timestamp"),
        name: r.get("name"),
        user_id: r.get("user_id"),
        metadata: r.get("metadata"),
        release: r.get("release"),
        version: r.get("version"),
        public: r.get("public"),
        bookmarked: r.get("bookmarked"),
        tags: r.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
        input: r.get("input"),
        output: r.get("output"),
        session_id: r.get("session_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }).collect())
}

/// 统计 project 的 trace 总数
pub async fn count_traces(pool: &PgPool, project_id: &str) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM traces WHERE project_id = $1")
        .bind(project_id)
        .fetch_one(pool)
        .await?;

    Ok(row.get::<i64, _>("cnt"))
}

// ============================================================================
// Filtered listing for the public API
// ============================================================================

const TRACE_SELECT: &str = r#"SELECT id, project_id, external_id, timestamp, name, user_id,
    metadata, release, version, public, bookmarked,
    tags, input, output, session_id, created_at, updated_at
FROM traces"#;

/// Columns the public API may sort by — the `orderBy` field whitelist from the
/// Langfuse contract (`id`, `timestamp`, `name`, `userId`, `release`,
/// `version`, `public`, `bookmarked`, `sessionId`).
///
/// A whitelist rather than free text: the sort column is the one part of the
/// query that cannot be a bound parameter, so anything not listed here has to be
/// rejected instead of interpolated.
const TRACE_ORDER_COLUMNS: &[&str] = &[
    "id",
    "timestamp",
    "name",
    "user_id",
    "release",
    "version",
    "public",
    "bookmarked",
    "session_id",
];

/// Filter set for the public trace list endpoint.
#[derive(Debug, Clone)]
pub struct TraceListFilter {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub tags: Vec<String>,
    pub version: Option<String>,
    pub release: Option<String>,
    pub from_timestamp: Option<chrono::NaiveDateTime>,
    pub to_timestamp: Option<chrono::NaiveDateTime>,
    /// `(column, ascending)`; defaults to `timestamp DESC`.
    pub order_by: Option<(String, bool)>,
    pub page: i64,
    pub limit: i64,
}

impl Default for TraceListFilter {
    fn default() -> Self {
        Self {
            user_id: None,
            session_id: None,
            name: None,
            tags: Vec::new(),
            version: None,
            release: None,
            from_timestamp: None,
            to_timestamp: None,
            order_by: None,
            page: 1,
            limit: 50,
        }
    }
}

/// Append the shared WHERE clauses. Kept in one place so the page query and the
/// count query can never disagree about what the filter means.
fn push_trace_filters(qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>, filter: &TraceListFilter) {
    if let Some(user_id) = &filter.user_id {
        qb.push(" AND user_id = ").push_bind(user_id.clone());
    }
    if let Some(session_id) = &filter.session_id {
        qb.push(" AND session_id = ").push_bind(session_id.clone());
    }
    if let Some(name) = &filter.name {
        qb.push(" AND name = ").push_bind(name.clone());
    }
    if let Some(version) = &filter.version {
        qb.push(" AND version = ").push_bind(version.clone());
    }
    if let Some(release) = &filter.release {
        qb.push(" AND release = ").push_bind(release.clone());
    }
    if !filter.tags.is_empty() {
        // The contract is "only traces that include ALL of these tags", which is
        // array containment, not overlap.
        qb.push(" AND tags @> ").push_bind(filter.tags.clone());
    }
    if let Some(from) = filter.from_timestamp {
        qb.push(" AND timestamp >= ").push_bind(from);
    }
    if let Some(to) = filter.to_timestamp {
        // Exclusive: the contract reads "before a certain datetime".
        qb.push(" AND timestamp < ").push_bind(to);
    }
}

/// List traces with filters and page/limit pagination, plus the total count for
/// the official `meta.totalItems`.
pub async fn list_traces_filtered(
    pool: &PgPool,
    project_id: &str,
    filter: &TraceListFilter,
) -> Result<(Vec<TraceRow>, i64)> {
    let limit = filter.limit.clamp(1, 100);
    let offset = (filter.page.max(1) - 1) * limit;

    let mut qb = sqlx::QueryBuilder::new(TRACE_SELECT);
    qb.push(" WHERE project_id = ").push_bind(project_id.to_string());
    push_trace_filters(&mut qb, filter);

    qb.push(" ORDER BY ");
    match &filter.order_by {
        Some((column, ascending)) if TRACE_ORDER_COLUMNS.contains(&column.as_str()) => {
            qb.push(column.clone())
                .push(if *ascending { " ASC" } else { " DESC" });
        }
        _ => {
            qb.push("timestamp DESC");
        }
    }
    qb.push(" LIMIT ").push_bind(limit).push(" OFFSET ").push_bind(offset);

    let rows = qb.build().fetch_all(pool).await?;
    let traces: Vec<TraceRow> = rows.iter().map(map_trace_row).collect();

    let mut count_qb = sqlx::QueryBuilder::new("SELECT COUNT(*) AS cnt FROM traces");
    count_qb
        .push(" WHERE project_id = ")
        .push_bind(project_id.to_string());
    push_trace_filters(&mut count_qb, filter);

    let total: i64 = count_qb
        .build()
        .fetch_one(pool)
        .await?
        .get::<i64, _>("cnt");

    Ok((traces, total))
}

fn map_trace_row(r: &sqlx::postgres::PgRow) -> TraceRow {
    TraceRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        external_id: r.get("external_id"),
        timestamp: r.get("timestamp"),
        name: r.get("name"),
        user_id: r.get("user_id"),
        metadata: r.get("metadata"),
        release: r.get("release"),
        version: r.get("version"),
        public: r.get("public"),
        bookmarked: r.get("bookmarked"),
        tags: r.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
        input: r.get("input"),
        output: r.get("output"),
        session_id: r.get("session_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// Per-trace roll-ups for the traces table's metric columns.
///
/// The table joins these onto the core trace rows by `id`, so a trace with no
/// observations still needs an entry — otherwise its latency/tokens/cost cells
/// stay in their loading state forever.
#[derive(Debug, Clone, Default)]
pub struct TraceMetricRow {
    pub id: String,
    /// `max(end_time) - min(start_time)` of the trace's observations, seconds.
    pub latency: f64,
    pub error_count: i64,
    pub warning_count: i64,
    pub debug_count: i64,
    pub default_count: i64,
    pub input_usage: i64,
    pub output_usage: i64,
    pub total_usage: i64,
    /// `usage_details` summed across the trace's observations.
    pub token_details: std::collections::BTreeMap<String, i64>,
    pub input_cost: rust_decimal::Decimal,
    pub output_cost: rust_decimal::Decimal,
    pub total_cost: rust_decimal::Decimal,
    /// `cost_details` summed across the trace's observations.
    pub cost_details: std::collections::BTreeMap<String, f64>,
}

/// Aggregate per-trace metrics for the given traces.
pub async fn trace_metrics(
    pool: &PgPool,
    project_id: &str,
    trace_ids: &[String],
) -> Result<Vec<TraceMetricRow>> {
    use std::collections::HashMap;

    if trace_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut metrics: HashMap<String, TraceMetricRow> = trace_ids
        .iter()
        .map(|id| {
            (
                id.clone(),
                TraceMetricRow {
                    id: id.clone(),
                    ..Default::default()
                },
            )
        })
        .collect();

    // Scalars: latency, level counts, token totals, cost total.
    for row in sqlx::query(
        r#"SELECT trace_id,
                  COALESCE(
                      EXTRACT(EPOCH FROM (MAX(end_time) - MIN(start_time))),
                      0
                  )::double precision AS latency,
                  COUNT(*) FILTER (WHERE level = 'ERROR') AS error_count,
                  COUNT(*) FILTER (WHERE level = 'WARNING') AS warning_count,
                  COUNT(*) FILTER (WHERE level = 'DEBUG') AS debug_count,
                  COUNT(*) FILTER (WHERE level = 'DEFAULT') AS default_count,
                  COALESCE(SUM(prompt_tokens), 0)::bigint AS input_usage,
                  COALESCE(SUM(completion_tokens), 0)::bigint AS output_usage,
                  COALESCE(SUM(total_tokens), 0)::bigint AS total_usage,
                  COALESCE(SUM(COALESCE(input_cost, calculated_input_cost, 0)), 0) AS input_cost,
                  COALESCE(SUM(COALESCE(output_cost, calculated_output_cost, 0)), 0) AS output_cost,
                  COALESCE(SUM(COALESCE(total_cost, calculated_total_cost, 0)), 0) AS total_cost
           FROM observations
           WHERE project_id = $1 AND trace_id = ANY($2::text[])
           GROUP BY trace_id"#,
    )
    .bind(project_id)
    .bind(trace_ids)
    .fetch_all(pool)
    .await?
    {
        let id: String = row.get("trace_id");
        if let Some(metric) = metrics.get_mut(&id) {
            metric.latency = row.get("latency");
            metric.error_count = row.get("error_count");
            metric.warning_count = row.get("warning_count");
            metric.debug_count = row.get("debug_count");
            metric.default_count = row.get("default_count");
            metric.input_usage = row.get("input_usage");
            metric.output_usage = row.get("output_usage");
            metric.total_usage = row.get("total_usage");
            metric.input_cost = row.get("input_cost");
            metric.output_cost = row.get("output_cost");
            metric.total_cost = row.get("total_cost");
        }
    }

    // The jsonb breakdowns are summed per usage type rather than in SQL: the key
    // sets differ between observations (one may report cache reads, another may
    // not), and `jsonb_object_agg` cannot add the overlapping values.
    for row in sqlx::query(
        r#"SELECT trace_id, usage_details, cost_details
           FROM observations
           WHERE project_id = $1 AND trace_id = ANY($2::text[])
             AND (usage_details IS NOT NULL OR cost_details IS NOT NULL)"#,
    )
    .bind(project_id)
    .bind(trace_ids)
    .fetch_all(pool)
    .await?
    {
        let id: String = row.get("trace_id");
        let Some(metric) = metrics.get_mut(&id) else {
            continue;
        };

        if let Some(usage) = row
            .try_get::<Option<serde_json::Value>, _>("usage_details")?
            .and_then(|v| v.as_object().cloned())
        {
            for (usage_type, value) in usage {
                if let Some(n) = json_int(&value) {
                    *metric.token_details.entry(usage_type).or_default() += n;
                }
            }
        }

        if let Some(cost) = row
            .try_get::<Option<serde_json::Value>, _>("cost_details")?
            .and_then(|v| v.as_object().cloned())
        {
            for (usage_type, value) in cost {
                if let Some(n) = json_float(&value) {
                    *metric.cost_details.entry(usage_type).or_default() += n;
                }
            }
        }
    }

    Ok(trace_ids
        .iter()
        .filter_map(|id| metrics.remove(id))
        .collect())
}

fn json_int(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn json_float(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}
