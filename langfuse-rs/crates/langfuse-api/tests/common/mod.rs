//! Shared test fixtures for the public API integration tests.
//!
//! These tests run against the real PostgreSQL instance: the failures worth
//! catching here are integration failures (wire decoding, column mapping, enum
//! casts, NOT NULL and uniqueness constraints), which a mocked repository
//! cannot reproduce.
//!
//! **On failure, a fixture leaks.** [`Fixture::cleanup`] is an explicit call at
//! the end of each test, so a panicking assertion skips it and leaves the org,
//! project, key, and user behind. That is the price of not adding a `Drop` impl
//! that would have to block on a runtime it no longer owns. The rows are
//! prefixed and inert, but they accumulate, so clear them out with:
//!
//! ```sql
//! DELETE FROM users         WHERE email LIKE '%@fixture.test';
//! DELETE FROM organizations WHERE id LIKE 'test-org-%';       -- projects cascade
//! DELETE FROM api_keys      WHERE public_key LIKE 'pk-lf-test-%';
//! ```
//!
//! A run against a throwaway database — CI, or a scratch `createdb` — never
//! needs this.

#![allow(dead_code)] // each test binary uses a subset

use base64::Engine;
use langfuse_api::app::{create_app, AppState};
use langfuse_core::api_key_hash;
use sqlx::PgPool;
use std::sync::Arc;

pub const SALT: &str = "dev-salt-for-local-development-only";

/// A provisioned org/project/API-key triple, removed via [`Fixture::cleanup`].
pub struct Fixture {
    pub pool: PgPool,
    pub org_id: String,
    pub project_id: String,
    pub public_key: String,
    pub secret_key: String,
    /// Set by [`Fixture::with_member`]. Tests that need a console session use
    /// it; the public-API tests leave it `None`.
    pub user_id: Option<String>,
    /// The email `with_member` created, for the sign-in tests.
    pub email: Option<String>,
    /// One throttle for the fixture's whole lifetime.
    ///
    /// It has to be shared across `app()` calls: building a fresh `AppState`
    /// per request would also build a fresh `LoginThrottle`, so the failed-login
    /// counter would reset every request and the lockout test could never pass.
    /// Production shares one instance across all requests, so this is also the
    /// more faithful fixture.
    pub throttle: Arc<langfuse_api::throttle::LoginThrottle>,
}

impl Fixture {
    pub async fn new(pool: PgPool, label: &str) -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let org_id = format!("test-org-{}", suffix);
        let project_id = format!("test-project-{}", suffix);
        let public_key = format!("pk-lf-test-{}", &suffix[..12]);
        let secret_key = format!("sk-lf-test-{}", &suffix[..12]);

        sqlx::query(
            "INSERT INTO organizations (id, name, created_at, updated_at) VALUES ($1, $2, NOW(), NOW())",
        )
        .bind(&org_id)
        .bind(format!("Test Org {}", label))
        .execute(&pool)
        .await
        .expect("insert organization");

        sqlx::query(
            "INSERT INTO projects (id, name, org_id, created_at, updated_at) VALUES ($1, $2, $3, NOW(), NOW())",
        )
        .bind(&project_id)
        .bind(format!("Test Project {}", label))
        .bind(&org_id)
        .execute(&pool)
        .await
        .expect("insert project");

