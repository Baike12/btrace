use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery { pub project_id: String, pub limit: Option<i64> }
#[derive(Debug, Serialize, ToSchema)] struct MonitorResponse {
    pub id: String, pub project_id: String, pub name: Option<String>,
    pub view: Option<String>, pub metric: Option<String>,
    pub status: Option<String>, pub severity: Option<String>,
    pub next_run_at: Option<String>, pub last_published_at: Option<String>,
    pub created_at: String, pub updated_at: String,
}
/// List monitors for a project
#[utoipa::path(
    get,
    path = "/api/monitors",
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
    let rows = sqlx::query(
        "SELECT id, project_id, name, view, metric, status, severity, next_run_at,
         last_published_at, created_at, updated_at
         FROM monitors WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2"
    ).bind(&q.project_id).bind(limit + 1).fetch_all(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<MonitorResponse> = rows.into_iter().take(limit as usize).map(|r| MonitorResponse {
        id: r.get("id"), project_id: r.get("project_id"), name: r.get("name"),
        view: r.get("view"), metric: r.get("metric"),
        status: r.get("status"), severity: r.get("severity"),
        next_run_at: r.get::<Option<chrono::NaiveDateTime>, _>("next_run_at").map(|t| t.and_utc().to_rfc3339()),
        last_published_at: r.get::<Option<chrono::NaiveDateTime>, _>("last_published_at").map(|t| t.and_utc().to_rfc3339()),
        created_at: r.get::<chrono::NaiveDateTime, _>("created_at").and_utc().to_rfc3339(),
        updated_at: r.get::<chrono::NaiveDateTime, _>("updated_at").and_utc().to_rfc3339(),
    }).collect();
    Ok(Json(serde_json::json!({
        "data": { "monitors": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
