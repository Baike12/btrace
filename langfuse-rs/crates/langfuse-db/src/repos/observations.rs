use langfuse_core::{ObservationRecord, Result};
use rust_decimal::Decimal;
use sqlx::PgPool;

/// The projection every read of an observation uses.
///
/// Kept in one place because the three queries below feed the same
/// [`ObservationRow`] mapper, and a column added to one list but not the others
/// yields a `Row::get` panic at runtime rather than a compile error.
const OBSERVATION_COLUMNS: &str = r#"id, trace_id, project_id, type::text as type,
    level::text as level, start_time, end_time, name, metadata,
    parent_observation_id, status_message, version, model, internal_model,
    internal_model_id, "modelParameters" as model_parameters, input, output,
    prompt_tokens, completion_tokens, total_tokens, unit,
    input_cost, output_cost, total_cost,
    calculated_input_cost, calculated_output_cost, calculated_total_cost,
    completion_start_time, prompt_id, usage_details, cost_details,
    created_at, updated_at"#;

/// UPSERT observation — the single event→PG sink for observations.
///
/// Column names must stay quoted where the schema uses camelCase
/// (`"modelParameters"`) and enum casts must be quoted (`"ObservationType"`,
/// `"ObservationLevel"`): PostgreSQL folds unquoted identifiers to lowercase,
/// so `model_parameters` / `observation_type` do not resolve to these objects.
pub async fn upsert_observation(pool: &PgPool, record: &ObservationRecord) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO observations (
            id, trace_id, project_id, type, start_time, end_time, name,
            metadata, parent_observation_id, level, status_message, version,
            model, internal_model, internal_model_id,
            "modelParameters", input, output,
            prompt_tokens, completion_tokens, total_tokens, unit,
            usage_details,
            input_cost, output_cost, total_cost,
            cost_details,
            calculated_input_cost, calculated_output_cost, calculated_total_cost,
            completion_start_time, prompt_id
        ) VALUES (
            $1, $2, $3, $4::"ObservationType", $5, $6, $7,
            $8, $9, $10::"ObservationLevel", $11, $12,
            $13, $14, $15,
            $16, $17, $18,
            $19, $20, $21, $22,
            $23,
            $24, $25, $26,
            $27,
            $28, $29, $30,
            $31, $32
        )
        ON CONFLICT (id, project_id) DO UPDATE SET
            trace_id = COALESCE(EXCLUDED.trace_id, observations.trace_id),
            type = EXCLUDED.type, start_time = EXCLUDED.start_time,
            end_time = COALESCE(EXCLUDED.end_time, observations.end_time),
            name = COALESCE(EXCLUDED.name, observations.name),
            metadata = COALESCE(EXCLUDED.metadata, observations.metadata),
            level = EXCLUDED.level, status_message = COALESCE(EXCLUDED.status_message, observations.status_message),
            version = COALESCE(EXCLUDED.version, observations.version),
            model = COALESCE(EXCLUDED.model, observations.model),
            "modelParameters" = COALESCE(EXCLUDED."modelParameters", observations."modelParameters"),
            input = COALESCE(EXCLUDED.input, observations.input),
            output = COALESCE(EXCLUDED.output, observations.output),
            prompt_tokens = COALESCE(EXCLUDED.prompt_tokens, observations.prompt_tokens),
            completion_tokens = COALESCE(EXCLUDED.completion_tokens, observations.completion_tokens),
            total_tokens = COALESCE(EXCLUDED.total_tokens, observations.total_tokens),
            unit = COALESCE(EXCLUDED.unit, observations.unit),
            usage_details = COALESCE(EXCLUDED.usage_details, observations.usage_details),
            input_cost = COALESCE(EXCLUDED.input_cost, observations.input_cost),
            output_cost = COALESCE(EXCLUDED.output_cost, observations.output_cost),
            total_cost = COALESCE(EXCLUDED.total_cost, observations.total_cost),
            cost_details = COALESCE(EXCLUDED.cost_details, observations.cost_details),
            calculated_input_cost = COALESCE(EXCLUDED.calculated_input_cost, observations.calculated_input_cost),
            calculated_output_cost = COALESCE(EXCLUDED.calculated_output_cost, observations.calculated_output_cost),
            calculated_total_cost = COALESCE(EXCLUDED.calculated_total_cost, observations.calculated_total_cost),
            completion_start_time = COALESCE(EXCLUDED.completion_start_time, observations.completion_start_time),
            prompt_id = COALESCE(EXCLUDED.prompt_id, observations.prompt_id),
            updated_at = now()"#
    )
    .bind(&record.id)
    .bind(&record.trace_id)
    .bind(&record.project_id)
    .bind(record.obs_type.to_string())
    .bind(record.start_time)
    .bind(record.end_time)
    .bind(&record.name)
    .bind(&record.metadata)
    .bind(&record.parent_observation_id)
    .bind(record.level.as_ref().map(|l| l.to_string()))
    .bind(&record.status_message)
    .bind(&record.version)
    .bind(&record.model)
    .bind(&record.internal_model)
    .bind(&record.internal_model_id)
    .bind(&record.model_parameters)
    .bind(&record.input)
    .bind(&record.output)
    .bind(record.prompt_tokens)
    .bind(record.completion_tokens)
    .bind(record.total_tokens)
    .bind(&record.unit)
    .bind(&record.usage_details)
    .bind(record.input_cost)
    .bind(record.output_cost)
    .bind(record.total_cost)
    .bind(&record.cost_details)
    .bind(record.calculated_input_cost)
    .bind(record.calculated_output_cost)
    .bind(record.calculated_total_cost)
    .bind(record.completion_start_time)
    .bind(&record.prompt_id)
    .execute(pool)
    .await?;

    Ok(())
}

