//! End-to-end tests for console authentication and project scoping.
//!
//! These cover the two properties the private `/api/*` surface has to hold:
//!
//! 1. **Authentication** — no credential, a forged token, or an expired token
//!    is rejected; a cookie or Bearer token issued by `POST /api/auth/login`
//!    is accepted.
//! 2. **Authorization** — a valid session for one organization cannot read a
//!    project that belongs to another. This is the property that a no-op
//!    `verify_project_access` silently broke: `?project_id=` comes from the
//!    request, so "authenticated" is not the same as "allowed".
//!
//! Run with: `cargo test -p langfuse-api --test private_api_auth`

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;
use common::{pool, Fixture};

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn send(app: axum::Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router responded");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Reply {
        status,
        headers,
        body,
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build request")
}

fn get_with(uri: &str, name: header::HeaderName, value: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(name, value)
        .body(Body::empty())
        .expect("build request")
}

fn post_json(uri: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).expect("serialize")))
        .expect("build request")
}

fn set_cookie(reply: &Reply) -> Option<String> {
    reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("langfuse_session="))
        .map(str::to_string)
}

// ============================================================================
// Authentication
// ============================================================================

#[tokio::test]
async fn a_request_with_no_credential_is_rejected() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "no-credential").await.with_member(None).await;

    let reply = send(
        fixture.app(),
        get(&format!("/api/traces?project_id={}", fixture.project_id)),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    // The official public-API error shape, so a client can read `message`
    // instead of guessing from a bare status code.
    assert!(reply.body["message"].is_string(), "got: {}", reply.body);
    assert!(reply.body["error"].is_string(), "got: {}", reply.body);

    fixture.cleanup().await;
}

#[tokio::test]
async fn a_bearer_token_from_the_signing_key_is_accepted() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "bearer").await.with_member(None).await;

    let reply = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", fixture.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "got: {}", reply.body);

    fixture.cleanup().await;
}

#[tokio::test]
async fn the_session_cookie_is_accepted_too() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "cookie").await.with_member(None).await;

    let reply = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::COOKIE,
            &format!("langfuse_session={}", fixture.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "got: {}", reply.body);

    fixture.cleanup().await;
}

#[tokio::test]
async fn a_token_signed_with_another_key_is_rejected() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "forged").await.with_member(None).await;

    let reply = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", fixture.token_signed_with_wrong_key()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    fixture.cleanup().await;
}

#[tokio::test]
async fn an_expired_token_is_rejected() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "expired").await.with_member(None).await;

    // Well past the leeway, not one minute past `exp`: a token that expired 60
    // seconds ago is still accepted on purpose (see `EXPIRY_LEEWAY_SECONDS`),
    // and a test that used that boundary would be pinning the wrong thing.
    let expired = fixture.token_with_expiry(-(langfuse_auth::jwt::EXPIRY_LEEWAY_SECONDS as i64) - 5);

    let reply = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {expired}"),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    fixture.cleanup().await;
}

#[tokio::test]
async fn a_token_deleted_user_is_treated_as_signed_out() {
    // A JWT cannot be revoked, so `/api/auth/session` re-reads the user row.
    // Without that, deleting an account leaves its console working for the
    // token's full 30-day lifetime.
    let pool = pool().await;
    let fixture = Fixture::new(pool, "deleted-user")
        .await
        .with_member(None)
        .await;
    let cookie = format!("langfuse_session={}", fixture.token());

    let before = send(
        fixture.app(),
        get_with("/api/auth/session", header::COOKIE, &cookie),
    )
    .await;
    assert_eq!(before.body["user"]["id"], fixture.user_id());

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(fixture.user_id())
        .execute(&fixture.pool)
        .await
        .expect("delete user");

    let after = send(
        fixture.app(),
        get_with("/api/auth/session", header::COOKIE, &cookie),
    )
    .await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.body, json!({}), "a deleted user must read as signed out");

    fixture.cleanup().await;
}

