//! API key management for the console.
//!
//! Every route here takes the project from the request (body or query), so each
//! one confirms the session user may act on that project before touching a key.
//! `delete` and `update_note` take only a key id, which is worse: they resolve
//! the key's project first and check access against *that*, otherwise a
//! signed-in user could delete any key in the database by guessing an id.

use axum::{
    Extension, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, put},
};
use langfuse_auth::jwt::Session;
use sqlx::Row;

use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, serde::Deserialize)]
pub struct CreateApiKeyBody {
    note: Option<String>,
    project_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    project_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateNoteBody {
    note: Option<String>,
}

/// POST /api/api-keys — create a new API key
pub(crate) async fn create(
    Extension(session): Extension<Session>,
    State(state): State<AppState>,
    Json(body): Json<CreateApiKeyBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &body.project_id).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let public_key = format!("pk-lf-{}", uuid::Uuid::new_v4().simple());
    let secret_key = format!("sk-lf-{}", uuid::Uuid::new_v4().simple());

    let salt = std::env::var("SALT").unwrap_or_else(|_| "dev-salt".to_string());
    // Both hashes come from the shared helpers `verify_api_key` reads with.
    // Writing them here by hand (as this handler used to) means a change to the
    // hashing scheme silently produces keys that can never authenticate.
    let fast_hash = langfuse_core::api_key_hash::fast_hash_secret_key(&secret_key, &salt);
    let bcrypt_hash = langfuse_core::api_key_hash::hash_secret_key(&secret_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("hash secret key: {e}")))?;
    let display = langfuse_core::api_key_hash::display_secret_key(&secret_key);

    let row = sqlx::query(
        r#"INSERT INTO api_keys (
               id, note, public_key, hashed_secret_key, fast_hashed_secret_key,
               display_secret_key, project_id, scope, created_at
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'PROJECT', NOW())
           RETURNING created_at"#,
    )
    .bind(&id)
    .bind(&body.note)
    .bind(&public_key)
    .bind(&bcrypt_hash)
    .bind(&fast_hash)
    .bind(&display)
    .bind(&body.project_id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;

    let created_at: chrono::NaiveDateTime = row.get("created_at");

    Ok(Json(serde_json::json!({
        "data": {
            "id": id,
            "note": body.note,
            "publicKey": public_key,
            "secretKey": secret_key,
            "displaySecretKey": display,
            "createdAt": created_at.and_utc().to_rfc3339(),
            "expiresAt": null,
            "projectId": body.project_id,
        },
        "meta": null,
        "error": null,
    })))
}

/// GET /api/api-keys — list API keys for a project
pub(crate) async fn list(
    Extension(session): Extension<Session>,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;

    let rows = sqlx::query(
        r#"SELECT id, note, public_key, display_secret_key, created_at, expires_at, project_id
           FROM api_keys WHERE project_id = $1 ORDER BY created_at DESC"#,
    )
    .bind(&q.project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;

    let keys: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "note": r.get::<Option<String>, _>("note"),
                "publicKey": r.get::<String, _>("public_key"),
                "displaySecretKey": r.get::<Option<String>, _>("display_secret_key"),
                "createdAt": r.get::<Option<chrono::NaiveDateTime>, _>("created_at").map(|t| t.and_utc().to_rfc3339()),
                "expiresAt": r.get::<Option<chrono::NaiveDateTime>, _>("expires_at").map(|t| t.and_utc().to_rfc3339()),
                "projectId": r.get::<String, _>("project_id"),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "data": keys,
        "meta": null,
        "error": null,
    })))
}

/// DELETE /api/api-keys/:id — delete an API key
pub(crate) async fn delete_key(
    Extension(session): Extension<Session>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = key_project(&state, &id).await?;
    require_project_access(&state.pool, &session, &project_id).await?;

    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;

    Ok(Json(serde_json::json!({
        "data": { "success": true },
        "meta": null,
        "error": null,
    })))
}

/// PUT /api/api-keys/:id — update API key note
pub(crate) async fn update_note(
    Extension(session): Extension<Session>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateNoteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = key_project(&state, &id).await?;
    require_project_access(&state.pool, &session, &project_id).await?;

    sqlx::query("UPDATE api_keys SET note = $1 WHERE id = $2")
        .bind(&body.note)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;

    Ok(Json(serde_json::json!({
        "data": { "id": id, "note": body.note },
        "meta": null,
        "error": null,
    })))
}

/// Resolve a key's owning project.
///
/// 404 for an unknown id — matching the access check's "do not confirm what
/// exists" rule, so a missing key and someone else's key are indistinguishable.
async fn key_project(state: &AppState, id: &str) -> Result<String, (StatusCode, String)> {
    sqlx::query_scalar::<_, String>("SELECT project_id FROM api_keys WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "API key not found".to_string()))
}

fn internal(e: sqlx::Error) -> (StatusCode, String) {
    tracing::error!(error = %e, "api_keys: database error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal error".to_string(),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", put(update_note).delete(delete_key))
}