use chrono::NaiveDateTime;
use sqlx::Row;

// ============================================================================
// ObservationRow — query result type
// ============================================================================

#[derive(Debug, Clone)]
pub struct ObservationRow {
    pub id: String,
    pub trace_id: Option<String>,
    pub project_id: String,
    pub obs_type: String,
    pub start_time: Option<NaiveDateTime>,
    pub end_time: Option<NaiveDateTime>,
    pub name: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub parent_observation_id: Option<String>,
    pub level: Option<String>,
    pub status_message: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
    pub internal_model: Option<String>,
    pub internal_model_id: Option<String>,
    pub model_parameters: Option<serde_json::Value>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub unit: Option<String>,
    pub usage_details: Option<serde_json::Value>,
    pub input_cost: Option<Decimal>,
    pub output_cost: Option<Decimal>,
    pub total_cost: Option<Decimal>,
    pub cost_details: Option<serde_json::Value>,
    pub calculated_input_cost: Option<Decimal>,
    pub calculated_output_cost: Option<Decimal>,
    pub calculated_total_cost: Option<Decimal>,
    pub completion_start_time: Option<NaiveDateTime>,
    pub prompt_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

fn map_row(r: &sqlx::postgres::PgRow) -> ObservationRow {
    ObservationRow {
        id: r.get("id"),
        trace_id: r.get("trace_id"),
        project_id: r.get("project_id"),
        obs_type: r.get("type"),
        start_time: r.get("start_time"),
        end_time: r.get("end_time"),
        name: r.get("name"),
        metadata: r.get("metadata"),
        parent_observation_id: r.get("parent_observation_id"),
        level: r.get("level"),
        status_message: r.get("status_message"),
        version: r.get("version"),
        model: r.get("model"),
        internal_model: r.get("internal_model"),
        internal_model_id: r.get("internal_model_id"),
        model_parameters: r.get("model_parameters"),
        input: r.get("input"),
        output: r.get("output"),
        prompt_tokens: r.get("prompt_tokens"),
        completion_tokens: r.get("completion_tokens"),
        total_tokens: r.get("total_tokens"),
        unit: r.get("unit"),
        usage_details: r.get("usage_details"),
        input_cost: r.get("input_cost"),
        output_cost: r.get("output_cost"),
        total_cost: r.get("total_cost"),
        cost_details: r.get("cost_details"),
        calculated_input_cost: r.get("calculated_input_cost"),
        calculated_output_cost: r.get("calculated_output_cost"),
        calculated_total_cost: r.get("calculated_total_cost"),
        completion_start_time: r.get("completion_start_time"),
        prompt_id: r.get("prompt_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// List observations with optional filters.
///
/// Each filter is applied in SQL rather than after the fetch: `obs_type` and
/// `cursor` used to be accepted and ignored, so `?type=GENERATION` returned
/// every observation and paging never advanced.
///
/// `type::text = UPPER($n)` rather than `type = $n::"ObservationType"`: the
/// labels are uppercase enum values, and comparing text keeps an unknown type
/// from raising a cast error that would surface as a 500.
pub async fn list_observations(
    pool: &PgPool,
    project_id: &str,
    trace_id: Option<&str>,
    obs_type: Option<&str>,
    cursor: Option<NaiveDateTime>,
    limit: i64,
) -> Result<Vec<ObservationRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations
         WHERE project_id = $1
           AND ($2::text IS NULL OR trace_id = $2)
           AND ($3::text IS NULL OR type::text = UPPER($3))
           AND ($4::timestamp IS NULL OR start_time < $4)
         ORDER BY start_time DESC NULLS LAST
         LIMIT $5"
    ))
    .bind(project_id)
    .bind(trace_id)
    .bind(obs_type)
    .bind(cursor)
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| map_row(&r)).collect())
}