#[tokio::test]
async fn a_garbage_token_is_rejected() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "garbage").await.with_member(None).await;

    let reply = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::COOKIE,
            "langfuse_session=not-a-jwt",
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    fixture.cleanup().await;
}

// ============================================================================
// Authorization — project scoping
// ============================================================================

#[tokio::test]
async fn a_session_cannot_read_a_project_in_another_organization() {
    let pool = pool().await;
    let mine = Fixture::new(pool.clone(), "mine")
        .await
        .with_member(None)
        .await;
    // A second org/project the session user has no membership in.
    let theirs = Fixture::new(pool, "theirs").await;

    let reply = send(
        mine.app(),
        get_with(
            &format!("/api/traces?project_id={}", theirs.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", mine.token()),
        ),
    )
    .await;

    assert_eq!(
        reply.status,
        StatusCode::NOT_FOUND,
        "a foreign project_id must be refused, not silently served: {}",
        reply.body
    );

    // The user's own project still works, so the check is scoping rather than
    // a blanket denial of everything after the first mismatch.
    let own = send(
        mine.app(),
        get_with(
            &format!("/api/traces?project_id={}", mine.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", mine.token()),
        ),
    )
    .await;
    assert_eq!(own.status, StatusCode::OK, "got: {}", own.body);

    mine.cleanup().await;
    theirs.cleanup().await;
}

#[tokio::test]
async fn a_foreign_project_is_404_not_403() {
    // 403 would confirm the project exists, letting any signed-in user
    // enumerate project ids. The scoped handlers answer 404.
    let pool = pool().await;
    let mine = Fixture::new(pool.clone(), "scope-mine")
        .await
        .with_member(None)
        .await;
    let theirs = Fixture::new(pool, "scope-theirs").await;

    let reply = send(
        mine.app(),
        get_with(
            &format!("/api/traces/metrics?project_id={}&trace_ids=[]", theirs.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", mine.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND, "got: {}", reply.body);

    mine.cleanup().await;
    theirs.cleanup().await;
}

#[tokio::test]
async fn api_keys_of_a_foreign_project_are_not_listed() {
    let pool = pool().await;
    let mine = Fixture::new(pool.clone(), "keys-mine")
        .await
        .with_member(None)
        .await;
    let theirs = Fixture::new(pool, "keys-theirs").await;

    let reply = send(
        mine.app(),
        get_with(
            &format!("/api/api-keys?project_id={}", theirs.project_id),
            header::AUTHORIZATION,
            &format!("Bearer {}", mine.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND, "got: {}", reply.body);
    assert!(reply.body["data"].is_null());

    mine.cleanup().await;
    theirs.cleanup().await;
}

#[tokio::test]
async fn api_keys_of_an_unknown_project_are_not_created() {
    let pool = pool().await;
    let mine = Fixture::new(pool, "keys-create").await.with_member(None).await;

    let mut req = post_json(
        "/api/api-keys",
        &json!({ "project_id": "does-not-exist", "note": "nope" }),
    );
    req.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", mine.token()).parse().unwrap(),
    );

    let reply = send(mine.app(), req).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "got: {}", reply.body);

    mine.cleanup().await;
}

// ============================================================================
// Sign in / sign out
// ============================================================================

#[tokio::test]
async fn login_verifies_the_password_and_issues_a_session() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "login-ok")
        .await
        .with_member(Some("correct-horse-battery"))
        .await;

    let reply = send(
        fixture.app(),
        post_json(
            "/api/auth/login",
            &json!({ "email": fixture.email(), "password": "correct-horse-battery" }),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "got: {}", reply.body);
    assert_eq!(reply.body["data"]["user"]["email"], fixture.email());

    let cookie = set_cookie(&reply).expect("login must set a session cookie");
    assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly: {cookie}");
    assert!(cookie.contains("Path=/"), "cookie must be site-wide: {cookie}");

    // The cookie the login just issued must actually authenticate.
    let echo = send(
        fixture.app(),
        get_with("/api/auth/session", header::COOKIE, &cookie),
    )
    .await;
    assert_eq!(echo.status, StatusCode::OK);
    assert_eq!(echo.body["user"]["id"], fixture.user_id());

    fixture.cleanup().await;
}

#[tokio::test]
async fn login_with_the_wrong_password_is_rejected() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "login-bad")
        .await
        .with_member(Some("correct-horse-battery"))
        .await;

    let reply = send(
        fixture.app(),
        post_json(
            "/api/auth/login",
            &json!({ "email": fixture.email(), "password": "wrong-password" }),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(set_cookie(&reply).is_none(), "a failed login must not set a cookie");

    fixture.cleanup().await;
}

#[tokio::test]
async fn an_unknown_email_and_a_wrong_password_are_indistinguishable() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "login-enum")
        .await
        .with_member(Some("correct-horse-battery"))
        .await;

    let wrong_password = send(
        fixture.app(),
        post_json(
            "/api/auth/login",
            &json!({ "email": fixture.email(), "password": "wrong-password" }),
        ),
    )
    .await;

    let unknown_email = send(
        fixture.app(),
        post_json(
            "/api/auth/login",
            &json!({ "email": "nobody@fixture.test", "password": "wrong-password" }),
        ),
    )
    .await;

    assert_eq!(wrong_password.status, unknown_email.status);
    assert_eq!(wrong_password.body, unknown_email.body);

    fixture.cleanup().await;
}

#[tokio::test]
async fn repeated_failures_are_throttled() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "login-throttle")
        .await
        .with_member(Some("correct-horse-battery"))
        .await;

    let body = json!({ "email": fixture.email(), "password": "wrong-password" });

    for _ in 0..langfuse_api::throttle::MAX_FAILURES {
        let attempt = send(fixture.app(), post_json("/api/auth/login", &body)).await;
        assert_eq!(attempt.status, StatusCode::UNAUTHORIZED);
    }

    let throttled = send(fixture.app(), post_json("/api/auth/login", &body)).await;
    assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS, "got: {}", throttled.body);

    // The correct password is refused too while the lockout holds — otherwise
    // the throttle would only slow an attacker down, not stop them.
    let with_right_password = send(
        fixture.app(),
        post_json(
            "/api/auth/login",
            &json!({ "email": fixture.email(), "password": "correct-horse-battery" }),
        ),
    )
    .await;
    assert_eq!(with_right_password.status, StatusCode::TOO_MANY_REQUESTS);

    fixture.cleanup().await;
}

