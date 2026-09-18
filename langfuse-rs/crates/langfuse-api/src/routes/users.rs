use axum::{Extension, Router, extract::Query, http::StatusCode, response::Json, routing::get};
use langfuse_auth::jwt::Session;
use serde::Deserialize;
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListQuery {
    pub limit: Option<i64>,
}

/// GET /api/users — users the caller shares an organization with.
///
/// Scoped to co-members rather than every row in `users`. The previous version
/// selected the whole table, which exposes every account on the instance —
/// including its email address and admin flag — to any signed-in user, and is
/// the natural pivot for privilege escalation once one account is compromised.
#[utoipa::path(
    get,
    path = "/api/users",
    params(ListQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]
pub(crate) async fn list(
    Extension(session): Extension<Session>,
    Query(q): Query<ListQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 100);

    let rows = sqlx::query(
        r#"SELECT DISTINCT u.id, u.name, u.email, u.email_verified, u.image, u.admin,
                  u.feature_flags, u.created_at, u.updated_at
           FROM users u
           JOIN organization_memberships mine ON mine.user_id = $1
           JOIN organization_memberships theirs
                ON theirs.org_id = mine.org_id AND theirs.user_id = u.id
           ORDER BY u.created_at DESC
           LIMIT $2"#,
    )
    .bind(&session.user_id)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let has_more = rows.len() > limit as usize;
    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .take(limit as usize)
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<Option<String>, _>("name"),
                "email": r.get::<Option<String>, _>("email"),
                "email_verified": r.get::<Option<chrono::NaiveDateTime>, _>("email_verified").map(|t| t.and_utc().to_rfc3339()),
                "image": r.get::<Option<String>, _>("image"),
                "admin": r.get::<bool, _>("admin"),
                "feature_flags": r.get::<Vec<String>, _>("feature_flags"),
                "created_at": r.get::<chrono::NaiveDateTime, _>("created_at").and_utc().to_rfc3339(),
                "updated_at": r.get::<chrono::NaiveDateTime, _>("updated_at").and_utc().to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "data": { "users": data },
        "meta": { "cursor": null, "has_more": has_more, "total": null },
        "error": null,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}
