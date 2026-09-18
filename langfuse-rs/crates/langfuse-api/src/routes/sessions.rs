use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use langfuse_auth::jwt::Session;
use langfuse_db::repos::sessions::{self, SessionRow};
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery { pub project_id: String, pub limit: Option<i64> }

#[derive(Debug, Serialize, ToSchema)] pub struct SessionResponse {
    pub id: String, pub project_id: String, pub environment: Option<String>,
    pub bookmarked: bool, pub public: bool, pub created_at: String, pub updated_at: String,
}
impl From<SessionRow> for SessionResponse {
    fn from(r: SessionRow) -> Self { Self {
        id: r.id, project_id: r.project_id, environment: r.environment,
        bookmarked: r.bookmarked, public: r.public,
        created_at: r.created_at.and_utc().to_rfc3339(), updated_at: r.updated_at.and_utc().to_rfc3339(),
    }}
}
/// List sessions for a project
#[utoipa::path(
    get,
    path = "/api/sessions",
    params(ListQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]

pub(crate) async fn list(Extension(session): Extension<Session>, Query(q): Query<ListQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;
    let limit = q.limit.unwrap_or(50).min(100);
    let rows = sessions::list_sessions(&state.pool, &q.project_id, limit)
        .await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<SessionResponse> = rows.into_iter().take(limit as usize).map(SessionResponse::from).collect();
    Ok(Json(serde_json::json!({
        "data": { "sessions": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
/// Query for the `hasAny` probes: just the project.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct HasAnyQuery {
    pub project_id: String,
}

/// `GET /api/sessions/hasAny` — whether a project has any session.
///
/// The sessions page branches on this before rendering the table, so returning
/// 404 here leaves the page stuck on its onboarding empty state even when
/// sessions exist.
#[utoipa::path(
    get,
    path = "/api/sessions/hasAny",
    params(HasAnyQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]
pub(crate) async fn has_any(
    Extension(session): Extension<Session>,
    Query(q): Query<HasAnyQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;
    probe_has_any(&state, &q.project_id).await
}

/// `GET /api/sessions/hasAnyFromEvents` — the same probe for the beta events
/// path.
///
/// There is no separate events table in this fork: sessions come from
/// `trace_sessions` whichever view the user is on, so both probes read it. The
/// endpoint exists so toggling the beta flag does not 404 the page.
pub(crate) async fn has_any_from_events(
    Extension(session): Extension<Session>,
    Query(q): Query<HasAnyQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;
    // Shares `probe_has_any` with `has_any` rather than calling that handler,
    // which would re-enter `require_project_access` and pay for the same
    // membership lookup twice.
    probe_has_any(&state, &q.project_id).await
}

async fn probe_has_any(
    state: &AppState,
    project_id: &str,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let exists = sessions::has_any(&state.pool, project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        serde_json::json!({ "data": exists, "meta": null, "error": null }),
    ))
}

/// Query for `GET /api/sessions/metrics`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct MetricsQuery {
    pub project_id: String,
    /// Session ids, sent by the table as a JSON array in one parameter.
    /// Comma-separated values are accepted too, so the endpoint stays usable by
    /// hand.
    #[serde(default, deserialize_with = "deserialize_ids")]
    pub session_ids: Vec<String>,
}

fn deserialize_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
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

/// `GET /api/sessions/metrics` — per-session roll-ups.
///
/// Returns a bare array (not an envelope) because the sessions table reduces it
/// directly; each entry is joined onto the session row by `id`.
#[utoipa::path(
    get,
    path = "/api/sessions/metrics",
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

    let rows = sessions::session_metrics(&state.pool, &q.project_id, &q.session_ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "createdAt": m.min_timestamp.map(|t| t.and_utc().to_rfc3339()),
                "userIds": m.user_ids,
                "traceTags": m.trace_tags,
                "countTraces": m.trace_count,
                // `trace_count` alongside `countTraces`: upstream emits both and
                // different table paths read different spellings.
                "trace_count": m.trace_count,
                "total_observations": m.total_observations,
                "sessionDuration": m.duration,
                "inputCost": decimal_f64(m.input_cost),
                "outputCost": decimal_f64(m.output_cost),
                "totalCost": decimal_f64(m.total_cost),
                "inputTokens": m.input_usage,
                "outputTokens": m.output_usage,
                "totalTokens": m.total_usage,
                // Upstream names these prompt/completion; the table reads the
                // `*Tokens` pair above.
                "promptTokens": m.input_usage,
                "completionTokens": m.output_usage,
                // Scores are joined separately by the table; an empty array keeps
                // the shape stable.
                "scores": [],
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "data": data, "meta": null, "error": null,
    })))
}

fn decimal_f64(value: rust_decimal::Decimal) -> f64 {
    langfuse_core::to_json_f64_or_zero(value)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        // Static segments outrank the dynamic `/{}` patterns axum may grow here
        // later, so these cannot be shadowed by a session id.
        .route("/hasAny", get(has_any))
        .route("/hasAnyFromEvents", get(has_any_from_events))
        .route("/metrics", get(metrics))
        .route("/metricsFromEvents", get(metrics))
}
