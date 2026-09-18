use axum::{
    Router,
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::get,
    Extension,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;

use langfuse_auth::jwt::Session;
use langfuse_db::repos::scores::{self, ScoreRow};

use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListScoresQuery {
    pub project_id: String,
    pub trace_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScoreResponse {
    pub id: String,
    pub project_id: String,
    pub timestamp: String,
    pub name: Option<String>,
    pub value: Option<f64>,
    pub source: String,
    pub author_user_id: Option<String>,
    pub comment: Option<String>,
    pub trace_id: Option<String>,
    pub observation_id: Option<String>,
    pub config_id: Option<String>,
    pub string_value: Option<String>,
    pub queue_id: Option<String>,
    pub data_type: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ScoreRow> for ScoreResponse {
    fn from(row: ScoreRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            timestamp: row.timestamp.and_utc().to_rfc3339(),
            name: row.name,
            value: row.value,
            source: row.source,
            author_user_id: row.author_user_id,
            comment: row.comment,
            trace_id: row.trace_id,
            observation_id: row.observation_id,
            config_id: row.config_id,
            string_value: row.string_value,
            queue_id: row.queue_id,
            data_type: row.data_type,
            created_at: row.created_at.and_utc().to_rfc3339(),
            updated_at: row.updated_at.and_utc().to_rfc3339(),
        }
    }
}
/// List scores with optional trace_id filter
#[utoipa::path(
    get,
    path = "/api/scores",
    params(ListScoresQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]

/// GET /api/scores
pub(crate) async fn list_scores(
    Extension(session): Extension<Session>,
    Query(params): Query<ListScoresQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &params.project_id).await?;
    let limit = params.limit.unwrap_or(50).min(100);
    let cursor_ts = params.cursor.as_deref()
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
            .ok());

    let rows = scores::list_scores(
        &state.pool, &params.project_id,
        params.trace_id.as_deref(), cursor_ts, limit,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let has_more = rows.len() > limit as usize;
    let scores: Vec<ScoreResponse> = rows.into_iter()
        .take(limit as usize).map(ScoreResponse::from).collect();
    let next_cursor = scores.last().map(|s| s.timestamp.clone());

    Ok(Json(serde_json::json!({
        "data": { "scores": scores },
        "meta": { "cursor": next_cursor, "has_more": has_more, "total": null },
        "error": null,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_scores))
}
