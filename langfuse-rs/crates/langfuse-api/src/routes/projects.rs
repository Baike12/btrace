use axum::{Router, http::StatusCode, response::Json, routing::{get, post}, Extension, extract::Path};
use sqlx::Row;
use langfuse_auth::jwt::Session;
use crate::app::AppState;


/// POST /api/projects — 创建新项目
#[derive(Debug, serde::Deserialize)]
pub struct CreateProjectQuery {
    organization_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateProjectBody {
    name: String,
}

pub(crate) async fn create(
    Extension(session): Extension<Session>,
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CreateProjectQuery>,
    axum::extract::Json(body): axum::extract::Json<CreateProjectBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // 1. Look up user's organization membership
    let org_member = sqlx::query(
        "SELECT id, role::text FROM organization_memberships WHERE org_id = $1 AND user_id = $2"
    )
    .bind(&q.organization_id)
    .bind(&session.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let org_membership = org_member
        .ok_or_else(|| (StatusCode::FORBIDDEN, "Not a member of this organization".to_string()))?;

    let om_id: String = org_membership.get("id");
    let om_role: String = org_membership.get("role");

    // 2. Generate project ID and insert project
    let project_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO projects (id, name, org_id) VALUES ($1, $2, $3)"
    )
    .bind(&project_id)
    .bind(&body.name)
    .bind(&q.organization_id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 3. Create project membership (inherit org role or default to MEMBER)
    let role = if om_role == "ADMIN" || om_role == "OWNER" { "OWNER" } else { "MEMBER" };

    sqlx::query(
        "INSERT INTO project_memberships (project_id, user_id, org_membership_id, role) VALUES ($1, $2, $3, $4::\"Role\")"
    )
    .bind(&project_id)
    .bind(&session.user_id)
    .bind(&om_id)
    .bind(role)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "data": {
            "id": project_id,
            "name": body.name,
            "org_id": q.organization_id,
            "role": role,
        },
        "meta": null,
        "error": null,
    })))
}

/// GET /api/projects — 返回当前用户可访问的所有 project
#[utoipa::path(
    get,
    path = "/api/projects",
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
        r#"SELECT p.id as project_id, p.name as project_name, pm.role::text as project_role,
           o.id as org_id, o.name as org_name
        FROM project_memberships pm
        JOIN projects p ON p.id = pm.project_id
        JOIN organization_memberships om ON om.id = pm.org_membership_id
        JOIN organizations o ON o.id = om.org_id
        WHERE pm.user_id = $1
        ORDER BY p.created_at DESC"#,
    )
    .bind(&session.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let projects: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "project_id": r.get::<String, _>("project_id"),
        "project_name": r.get::<String, _>("project_name"),
        "project_role": r.get::<String, _>("project_role"),
        "org_id": r.get::<String, _>("org_id"),
        "org_name": r.get::<String, _>("org_name"),
    })).collect();

    Ok(Json(serde_json::json!({
        "data": { "projects": projects },
        "meta": null,
        "error": null,
    })))
}

/// POST /api/project/:id/visit — no-op beacon for sentinel cookie routing
pub(crate) async fn visit(
    Path(_project_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}/visit", post(visit))
}