#[tokio::test]
async fn session_without_a_cookie_is_an_empty_object_not_an_error() {
    // `{}` is the shape the UI treats as "signed out"; a 401 here would surface
    // as a failed request in the browser console on every page load.
    let pool = pool().await;
    let fixture = Fixture::new(pool, "session-anon").await;

    let reply = send(fixture.app(), get("/api/auth/session")).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, json!({}));

    fixture.cleanup().await;
}

#[tokio::test]
async fn logout_clears_the_session_cookie() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "logout").await.with_member(None).await;

    let reply = send(fixture.app(), post_json("/api/auth/logout", &json!({}))).await;

    assert_eq!(reply.status, StatusCode::OK);
    let cookie = set_cookie(&reply).expect("logout must clear the session cookie");
    assert!(
        cookie.contains("Max-Age=0"),
        "the cookie must be expired, not merely blank: {cookie}"
    );

    fixture.cleanup().await;
}

#[tokio::test]
async fn signup_provisions_an_organization_and_a_project() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "signup").await;
    let email = format!("{}@fixture.test", uuid::Uuid::new_v4().simple());

    let reply = send(
        fixture.app(),
        post_json(
            "/api/auth/signup",
            &json!({ "email": email, "name": "New Person", "password": "a-good-password" }),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "got: {}", reply.body);
    let user_id = reply.body["data"]["user"]["id"]
        .as_str()
        .expect("signup returns the new user id")
        .to_string();
    let cookie = set_cookie(&reply).expect("signup signs the new user in");

    // A user with no organization sees an empty project list and cannot reach
    // any page, so signup has to create both.
    let session = send(
        fixture.app(),
        get_with("/api/auth/session", header::COOKIE, &cookie),
    )
    .await;
    let orgs = session.body["user"]["organizations"]
        .as_array()
        .expect("session lists organizations");
    assert_eq!(orgs.len(), 1, "got: {}", session.body);
    assert_eq!(orgs[0]["role"], "OWNER");
    assert_eq!(orgs[0]["projects"].as_array().map(Vec::len), Some(1));

    // The project it created must be readable with the session it issued.
    let project_id = orgs[0]["projects"][0]["id"].as_str().unwrap();
    let traces = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={project_id}"),
            header::COOKIE,
            &cookie,
        ),
    )
    .await;
    assert_eq!(traces.status, StatusCode::OK, "got: {}", traces.body);

    // Clean up the account the endpoint created (the fixture org/project are
    // the only rows `cleanup` knows about, so remove these by hand).
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(&user_id)
        .execute(&fixture.pool)
        .await;
    let _ = sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(orgs[0]["id"].as_str().unwrap())
        .execute(&fixture.pool)
        .await;
    fixture.cleanup().await;
}

