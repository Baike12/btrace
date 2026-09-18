//! Session authentication for the console's private `/api/*` routes.
//!
//! The browser signs in through `POST /api/auth/login`, which sets an
//! `HttpOnly` session cookie holding a JWT signed with `JWT_SECRET`. Every
//! subsequent call is verified here and the resulting [`Session`] is injected
//! as a request extension.
//!
//! Two things this middleware deliberately does **not** do:
//!
//! * It does not fall back to a default identity. An absent, malformed, or
//!   expired credential is a 401, never "the local dev user".
//! * It does not treat a valid session as authorization for a particular
//!   project. `project_id` arrives as a query parameter on nearly every private
//!   route, so each handler must *also* call [`require_project_access`] to
//!   confirm the session user belongs to the project's organization.

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use langfuse_auth::jwt::{Session, verify_token};
use langfuse_auth::rbac::user_can_access_project;
use sqlx::PgPool;

use crate::app::AppState;
use crate::response::public_error;

/// Verify the session JWT and inject the resulting [`Session`].
///
/// Accepts the credential from either the `langfuse_session` cookie (what the
/// browser sends) or `Authorization: Bearer <jwt>` (what curl and integration
/// tests use, and what keeps the API usable from non-browser clients).
pub async fn verify_session(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let Some(token) = session_token(&req) else {
        return Err(unauthorized("No session. Sign in first."));
    };

    let claims = verify_token(&token, &state.jwt_secret)
        .map_err(|e| {
            // The specific reason (expired vs. bad signature) is useful in the
            // logs but is not echoed to the client: telling an unauthenticated
            // caller *why* their forged token failed only helps them.
            tracing::debug!(error = %e, "session token rejected");
            unauthorized("Invalid or expired session. Sign in again.")
        })?;

    req.extensions_mut().insert(Session::from(claims));
    Ok(next.run(req).await)
}

/// Pull the session JWT out of the request, cookie first.
fn session_token(req: &Request<axum::body::Body>) -> Option<String> {
    let by_cookie = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(langfuse_auth::session_cookie::read_session_cookie);

    if by_cookie.is_some() {
        return by_cookie;
    }

    req.headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string)
        .filter(|t| !t.is_empty())
}

/// Confirm the session user may act on `project_id`.
///
/// Call this at the top of every private handler that takes a `project_id`.
/// The value comes from the request, so skipping this makes the route an
/// IDOR: `?project_id=<someone else's>` returns their data.
///
/// Returns 404 rather than 403 for a project the user is not in. The
/// distinction matters: a 403 confirms the project exists, which lets any
/// signed-in user enumerate project ids.
///
/// The error type matches this API's existing handlers (`(StatusCode, String)`),
/// which the console reads as plain text; the JSON `{message, error}` body is
/// reserved for `/api/public/*`, where machine clients parse it.
pub async fn require_project_access(
    pool: &PgPool,
    session: &Session,
    project_id: &str,
) -> Result<(), (StatusCode, String)> {
    match user_can_access_project(pool, &session.user_id, project_id).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            tracing::warn!(
                user_id = %session.user_id,
                project_id = %project_id,
                "session user is not a member of the project's organization"
            );
            Err((StatusCode::NOT_FOUND, "Project not found".to_string()))
        }
        Err(e) => {
            tracing::error!(error = %e, project_id = %project_id, "project access check failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            ))
        }
    }
}

fn unauthorized(message: &str) -> Response {
    public_error(StatusCode::UNAUTHORIZED, message, "unauthorized")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn req_with(headers: &[(&str, &str)]) -> Request<Body> {
        let mut b = Request::builder().uri("/api/traces");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    #[test]
    fn accepts_a_bearer_token() {
        let req = req_with(&[("authorization", "Bearer a.b.c")]);
        assert_eq!(session_token(&req).as_deref(), Some("a.b.c"));
    }

    #[test]
    fn accepts_the_session_cookie() {
        let req = req_with(&[("cookie", "langfuse_session=a.b.c")]);
        assert_eq!(session_token(&req).as_deref(), Some("a.b.c"));
    }

    #[test]
    fn prefers_the_cookie_when_both_are_present() {
        let req = req_with(&[
            ("cookie", "langfuse_session=cookie.token"),
            ("authorization", "Bearer header.token"),
        ]);
        assert_eq!(session_token(&req).as_deref(), Some("cookie.token"));
    }

    #[test]
    fn missing_or_blank_credentials_yield_no_token() {
        assert!(session_token(&req_with(&[])).is_none());
        assert!(session_token(&req_with(&[("authorization", "Bearer ")])).is_none());
        assert!(session_token(&req_with(&[("authorization", "Basic zzz")])).is_none());
        assert!(session_token(&req_with(&[("cookie", "other=1")])).is_none());
    }
}
