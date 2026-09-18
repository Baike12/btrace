use axum::{Router, http::StatusCode, response::Json, routing::get, Extension};
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;

/// GET /api/organizations — 返回当前用户所属的所有 org
#[utoipa::path(
    get,
    path = "/api/organizations",
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer_auth" = [])),
)]
pub(crate) async fn list(
    Extension(session): Extension<Session>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = sqlx::query(
        r#"SELECT o.id as org_id, o.name as org_name, om.role::text as org_role,
           COUNT(pm.project_id) as project_count
        FROM organization_memberships om
        JOIN organizations o ON o.id = om.org_id
        LEFT JOIN project_memberships pm ON pm.org_membership_id = om.id
        WHERE om.user_id = $1
        GROUP BY o.id, o.name, om.role
        ORDER BY o.created_at DESC"#,
    )
    .bind(&session.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let orgs: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "org_id": r.get::<String, _>("org_id"),
        "org_name": r.get::<String, _>("org_name"),
        "org_role": r.get::<String, _>("org_role"),
        "project_count": r.get::<i64, _>("project_count"),
    })).collect();

    Ok(Json(serde_json::json!({
        "data": { "organizations": orgs },
        "meta": null,
        "error": null,
    })))
}

pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