/// Find a single observation by ID
pub async fn find_by_id(pool: &PgPool, id: &str, project_id: &str) -> Result<Option<ObservationRow>> {
    let row = sqlx::query(&format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations WHERE id = $1 AND project_id = $2"
    ))
    .bind(id).bind(project_id)
    .fetch_optional(pool).await?;
    Ok(row.map(|r| map_row(&r)))
}

/// Per-trace rollups used by the public trace endpoints.
#[derive(Debug, Clone, Default)]
pub struct TraceRollup {
    /// `max(end_time) - min(start_time)` in milliseconds.
    pub latency_ms: Option<f64>,
    /// Sum of `calculated_total_cost`.
    pub total_cost: Option<Decimal>,
    pub observation_count: i64,
}

/// Roll up latency, cost and observation count for a page of traces in one
/// query.
///
/// The public trace endpoints report these per trace; one query per row would
/// turn a 50-row page into 50 round trips.
pub async fn trace_rollups(
    pool: &PgPool,
    project_id: &str,
    trace_ids: &[String],
) -> Result<std::collections::HashMap<String, TraceRollup>> {
    if trace_ids.is_empty() {
        return Ok(Default::default());
    }

    let rows = sqlx::query(
        r#"SELECT trace_id,
                  COUNT(*) AS observation_count,
                  -- Cast: EXTRACT returns numeric, which does not decode into f64.
                  (EXTRACT(EPOCH FROM (MAX(COALESCE(end_time, start_time)) - MIN(start_time))) * 1000)
                      ::double precision AS latency_ms,
                  SUM(calculated_total_cost) AS total_cost
           FROM observations
           WHERE project_id = $1 AND trace_id = ANY($2)
           GROUP BY trace_id"#,
    )
    .bind(project_id)
    .bind(trace_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            // A decode error here means a type changed in SQL. Left silent it
            // would present as "this trace has no latency", which is
            // indistinguishable from a trace with no observations — so it is
            // logged rather than swallowed by `unwrap_or(None)`.
            let latency_ms = match r.try_get::<Option<f64>, _>("latency_ms") {
                Ok(value) => value,
                Err(e) => {
                    tracing::warn!(error = %e, "trace rollup: latency_ms decode failed");
                    None
                }
            };
            let total_cost = match r.try_get::<Option<Decimal>, _>("total_cost") {
                Ok(value) => value,
                Err(e) => {
                    tracing::warn!(error = %e, "trace rollup: total_cost decode failed");
                    None
                }
            };

            let rollup = TraceRollup {
                latency_ms,
                total_cost,
                observation_count: r.try_get::<i64, _>("observation_count").unwrap_or(0),
            };
            (r.get::<String, _>("trace_id"), rollup)
        })
        .collect())
}
