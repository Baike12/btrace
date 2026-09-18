use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery { pub project_id: String, pub limit: Option<i64> }
#[derive(Debug, Serialize, ToSchema)] struct MediaResponse {
    pub id: String, pub project_id: String, pub sha_256_hash: String,
    pub content_type: Option<String>, pub content_length: Option<i64>,
    pub bucket_path: Option<String>, pub bucket_name: Option<String>,
    pub uploaded_at: Option<String>, pub created_at: String, pub updated_at: String,
}
/// List media for a project
#[utoipa::path(
    get,
    path = "/api/media",
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
        "SELECT id, project_id, sha_256_hash, content_type, content_length, bucket_path, bucket_name,
         uploaded_at, created_at, updated_at
         FROM media WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2"
    ).bind(&q.project_id).bind(limit + 1).fetch_all(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<MediaResponse> = rows.into_iter().take(limit as usize).map(|r| MediaResponse {
        id: r.get("id"), project_id: r.get("project_id"), sha_256_hash: r.get("sha_256_hash"),
        content_type: r.get("content_type"), content_length: r.get("content_length"),
        bucket_path: r.get("bucket_path"), bucket_name: r.get("bucket_name"),
        uploaded_at: r.get::<Option<chrono::NaiveDateTime>, _>("uploaded_at").map(|t| t.and_utc().to_rfc3339()),
        created_at: r.get::<chrono::NaiveDateTime, _>("created_at").and_utc().to_rfc3339(),
        updated_at: r.get::<chrono::NaiveDateTime, _>("updated_at").and_utc().to_rfc3339(),
    }).collect();
    Ok(Json(serde_json::json!({
        "data": { "media": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
