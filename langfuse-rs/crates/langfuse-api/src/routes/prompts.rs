use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery {
    pub project_id: String, pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)] struct PromptResponse {
    pub id: String, pub project_id: String, pub name: String, pub version: i32,
    pub is_active: bool, pub prompt: Option<String>, pub config: Option<serde_json::Value>,
    pub tags: Vec<String>, pub labels: Vec<String>,
    pub created_at: String, pub updated_at: String,
}
/// List prompts for a project
#[utoipa::path(
    get,
    path = "/api/prompts",
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
        "SELECT id, project_id, name, version, is_active, prompt, config, tags, labels, created_at, updated_at
         FROM prompts WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2"
    ).bind(&q.project_id).bind(limit + 1).fetch_all(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<PromptResponse> = rows.into_iter().take(limit as usize).map(|r| PromptResponse {
        id: r.get("id"), project_id: r.get("project_id"), name: r.get("name"),
        version: r.get("version"), is_active: r.get("is_active"),
        prompt: r.get("prompt"), config: r.get("config"),
        tags: r.get::<Vec<String>, _>("tags"), labels: r.get::<Vec<String>, _>("labels"),
        created_at: r.get::<chrono::NaiveDateTime, _>("created_at").and_utc().to_rfc3339(),
        updated_at: r.get::<chrono::NaiveDateTime, _>("updated_at").and_utc().to_rfc3339(),
    }).collect();
    Ok(Json(serde_json::json!({
        "data": { "prompts": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
