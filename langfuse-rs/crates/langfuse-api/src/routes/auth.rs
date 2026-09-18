//! Console authentication: sign-in, session lookup, sign-out, sign-up.
//!
//! These four routes are mounted **outside** the session middleware (they are
//! how a caller obtains a session), so each one verifies whatever credential it
//! relies on itself. `session` and `logout` read the cookie; `login` and
//! `signup` check a password.

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use langfuse_auth::jwt::{Session, create_token, verify_token};
use langfuse_auth::password::{equalize_timing, hash_password, is_valid_password, verify_password};
use langfuse_auth::session_cookie::{
    SESSION_TTL_MINUTES, cleared_session_cookie, read_session_cookie, session_cookie,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::response::ApiResponse;
use crate::response::public_error;

// ============================================================================
// Request / response types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupRequest {
    pub email: String,
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// `POST /api/auth/login` — verify a password and start a session.
///
/// Failures are deliberately uniform: a missing account, an account with no
/// password (SSO-only), and a wrong password all return the same 401 body.
/// Distinguishing them would confirm which addresses have accounts.
pub(crate) async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, Response> {
    let email = normalize_email(&body.email);

    if email.is_empty() || body.password.is_empty() {
        return Err(invalid_credentials());
    }

    if let Err(retry_after) = state.login_throttle.check(&email) {
        return Err(public_error(
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many failed sign-in attempts. Try again in {retry_after} seconds."),
            "too_many_requests",
        ));
    }

    let row = sqlx::query("SELECT id, name, email, password FROM users WHERE lower(email) = $1")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "login: user lookup failed");
            internal_error()
        })?;

    let Some(row) = row else {
        // Spend the same time a real check would, so the response time does not
        // answer "does this address have an account?".
        equalize_timing();
        state.login_throttle.record_failure(&email);
        return Err(invalid_credentials());
    };

    let stored_hash: Option<String> = row.get("password");
    let password_ok = match stored_hash {
        Some(ref hash) => verify_password(&body.password, hash).unwrap_or(false),
        // An account created through SSO has no password to check against.
        None => {
            equalize_timing();
            false
        }
    };

    if !password_ok {
        state.login_throttle.record_failure(&email);
        return Err(invalid_credentials());
    }

    state.login_throttle.record_success(&email);

    let user_id: String = row.get("id");
    let user_email: Option<String> = row.get("email");
    let user_name: Option<String> = row.get("name");
    let user_email = user_email.unwrap_or_else(|| email.clone());
    let user_name = user_name.unwrap_or_default();

    let token = create_token(
        &user_id,
        &user_email,
        &user_name,
        &state.jwt_secret,
        SESSION_TTL_MINUTES,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "login: token creation failed");
        internal_error()
    })?;

    let response = ApiResponse::success(LoginResponse {
        token: token.clone(),
        user: UserInfo {
            id: user_id,
            email: user_email,
            name: user_name,
        },
    });

    Ok(with_session_cookie(Json(response).into_response(), token))
}

