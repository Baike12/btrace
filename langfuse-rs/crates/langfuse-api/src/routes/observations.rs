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
use langfuse_db::repos::observations::{self, ObservationRow};

use crate::app::AppState;
use crate::middleware::auth::require_project_access;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListObservationsQuery {
    pub project_id: String,
    pub trace_id: Option<String>,
    #[serde(rename = "type")]
    pub obs_type: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

// ============================================================================
// Response types
// ============================================================================

#[derive(Debug, Serialize, ToSchema)]
pub struct ObservationResponse {
    pub id: String,
    pub trace_id: Option<String>,
    pub project_id: String,
    #[serde(rename = "type")]
    pub obs_type: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
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
    /// Usage broken down by usage type, including the cache dimensions
    /// (`cache_read_input_tokens`, `cache_creation_input_tokens`).
    pub usage_details: Option<serde_json::Value>,
    /// The headline usage numbers, reduced from `usage_details` the same way the
    /// read converter does it. The trace tree reads these rather than the three
    /// integer columns, so they have to be present for token counts to render.
    pub input_usage: Option<i64>,
    pub output_usage: Option<i64>,
    pub total_usage: Option<i64>,
    /// Cost broken down by the same usage types as `usage_details`.
    pub cost_details: Option<serde_json::Value>,
    pub input_cost: Option<f64>,
    pub output_cost: Option<f64>,
    pub total_cost: Option<f64>,
    /// The public API's spelling of the same three numbers; the read path
    /// derives them from `cost_details`, so they are aliases and not a second
    /// computation.
    pub calculated_input_cost: Option<f64>,
    pub calculated_output_cost: Option<f64>,
    pub calculated_total_cost: Option<f64>,
    pub completion_start_time: Option<String>,
    pub prompt_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Reduce a `usage_details` / `cost_details` object the way the read converter
/// does (`observations_converters.ts` → `reduceUsageOrCostDetails`): every key
/// carrying the `input` prefix sums into one number, likewise `output`, and
/// `total` is read from the explicit entry.
///
/// A side with no matching keys stays `None` rather than reporting `0`, so an
/// observation that was never priced does not look like one that cost nothing.
fn reduce_details(details: Option<&serde_json::Value>) -> (Option<f64>, Option<f64>, Option<f64>) {
    let Some(object) = details.and_then(serde_json::Value::as_object) else {
        return (None, None, None);
    };

    let sum = |prefix: &str| -> Option<f64> {
        let matched: Vec<f64> = object
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .filter_map(|(_, value)| as_f64(value))
            .collect();
        (!matched.is_empty()).then(|| matched.iter().sum())
    };

    (
        sum("input"),
        sum("output"),
        object.get("total").and_then(as_f64),
    )
}

fn as_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Money columns are `numeric(65,30)`, so they arrive scaled to 30 decimals;
/// `langfuse_core::to_json_f64` normalises before converting, which is what
/// keeps a value like `1000` from rendering as `1000.0000000000001`.
fn decimal_f64(value: Option<rust_decimal::Decimal>) -> Option<f64> {
    value.and_then(langfuse_core::to_json_f64)
}

fn opt_ts(ts: Option<chrono::NaiveDateTime>) -> Option<String> {
    ts.map(|t| t.and_utc().to_rfc3339())
}

impl From<ObservationRow> for ObservationResponse {
    fn from(row: ObservationRow) -> Self {
        let (input_usage, output_usage, total_usage) = reduce_details(row.usage_details.as_ref());
        let (input_cost, output_cost, total_cost) = reduce_details(row.cost_details.as_ref());

        // `cost_details` is the source of truth once it is populated. The stored
        // columns are the fallback for rows written before it was, so a
        // half-migrated database still reports the costs it does have.
        let input_cost = input_cost.or_else(|| decimal_f64(row.input_cost));
        let output_cost = output_cost.or_else(|| decimal_f64(row.output_cost));
        let total_cost = total_cost
            .or_else(|| decimal_f64(row.total_cost))
            .or_else(|| match (input_cost, output_cost) {
                (None, None) => None,
                (i, o) => Some(i.unwrap_or(0.0) + o.unwrap_or(0.0)),
            });

        Self {
            id: row.id,
            trace_id: row.trace_id,
            project_id: row.project_id,
            obs_type: row.obs_type,
            start_time: opt_ts(row.start_time),
            end_time: opt_ts(row.end_time),
            name: row.name,
            metadata: row.metadata,
            parent_observation_id: row.parent_observation_id,
            level: row.level,
            status_message: row.status_message,
            version: row.version,

            model: row.model,
            internal_model: row.internal_model,
            internal_model_id: row.internal_model_id,
            model_parameters: row.model_parameters,
            input: row.input,
            output: row.output,
            // Derived from `usage_details` when it is there, which is how the
            // read converter sources them.
            prompt_tokens: input_usage
                .map(|v| v as i32)
                .or(row.prompt_tokens),
            completion_tokens: output_usage
                .map(|v| v as i32)
                .or(row.completion_tokens),
            total_tokens: total_usage.map(|v| v as i32).or(row.total_tokens),
            unit: row.unit,
            usage_details: row.usage_details,
            input_usage: input_usage.map(|v| v as i64),
            output_usage: output_usage.map(|v| v as i64),
            total_usage: total_usage.map(|v| v as i64),
            cost_details: row.cost_details,
            input_cost,
            output_cost,
            total_cost,
            calculated_input_cost: input_cost,
            calculated_output_cost: output_cost,
            calculated_total_cost: total_cost,
            completion_start_time: opt_ts(row.completion_start_time),
            prompt_id: row.prompt_id,
            created_at: row.created_at.and_utc().to_rfc3339(),
            updated_at: row.updated_at.and_utc().to_rfc3339(),
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================
/// List observations with optional filters
#[utoipa::path(
    get,
    path = "/api/observations",
    params(ListObservationsQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]

/// GET /api/observations — 列出 observations
pub(crate) async fn list_observations(
    Extension(session): Extension<Session>,
    Query(params): Query<ListObservationsQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &params.project_id).await?;

    let limit = params.limit.unwrap_or(50).min(100);
    let cursor_ts = params.cursor.as_deref().and_then(parse_iso);

    let rows = observations::list_observations(
        &state.pool,
        &params.project_id,
        params.trace_id.as_deref(),
        params.obs_type.as_deref(),
        cursor_ts,
        limit,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let has_more = rows.len() > limit as usize;
    let obs: Vec<ObservationResponse> = rows.into_iter()
        .take(limit as usize)
        .map(ObservationResponse::from)
        .collect();
    let next_cursor = obs.last().and_then(|o| o.start_time.clone());

    Ok(Json(serde_json::json!({
        "data": { "observations": obs },
        "meta": { "cursor": next_cursor, "has_more": has_more, "total": null },
        "error": null,
    })))
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_iso(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
        .ok()
}

// ============================================================================
// Router
// ============================================================================

/// GET /api/observations/:id — 获取单个 observation
pub(crate) async fn get_observation(
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Query(params): Query<GetObservationQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &params.project_id).await?;

    let obs = observations::find_by_id(&state.pool, &id, &params.project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Observation not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "data": ObservationResponse::from(obs),
        "meta": null,
        "error": null,
    })))
}

// ============================================================================
// Router
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetObservationQuery {
    pub project_id: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_observations))
        .route("/{id}", get(get_observation))
}
