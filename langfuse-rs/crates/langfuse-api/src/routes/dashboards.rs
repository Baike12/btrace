use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery { pub project_id: String, pub limit: Option<i64> }
#[derive(Debug, Serialize, ToSchema)] struct DashboardResponse {
    pub id: String, pub project_id: String, pub name: String,
    pub description: Option<String>, pub definition: Option<serde_json::Value>,
    pub filters: Option<serde_json::Value>,
    pub created_at: String, pub updated_at: String,
}
/// List dashboards for a project
#[utoipa::path(
    get,
    path = "/api/dashboards",
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
        "SELECT id, project_id, name, description, definition, filters, created_at, updated_at
         FROM dashboards WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2"
    ).bind(&q.project_id).bind(limit + 1).fetch_all(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<DashboardResponse> = rows.into_iter().take(limit as usize).map(|r| DashboardResponse {
        id: r.get("id"), project_id: r.get("project_id"), name: r.get("name"),
        description: r.get("description"), definition: r.get("definition"),
        filters: r.get("filters"),
        created_at: r.get::<chrono::NaiveDateTime, _>("created_at").and_utc().to_rfc3339(),
        updated_at: r.get::<chrono::NaiveDateTime, _>("updated_at").and_utc().to_rfc3339(),
    }).collect();
    Ok(Json(serde_json::json!({
        "data": { "dashboards": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