/// `POST /api/auth/signup` — create an account, its organization, and a first
/// project, then start a session.
///
/// The organization and project are created here rather than left to an
/// onboarding step: a user with no organization sees an empty project list and
/// cannot reach any of the console.
///
/// Set `LANGFUSE_DISABLE_SIGNUP=true` to close registration on an instance
/// where accounts are provisioned by `LANGFUSE_INIT_*` instead.
pub(crate) async fn signup(
    State(state): State<AppState>,
    Json(body): Json<SignupRequest>,
) -> Result<Response, Response> {
    if signup_disabled() {
        return Err(public_error(
            StatusCode::FORBIDDEN,
            "Sign-up is disabled on this instance",
            "forbidden",
        ));
    }

    let email = normalize_email(&body.email);
    let name = body.name.trim();

    if !email.contains('@') {
        return Err(bad_request("A valid email address is required"));
    }
    if !is_valid_password(&body.password) {
        return Err(bad_request("Password must be at least 8 characters"));
    }

    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = $1")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "signup: user lookup failed");
            internal_error()
        })?;

    if existing.is_some() {
        // Same uniform shape as a bad login: an open signup endpoint that says
        // "already registered" is an account-enumeration oracle.
        return Err(public_error(
            StatusCode::CONFLICT,
            "Could not create the account with those details",
            "conflict",
        ));
    }

    let password_hash = hash_password(&body.password).map_err(|e| {
        tracing::error!(error = %e, "signup: password hashing failed");
        internal_error()
    })?;

    let user_id = uuid::Uuid::new_v4().to_string();
    let display_name: String = if name.is_empty() {
        email.clone()
    } else {
        name.to_string()
    };

    // One transaction: a half-created account (user with no membership) would
    // render an empty console that the user cannot fix from the UI.
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "signup: begin transaction failed");
        internal_error()
    })?;

    // `email_verified` is a timestamp, not a flag. `NULL` is the honest value:
    // nothing verified the address, and this fork has no verification flow.
    sqlx::query(
        r#"INSERT INTO users (id, name, email, password, email_verified, admin, created_at, updated_at)
           VALUES ($1, $2, $3, $4, NULL, false, NOW(), NOW())"#,
    )
    .bind(&user_id)
    .bind(&display_name)
    .bind(&email)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "signup: insert user failed");
        internal_error()
    })?;

    let org_id = uuid::Uuid::new_v4().to_string();
    let org_name = format!("{display_name}'s Organization");

    sqlx::query(
        r#"INSERT INTO organizations (id, name, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())"#,
    )
    .bind(&org_id)
    .bind(&org_name)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "signup: insert organization failed");
        internal_error()
    })?;

    let org_membership_id: String = sqlx::query_scalar(
        r#"INSERT INTO organization_memberships (id, org_id, user_id, role, created_at, updated_at)
           VALUES (gen_random_uuid(), $1, $2, 'OWNER', NOW(), NOW())
           RETURNING id"#,
    )
    .bind(&org_id)
    .bind(&user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "signup: insert org membership failed");
        internal_error()
    })?;

    let project_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"INSERT INTO projects (id, name, org_id, created_at, updated_at)
           VALUES ($1, 'Default Project', $2, NOW(), NOW())"#,
    )
    .bind(&project_id)
    .bind(&org_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "signup: insert project failed");
        internal_error()
    })?;

    sqlx::query(
        r#"INSERT INTO project_memberships (project_id, user_id, org_membership_id, role, created_at, updated_at)
           VALUES ($1, $2, $3, 'OWNER', NOW(), NOW())"#,
    )
    .bind(&project_id)
    .bind(&user_id)
    .bind(&org_membership_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "signup: insert project membership failed");
        internal_error()
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "signup: commit failed");
        internal_error()
    })?;

    let token = create_token(
        &user_id,
        &email,
        &display_name,
        &state.jwt_secret,
        SESSION_TTL_MINUTES,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "signup: token creation failed");
        internal_error()
    })?;

    let response = ApiResponse::success(LoginResponse {
        token: token.clone(),
        user: UserInfo {
            id: user_id,
            email,
            name: display_name,
        },
    });

    Ok(with_session_cookie(Json(response).into_response(), token))
}

/// `GET /api/auth/session` — the current session, or `{}` when signed out.
///
/// `{}` is the shape NextAuth uses for "nobody is signed in", and what the
/// UI's `fetchSession` treats as unauthenticated. Returning a synthetic user
/// here instead would make every page render as if signed in.
///
/// The user row is re-read on every call rather than being reconstructed from
/// the token's claims. A JWT stays valid for its full lifetime with no way to
/// revoke it, so without this check a deleted — or demoted — account would keep
/// a working console for up to 30 days. The UI's auth guard already has a
/// branch for "session exists but the user does not"; this is what feeds it.
pub(crate) async fn session(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    let Some(token) = bearer_or_cookie(&headers) else {
        return Json(serde_json::json!({})).into_response();
    };

    let Ok(claims) = verify_token(&token, &state.jwt_secret) else {
        // An expired or forged cookie is not an error the client can act on;
        // answering `{}` sends it to the sign-in page.
        return Json(serde_json::json!({})).into_response();
    };
    let session = Session::from(claims);

    let user = match sqlx::query(
        "SELECT name, email, email_verified, admin, feature_flags FROM users WHERE id = $1",
    )
    .bind(&session.user_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::warn!(user_id = %session.user_id, "session: token names a user that no longer exists");
            return Json(serde_json::json!({})).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, user_id = %session.user_id, "session: user lookup failed");
            // Fail closed: an unreachable database must not present the caller
            // as signed in.
            return Json(serde_json::json!({})).into_response();
        }
    };

    let orgs = load_user_orgs(&state.pool, &session.user_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, user_id = %session.user_id, "session: loading organizations failed");
            Vec::new()
        });

    let expires = chrono::Utc::now() + chrono::Duration::minutes(SESSION_TTL_MINUTES);

    Json(serde_json::json!({
        "user": {
            "id": session.user_id,
            // The row is the source of truth; the token's copy can be stale.
            "email": row_opt_string(&user, "email").unwrap_or(session.email),
            "name": row_opt_string(&user, "name").unwrap_or(session.name),
            // A timestamp, matching the column and what the reset-password page
            // parses. `null` means "never verified".
            "emailVerified": user
                .get::<Option<chrono::NaiveDateTime>, _>("email_verified")
                .map(|t| t.and_utc().to_rfc3339()),
            "organizations": orgs,
            "featureFlags": user.get::<Vec<String>, _>("feature_flags"),
            "admin": user.get::<bool, _>("admin"),
        },
        "environment": { "selfHostedInstancePlan": null },
        "expires": expires.to_rfc3339(),
    }))
    .into_response()
}

