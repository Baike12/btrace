use axum::{
    Router,
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::get,
    Extension,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;

use langfuse_auth::jwt::Session;
use langfuse_db::repos::traces::{self, TraceRow};
use langfuse_db::repos::{observations, scores};

use crate::app::AppState;
use crate::middleware::auth::require_project_access;
use crate::routes::observations::ObservationResponse;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListTracesQuery {
    pub project_id: String,
    pub cursor: Option<String>,   // ISO timestamp
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct GetTraceQuery {
    pub project_id: String,
}

// ============================================================================
// Response types
// ============================================================================

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceResponse {
    pub id: String,
    pub project_id: String,
    pub external_id: Option<String>,
    pub timestamp: String,        // ISO 8601
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
    pub created_at: String,
    pub updated_at: String,
}

impl From<TraceRow> for TraceResponse {
    fn from(row: TraceRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            external_id: row.external_id,
            timestamp: row.timestamp.and_utc().to_rfc3339(),
            name: row.name,
            user_id: row.user_id,
            metadata: row.metadata,
            release: row.release,
            version: row.version,
            public: row.public,
            bookmarked: row.bookmarked,
            tags: row.tags,
            input: row.input,
            output: row.output,
            session_id: row.session_id,
            created_at: row.created_at.and_utc().to_rfc3339(),
            updated_at: row.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListTracesData {
    pub traces: Vec<TraceResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PaginationMeta {
    pub cursor: Option<String>,
    pub has_more: bool,
    pub total: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================
/// List of traces with cursor pagination
#[utoipa::path(
    get,
    path = "/api/traces",
    params(ListTracesQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]

/// GET /api/traces — 列出 traces
pub(crate) async fn list_traces(
    Extension(session): Extension<Session>,
    Query(params): Query<ListTracesQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // 验证用户有该项目权限
    require_project_access(&state.pool, &session, &params.project_id).await?;

    let limit = params.limit.unwrap_or(50).min(100);

    let cursor_ts = params.cursor.as_deref()
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
            .ok());

    let rows = traces::list_traces(&state.pool, &params.project_id, cursor_ts, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let has_more = rows.len() > limit as usize;
    let traces: Vec<TraceResponse> = rows.into_iter()
        .take(limit as usize)
        .map(TraceResponse::from)
        .collect();

    let next_cursor = traces.last().map(|t| t.timestamp.clone());

    let total = traces::count_traces(&state.pool, &params.project_id)
        .await
        .ok();

    let response = serde_json::json!({
        "data": { "traces": traces },
        "meta": {
            "cursor": next_cursor,
            "has_more": has_more,
            "total": total,
        },
        "error": null,
    });

    Ok(Json(response))
}

/// GET /api/traces/:trace_id — 获取单个 trace
#[utoipa::path(
    get,
    path = "/api/traces/{trace_id}",
    params(
        ("trace_id" = String, Path, description = "Trace ID"),
        GetTraceQuery,
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_auth" = [])),
)]
pub(crate) async fn get_trace(
    Extension(session): Extension<Session>,
    Path(trace_id): Path<String>,
    Query(params): Query<GetTraceQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &params.project_id).await?;

    let trace = traces::find_by_id(&state.pool, &trace_id, &params.project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Trace not found".to_string()))?;

    let response = serde_json::json!({
        "data": TraceResponse::from(trace),
        "meta": null,
        "error": null,
    });

    Ok(Json(response))
}

// ============================================================================
// Router
// ============================================================================

/// GET /api/traces/:trace_id/full — trace with nested observations and scores
pub(crate) async fn get_trace_full(
    Extension(session): Extension<Session>,
    Path(trace_id): Path<String>,
    Query(params): Query<GetTraceQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &params.project_id).await?;

    let trace = traces::find_by_id(&state.pool, &trace_id, &params.project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Trace not found".to_string()))?;

    let observations = observations::list_observations(
        &state.pool, &params.project_id, Some(&trace_id), None, None, 500,
    ).await.unwrap_or_default();

    let score_rows = scores::find_by_trace_id(&state.pool, &trace_id, &params.project_id)
        .await.unwrap_or_default();

    let scores_json: Vec<serde_json::Value> = score_rows.iter().map(|s| {
        serde_json::json!({
            "id": s.id, "project_id": s.project_id,
            "timestamp": s.timestamp.and_utc().to_rfc3339(),
            "name": s.name, "value": s.value, "source": s.source,
            "data_type": s.data_type, "comment": s.comment,
            "author_user_id": s.author_user_id,
            "trace_id": s.trace_id, "observation_id": s.observation_id,
            "config_id": s.config_id, "string_value": s.string_value,
            "queue_id": s.queue_id,
            "created_at": s.created_at.and_utc().to_rfc3339(),
            "updated_at": s.updated_at.and_utc().to_rfc3339(),
        })
    }).collect();

    let response = serde_json::json!({
        "data": {
            "trace": TraceResponse::from(trace),
            "observations": observations.into_iter().map(ObservationResponse::from).collect::<Vec<_>>(),
            "scores": scores_json,
            "corrections": [],
        },
        "meta": null,
        "error": null,
    });
    Ok(Json(response))
}

// ============================================================================
// Router
// ============================================================================

/// Query for `GET /api/traces/metrics`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct MetricsQuery {
    pub project_id: String,
    /// Trace ids, sent by the table as a JSON array in one parameter.
    /// Comma-separated values are accepted too.
    #[serde(default, deserialize_with = "deserialize_trace_ids")]
    pub trace_ids: Vec<String>,
}

fn deserialize_trace_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(match raw {
        None => Vec::new(),
        Some(raw) if raw.trim().is_empty() => Vec::new(),
        Some(raw) => serde_json::from_str::<Vec<String>>(&raw)
            .unwrap_or_else(|_| raw.split(',').map(|s| s.trim().to_string()).collect()),
    })
}

/// `GET /api/traces/metrics` — per-trace roll-ups for the traces table.
///
/// Returns a bare array because the table joins it onto the trace rows by `id`
/// itself. Every requested trace gets an entry, including one with no
/// observations, so its cells leave the loading state.
///
/// The fields are **flat** (`promptTokens`, `errorCount`, `calculatedTotalCost`,
/// …), which is the shape upstream's `getTraceMetrics` returns. The table
/// rebuilds its rows field by field from this object, so a nested
/// `usage` / `levelCounts` grouping would be silently dropped and those columns
/// would render blank.
#[utoipa::path(
    get,
    path = "/api/traces/metrics",
    params(MetricsQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]
pub(crate) async fn metrics(
    Extension(session): Extension<Session>,
    Query(q): Query<MetricsQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;

    let rows = traces::trace_metrics(&state.pool, &q.project_id, &q.trace_ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|m| {
            let total_cost = langfuse_core::to_json_f64_or_zero(m.total_cost);
            serde_json::json!({
                "id": m.id,
                "latency": m.latency,
                "promptTokens": m.input_usage,
                "completionTokens": m.output_usage,
                "totalTokens": m.total_usage,
                "errorCount": m.error_count,
                "warningCount": m.warning_count,
                "debugCount": m.debug_count,
                "defaultCount": m.default_count,
                // Keyed by usage type, so the tooltips list the same lines the
                // observation detail view shows.
                "usageDetails": m.token_details,
                "costDetails": m.cost_details,
                "calculatedInputCost": langfuse_core::to_json_f64_or_zero(m.input_cost),
                "calculatedOutputCost": langfuse_core::to_json_f64_or_zero(m.output_cost),
                "calculatedTotalCost": total_cost,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "data": data, "meta": null, "error": null,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_traces))
        .route("/{trace_id}", get(get_trace))
        .route("/{trace_id}/full", get(get_trace_full))
        // Registered after the dynamic routes on purpose: axum prefers a static
        // segment, so `/metrics` is not swallowed by `/{trace_id}`.
        .route("/metrics", get(metrics))
}
