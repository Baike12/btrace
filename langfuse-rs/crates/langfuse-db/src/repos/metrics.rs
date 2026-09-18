//! Aggregation queries backing `GET /api/public/metrics/daily`.
//!
//! The v1 daily endpoint is not part of this fork's fern sources (only the v2
//! metrics engine is), but it is the shape that answers "what did this project
//! spend, per day and per model" without building an OLAP query builder — which
//! is what LexQA needs for cost reconciliation.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};

use langfuse_core::Result;

/// One (day, model, unit) bucket of token usage and cost.
#[derive(Debug, Clone)]
pub struct DailyUsageRow {
    pub date: NaiveDate,
    pub model: Option<String>,
    pub unit: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub total_cost: Decimal,
}

/// One day's trace and observation counts.
#[derive(Debug, Clone)]
pub struct DailyCountRow {
    pub date: NaiveDate,
    pub count_traces: i64,
    pub count_observations: i64,
}

/// Usage and cost per day/model/unit within `[from, to)`.
///
/// Cost is summed from the pre-calculated columns, so the numbers match what the
/// UI shows for the same observations — recomputing here from prices would let
/// the two disagree after a price change.
pub async fn daily_usage(
    pool: &PgPool,
    project_id: &str,
    from: chrono::NaiveDateTime,
    to: chrono::NaiveDateTime,
) -> Result<Vec<DailyUsageRow>> {
    let rows = sqlx::query(
        r#"SELECT (start_time AT TIME ZONE 'UTC')::date AS day,
                  model,
                  unit,
                  COALESCE(SUM(prompt_tokens), 0)::bigint     AS input_tokens,
                  COALESCE(SUM(completion_tokens), 0)::bigint AS output_tokens,
                  COALESCE(SUM(total_tokens), 0)::bigint      AS total_tokens,
                  COALESCE(SUM(calculated_input_cost), 0)     AS input_cost,
                  COALESCE(SUM(calculated_output_cost), 0)    AS output_cost,
                  COALESCE(SUM(calculated_total_cost), 0)     AS total_cost
           FROM observations
           WHERE project_id = $1 AND start_time >= $2 AND start_time < $3
           GROUP BY 1, 2, 3
           ORDER BY 1 DESC, 2 NULLS LAST, 3 NULLS LAST"#,
    )
    .bind(project_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DailyUsageRow {
            date: r.get("day"),
            model: r.get("model"),
            unit: r.get("unit"),
            input_tokens: r.get("input_tokens"),
            output_tokens: r.get("output_tokens"),
            total_tokens: r.get("total_tokens"),
            input_cost: r.get("input_cost"),
            output_cost: r.get("output_cost"),
            total_cost: r.get("total_cost"),
        })
        .collect())
}

/// Trace and observation counts per day within `[from, to)`.
pub async fn daily_counts(
    pool: &PgPool,
    project_id: &str,
    from: chrono::NaiveDateTime,
    to: chrono::NaiveDateTime,
) -> Result<Vec<DailyCountRow>> {
    let rows = sqlx::query(
        r#"WITH days AS (
               SELECT (timestamp AT TIME ZONE 'UTC')::date AS day, COUNT(*) AS count_traces
               FROM traces
               WHERE project_id = $1 AND timestamp >= $2 AND timestamp < $3
               GROUP BY 1
           ),
           obs AS (
               SELECT (start_time AT TIME ZONE 'UTC')::date AS day, COUNT(*) AS count_observations
               FROM observations
               WHERE project_id = $1 AND start_time >= $2 AND start_time < $3
               GROUP BY 1
           )
           SELECT COALESCE(days.day, obs.day) AS day,
                  COALESCE(days.count_traces, 0)::bigint       AS count_traces,
                  COALESCE(obs.count_observations, 0)::bigint  AS count_observations
           FROM days FULL OUTER JOIN obs ON days.day = obs.day
           ORDER BY 1 DESC"#,
    )
    .bind(project_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DailyCountRow {
            date: r.get("day"),
            count_traces: r.get("count_traces"),
            count_observations: r.get("count_observations"),
        })
        .collect())
}
