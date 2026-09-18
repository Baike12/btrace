use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use langfuse_auth::jwt::Session;
use langfuse_db::repos::datasets::{self, DatasetRow};
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery { pub project_id: String, pub limit: Option<i64> }

#[derive(Debug, Serialize, ToSchema)] struct DatasetResponse {
    pub id: String, pub project_id: String, pub name: String,
    pub description: Option<String>, pub metadata: Option<serde_json::Value>,
    pub created_at: String, pub updated_at: String,
}
impl From<DatasetRow> for DatasetResponse {
    fn from(r: DatasetRow) -> Self { Self {
        id: r.id, project_id: r.project_id, name: r.name, description: r.description,
        metadata: r.metadata,
        created_at: r.created_at.and_utc().to_rfc3339(), updated_at: r.updated_at.and_utc().to_rfc3339(),
    }}
}
/// List datasets for a project
#[utoipa::path(
    get,
    path = "/api/datasets",
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
    let rows = datasets::list_datasets(&state.pool, &q.project_id, limit)
        .await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let has_more = rows.len() > limit as usize;
    let data: Vec<DatasetResponse> = rows.into_iter().take(limit as usize).map(DatasetResponse::from).collect();
    Ok(Json(serde_json::json!({
        "data": { "datasets": data }, "meta": { "cursor": null, "has_more": has_more, "total": null }, "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