        // Written exactly the way `langfuse-db`'s bootstrap writes it, so this
        // fixture also pins the key creation / verification contract.
        // `api_keys` has no `updated_at` column.
        sqlx::query(
            r#"INSERT INTO api_keys (
                   id, project_id, public_key, hashed_secret_key,
                   fast_hashed_secret_key, display_secret_key,
                   note, scope, created_at
               ) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, 'test', 'PROJECT', NOW())"#,
        )
        .bind(&project_id)
        .bind(&public_key)
        .bind(api_key_hash::hash_secret_key(&secret_key).expect("hash secret"))
        .bind(api_key_hash::fast_hash_secret_key(&secret_key, SALT))
        .bind(api_key_hash::display_secret_key(&secret_key))
        .execute(&pool)
        .await
        .expect("insert api key");

        Self {
            pool,
            org_id,
            project_id,
            public_key,
            secret_key,
            user_id: None,
            email: None,
            throttle: Arc::new(langfuse_api::throttle::LoginThrottle::new()),
        }
    }

    /// Create a user who is an OWNER of this fixture's org, and therefore has
    /// access to its project.
    ///
    /// The password is optional: the session tests need a session, not a
    /// credential, and `users.password` is nullable.
    pub async fn with_member(mut self, password: Option<&str>) -> Self {
        let user_id = uuid::Uuid::new_v4().to_string();
        let email = format!("{}@fixture.test", uuid::Uuid::new_v4().simple());

        sqlx::query(
            r#"INSERT INTO users (id, name, email, password, email_verified, admin, created_at, updated_at)
               VALUES ($1, $2, $3, $4, NULL, false, NOW(), NOW())"#,
        )
        .bind(&user_id)
        .bind("Fixture User")
        .bind(&email)
        .bind(password.map(|p| langfuse_auth::password::hash_password(p).expect("hash password")))
        .execute(&self.pool)
        .await
        .expect("insert user");

        let membership_id: String = sqlx::query_scalar(
            r#"INSERT INTO organization_memberships (id, org_id, user_id, role, created_at, updated_at)
               VALUES (gen_random_uuid(), $1, $2, 'OWNER', NOW(), NOW())
               RETURNING id"#,
        )
        .bind(&self.org_id)
        .bind(&user_id)
        .fetch_one(&self.pool)
        .await
        .expect("insert org membership");

        sqlx::query(
            r#"INSERT INTO project_memberships (project_id, user_id, org_membership_id, role, created_at, updated_at)
               VALUES ($1, $2, $3, 'OWNER', NOW(), NOW())"#,
        )
        .bind(&self.project_id)
        .bind(&user_id)
        .bind(&membership_id)
        .execute(&self.pool)
        .await
        .expect("insert project membership");

        self.user_id = Some(user_id);
        self.email = Some(email);
        self
    }

    /// The fixture user's id. Panics if [`Fixture::with_member`] was not called.
    pub fn user_id(&self) -> &str {
        self.user_id
            .as_deref()
            .expect("call Fixture::with_member first")
    }

    /// The fixture user's email. Panics if [`Fixture::with_member`] was not called.
    pub fn email(&self) -> &str {
        self.email.as_deref().expect("call Fixture::with_member first")
    }

    /// A session JWT signed with the key `Fixture::app` verifies against.
    pub fn token(&self) -> String {
        self.token_with_expiry(langfuse_auth::session_cookie::SESSION_TTL_MINUTES)
    }

    /// A session JWT with a chosen lifetime, so a test can mint an expired one.
    pub fn token_with_expiry(&self, minutes: i64) -> String {
        langfuse_auth::jwt::create_token(
            self.user_id(),
            "fixture@test.dev",
            "Fixture User",
            b"test-secret",
            minutes,
        )
        .expect("sign token")
    }

    /// A credential for a *different* signing key, i.e. a forged token.
    pub fn token_signed_with_wrong_key(&self) -> String {
        langfuse_auth::jwt::create_token(
            self.user_id(),
            "fixture@test.dev",
            "Fixture User",
            b"not-the-test-secret",
            langfuse_auth::session_cookie::SESSION_TTL_MINUTES,
        )
        .expect("sign token")
    }

    pub fn basic_auth(&self) -> String {
        let raw = format!("{}:{}", self.public_key, self.secret_key);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    pub fn app(&self) -> axum::Router {
        create_app(AppState {
            pool: self.pool.clone(),
            jwt_secret: Arc::new(b"test-secret".to_vec()),
            worker: None,
            login_throttle: self.throttle.clone(),
        })
    }

    /// Delete the fixture's org and project. Child rows (api_keys, traces,
    /// observations, memberships) cascade.
    ///
    /// The fixture user, if any, is deleted too — `users` has no foreign key
    /// into `organizations`, so removing the org does not take it with it.
    pub async fn cleanup(self) {
        if let Some(user_id) = &self.user_id {
            let _ = sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(user_id)
                .execute(&self.pool)
                .await;
        }
        let _ = sqlx::query("DELETE FROM organizations WHERE id = $1")
            .bind(&self.org_id)
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(&self.project_id)
            .execute(&self.pool)
            .await;
    }
}

pub async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://baike@127.0.0.1:5432/lanfuse".to_string());
    langfuse_db::pool::init_pool(&url)
        .await
        .expect("connect to DATABASE_URL")
}