#[tokio::test]
async fn signup_rejects_a_short_password_and_a_duplicate_email() {
    let pool = pool().await;
    let fixture = Fixture::new(pool, "signup-bad")
        .await
        .with_member(None)
        .await;

    let short = send(
        fixture.app(),
        post_json(
            "/api/auth/signup",
            &json!({ "email": "short@fixture.test", "name": "S", "password": "1234" }),
        ),
    )
    .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST);

    let duplicate = send(
        fixture.app(),
        post_json(
            "/api/auth/signup",
            &json!({
                "email": fixture.email(),
                "name": "Dup",
                "password": "a-good-password",
            }),
        ),
    )
    .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);

    fixture.cleanup().await;
}

#[tokio::test]
async fn the_health_endpoint_never_requires_a_credential() {
    // Container HEALTHCHECK and orchestrator probes have no session; a 401 here
    // would make every container look unhealthy.
    let pool = pool().await;
    let fixture = Fixture::new(pool, "health").await;

    let api_health = send(fixture.app(), get("/api/public/health")).await;
    assert_eq!(api_health.status, StatusCode::OK);

    let root_health = send(fixture.app(), get("/health")).await;
    assert_eq!(root_health.status, StatusCode::OK);

    fixture.cleanup().await;
}

#[tokio::test]
async fn the_public_api_still_authenticates_with_an_api_key_not_a_session() {
    // The two credential systems must not bleed into each other: an API key is
    // not a session, and a session is not an API key.
    let pool = pool().await;
    let fixture = Fixture::new(pool, "cross-credential")
        .await
        .with_member(None)
        .await;

    let with_session = send(
        fixture.app(),
        get_with(
            "/api/public/traces",
            header::AUTHORIZATION,
            &format!("Bearer {}", fixture.token()),
        ),
    )
    .await;
    assert_eq!(with_session.status, StatusCode::UNAUTHORIZED);

    let with_api_key = send(
        fixture.app(),
        get_with("/api/public/traces", header::AUTHORIZATION, &fixture.basic_auth()),
    )
    .await;
    assert_eq!(with_api_key.status, StatusCode::OK, "got: {}", with_api_key.body);

    let private_with_api_key = send(
        fixture.app(),
        get_with(
            &format!("/api/traces?project_id={}", fixture.project_id),
            header::AUTHORIZATION,
            &fixture.basic_auth(),
        ),
    )
    .await;
    assert_eq!(private_with_api_key.status, StatusCode::UNAUTHORIZED);

    fixture.cleanup().await;
}
