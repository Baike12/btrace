//! `GET /api/public/metrics/daily`.
//!
//! Per-day token usage and cost, broken down by model and unit. This is the
//! endpoint LexQA needs for cost reconciliation — the v2 metrics API in this
//! fork's fern is a general OLAP query engine, and a caller that just wants
//! "what did we spend, per day" should not have to build one.
//!
//! The v1 daily endpoint is not in this fork's fern sources, so the response
//! shape follows the published v1 contract.

use std::collections::BTreeMap;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::{Duration, Utc};
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_db::repos::metrics;
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::response::public_error;

/// Applied when the caller omits `fromTimestamp`: the window the Langfuse UI
/// defaults to.
const DEFAULT_WINDOW_DAYS: i64 = 7;

pub async fn daily(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    uri: axum::http::Uri,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    let pairs = super::traces::parse_query_pairs(uri.query().unwrap_or(""));

    let now = Utc::now().naive_utc();
    let mut from = now - Duration::days(DEFAULT_WINDOW_DAYS);
    let mut to = now;

    for (key, value) in &pairs {
        match key.as_str() {
            "fromTimestamp" => match super::traces::parse_timestamp(value) {
                Ok(parsed) => from = parsed,
                Err(message) => {
                    return public_error(StatusCode::BAD_REQUEST, message, "bad_request")
                }
            },
            "toTimestamp" => match super::traces::parse_timestamp(value) {
                Ok(parsed) => to = parsed,
                Err(message) => {
                    return public_error(StatusCode::BAD_REQUEST, message, "bad_request")
                }
            },
            other => {
                return public_error(
                    StatusCode::BAD_REQUEST,
                    format!("Unknown query parameter: {}", other),
                    "bad_request",
                )
            }
        }
    }

    if to <= from {
        return public_error(
            StatusCode::BAD_REQUEST,
            "toTimestamp must be after fromTimestamp",
            "bad_request",
        );
    }

    let (counts, usage) = tokio::join!(
        metrics::daily_counts(&state.pool, &project_id, from, to),
        metrics::daily_usage(&state.pool, &project_id, from, to),
    );

    let (counts, usage) = match (counts, usage) {
        (Ok(counts), Ok(usage)) => (counts, usage),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "public metrics: daily aggregation failed");
            return public_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to compute daily metrics",
                "internal_error",
            );
        }
    };

    // Group usage under its day, keeping days that only have counts.
    let mut by_day: BTreeMap<String, (i64, i64, Vec<Value>)> = BTreeMap::new();

    for row in &counts {
        let entry = by_day
            .entry(row.date.to_string())
            .or_insert((0, 0, Vec::new()));
        entry.0 = row.count_traces;
        entry.1 = row.count_observations;
    }

    for row in &usage {
        let entry = by_day
            .entry(row.date.to_string())
            .or_insert((0, 0, Vec::new()));
        entry.2.push(json!({
            "model": row.model,
            "unit": row.unit,
            "inputTokens": row.input_tokens,
            "outputTokens": row.output_tokens,
            "totalTokens": row.total_tokens,
            // Costs are rendered as JSON numbers, matching the contract's
            // `double`; the values are already computed at ingest time.
            "inputCost": decimal_to_f64(row.input_cost),
            "outputCost": decimal_to_f64(row.output_cost),
            "totalCost": decimal_to_f64(row.total_cost),
        }));
    }

    let data: Vec<Value> = by_day
        .into_iter()
        .map(|(date, (count_traces, count_observations, usage))| {
            let day_cost: f64 = usage
                .iter()
                .filter_map(|u| u.get("totalCost").and_then(Value::as_f64))
                .sum();
            let mut day = Map::new();
            day.insert("date".into(), json!(date));
            day.insert("countTraces".into(), json!(count_traces));
            day.insert("countObservations".into(), json!(count_observations));
            day.insert("totalCost".into(), json!(day_cost));
            day.insert("usage".into(), Value::Array(usage));
            Value::Object(day)
        })
        .collect();

    (
        StatusCode::OK,
        axum::Json(json!({
            "data": data,
            "meta": { "fromTimestamp": from.and_utc().to_rfc3339(), "toTimestamp": to.and_utc().to_rfc3339() },
        })),
    )
        .into_response()
}

fn decimal_to_f64(value: rust_decimal::Decimal) -> f64 {
    value.to_string().parse().unwrap_or(0.0)
}