fn row_opt_string(row: &sqlx::postgres::PgRow, column: &str) -> Option<String> {
    row.get::<Option<String>, _>(column)
}

/// `POST /api/auth/logout` — clear the session cookie.
///
/// Unauthenticated by design: signing out is idempotent, and requiring a valid
/// session to clear an invalid one would strand a user with a stale cookie in a
/// redirect loop.
pub(crate) async fn logout() -> Response {
    let mut response = Json(ApiResponse::<()>::success(())).into_response();
    if let Ok(value) = HeaderValue::from_str(&cleared_session_cookie().to_string()) {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}

// ============================================================================
// Helpers
// ============================================================================

fn with_session_cookie(mut response: Response, token: String) -> Response {
    match HeaderValue::from_str(&session_cookie(token).to_string()) {
        Ok(value) => {
            response.headers_mut().append(SET_COOKIE, value);
            response
        }
        Err(e) => {
            // The cookie value is a JWT (base64url + dots), which is always a
            // valid header value; reaching here means something is badly wrong,
            // and handing back a session the browser never stored would look
            // like a successful sign-in that evaporates on the next request.
            tracing::error!(error = %e, "failed to build Set-Cookie header");
            public_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not start a session",
                "internal_error",
            )
        }
    }
}

fn bearer_or_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let from_cookie = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(read_session_cookie);

    if from_cookie.is_some() {
        return from_cookie;
    }

    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string)
        .filter(|t| !t.is_empty())
}

/// Emails are stored lower-cased (bootstrap and the UI both do this), so
/// normalizing here keeps a login for `A@B.com` finding the row for `a@b.com`.
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn signup_disabled() -> bool {
    matches!(
        std::env::var("LANGFUSE_DISABLE_SIGNUP").as_deref(),
        Ok("true") | Ok("1")
    )
}

fn invalid_credentials() -> Response {
    public_error(
        StatusCode::UNAUTHORIZED,
        "Invalid email or password",
        "unauthorized",
    )
}

fn bad_request(message: &str) -> Response {
    public_error(StatusCode::BAD_REQUEST, message, "bad_request")
}

fn internal_error() -> Response {
    public_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal error",
        "internal_error",
    )
}

/// Load the organizations (and their projects) a user belongs to.
///
/// This is what the console renders its project picker from. The `role` is the
/// user's real role in the organization — reporting a fixed `OWNER` would give
/// the UI permission hints that do not match what the API enforces.
async fn load_user_orgs(pool: &PgPool, user_id: &str) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    use std::collections::HashMap;

    let rows = sqlx::query(
        r#"SELECT om.org_id, om.role::text AS org_role, o.name AS org_name,
                  pm.project_id, pm.role::text AS project_role, p.name AS project_name
           FROM organization_memberships om
           JOIN organizations o ON o.id = om.org_id
           LEFT JOIN project_memberships pm
                  ON pm.user_id = om.user_id AND pm.org_membership_id = om.id
           LEFT JOIN projects p ON p.id = pm.project_id AND p.deleted_at IS NULL
           WHERE om.user_id = $1"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    struct Org {
        id: String,
        name: String,
        role: String,
        projects: Vec<serde_json::Value>,
    }

    let mut orgs: HashMap<String, Org> = HashMap::new();
    for row in &rows {
        let org_id: String = row.get("org_id");
        let entry = orgs.entry(org_id.clone()).or_insert_with(|| Org {
            id: org_id,
            name: row.get("org_name"),
            role: row.get("org_role"),
            projects: Vec::new(),
        });

        let project_id: Option<String> = row.get("project_id");
        let project_name: Option<String> = row.get("project_name");
        let project_role: Option<String> = row.get("project_role");

        if let (Some(id), Some(name), Some(role)) = (project_id, project_name, project_role) {
            entry.projects.push(serde_json::json!({
                "id": id, "name": name, "role": role,
            }));
        }
    }

    let mut orgs: Vec<Org> = orgs.into_values().collect();
    // HashMap iteration order is arbitrary; a stable order keeps the project
    // picker from reshuffling between requests.
    orgs.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

    for org in &mut orgs {
        org.projects
            .sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }

    Ok(orgs
        .into_iter()
        .map(|o| {
            serde_json::json!({
                "id": o.id,
                "name": o.name,
                "role": o.role,
                "projects": o.projects,
            })
        })
        .collect())
}

// ============================================================================
// Router
// ============================================================================

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/signup", post(signup))
        .route("/session", get(session))
        .route("/logout", post(logout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_normalized_before_lookup() {
        assert_eq!(normalize_email("  QA-Test@Langfuse.DEV "), "qa-test@langfuse.dev");
    }
}
