//! Startup bootstrap — auto-provision org/project/user/api-key on first run.
//! Equivalent to the removed web/src/initialize.ts.
//!
//! Environment variables:
//! - LANGFUSE_INIT_ORG_ID, LANGFUSE_INIT_ORG_NAME
//! - LANGFUSE_INIT_PROJECT_ID, LANGFUSE_INIT_PROJECT_NAME, LANGFUSE_INIT_PROJECT_RETENTION
//! - LANGFUSE_INIT_PROJECT_PUBLIC_KEY, LANGFUSE_INIT_PROJECT_SECRET_KEY
//! - LANGFUSE_INIT_USER_EMAIL, LANGFUSE_INIT_USER_NAME, LANGFUSE_INIT_USER_PASSWORD
//! - LANGFUSE_INIT_ORG_MEMBER_EMAIL (comma-separated; binds existing accounts
//!   to the provisioned org/project as OWNER)

use anyhow::Context;
use sqlx::PgPool;

pub async fn run_bootstrap(pool: &PgPool) -> anyhow::Result<()> {
    let org_id = std::env::var("LANGFUSE_INIT_ORG_ID").ok();
    let org_name = std::env::var("LANGFUSE_INIT_ORG_NAME")
        .ok()
        .unwrap_or_else(|| "Default Organization".into());

    let project_id = std::env::var("LANGFUSE_INIT_PROJECT_ID").ok();
    let project_name = std::env::var("LANGFUSE_INIT_PROJECT_NAME")
        .ok()
        .unwrap_or_else(|| "My Project".into());
    let project_retention: Option<i32> = std::env::var("LANGFUSE_INIT_PROJECT_RETENTION")
        .ok()
        .and_then(|v| v.parse().ok());

    let project_public_key = std::env::var("LANGFUSE_INIT_PROJECT_PUBLIC_KEY").ok();
    let project_secret_key = std::env::var("LANGFUSE_INIT_PROJECT_SECRET_KEY").ok();

    let user_email = std::env::var("LANGFUSE_INIT_USER_EMAIL").ok();
    let user_name = std::env::var("LANGFUSE_INIT_USER_NAME").ok();
    let user_password = std::env::var("LANGFUSE_INIT_USER_PASSWORD").ok();

    // If no org_id is configured, skip bootstrap entirely
    let Some(ref org_id) = org_id else {
        tracing::info!(
            "bootstrap: LANGFUSE_INIT_ORG_ID not set, skipping auto-provisioning"
        );
        return Ok(());
    };

    tracing::info!("bootstrap: running auto-provisioning for org_id={}", org_id);

    // 1. Upsert organization
    sqlx::query(
        r#"INSERT INTO organizations (id, name, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET name = $2, updated_at = NOW()"#,
    )
    .bind(org_id)
    .bind(&org_name)
    .execute(pool)
    .await
    .context("bootstrap: upsert organization")?;
    tracing::info!("bootstrap: organization '{}' ready", org_name);

    // 2. Upsert project (if configured)
    if let Some(ref project_id) = project_id {
        sqlx::query(
            r#"INSERT INTO projects (id, name, org_id, retention_days, created_at, updated_at)
               VALUES ($1, $2, $3, $4, NOW(), NOW())
               ON CONFLICT (id) DO UPDATE SET name = $2, retention_days = $4, updated_at = NOW()"#,
        )
        .bind(project_id)
        .bind(&project_name)
        .bind(org_id)
        .bind(project_retention)
        .execute(pool)
        .await
        .context("bootstrap: upsert project")?;
        tracing::info!("bootstrap: project '{}' ready", project_name);

        // 3. Create API keys (if both keys configured)
        //
        // Idempotent: a key pair that already exists is left untouched.
        // Recreating it on every boot would rotate the credential out from
        // under any client that has it configured (LexQA reads its
        // LANGFUSE_SECRET_KEY from its own .env), so a Langfuse restart would
        // silently break ingestion until the client was restarted too.
        if let (Some(ref pk), Some(ref sk)) = (&project_public_key, &project_secret_key) {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM api_keys WHERE public_key = $1",
            )
            .bind(pk)
            .fetch_optional(pool)
            .await
            .context("bootstrap: look up existing api key")?;

            if existing.is_some() {
                tracing::info!(
                    "bootstrap: api key '{}' already exists, leaving it unchanged",
                    pk
                );
            } else {
                // One row holds both halves of the pair and both hashes, which
                // is the shape `langfuse_auth::api_key::verify_api_key` reads:
                // it selects by public_key and then compares
                // fast_hashed_secret_key, falling back to bcrypt over
                // hashed_secret_key. Writing the halves as two rows (the
                // previous behaviour) can never authenticate.
                let salt = std::env::var("SALT").unwrap_or_else(|_| "dev-salt".to_string());
                let fast_hash = langfuse_core::api_key_hash::fast_hash_secret_key(sk, &salt);
                let bcrypt_hash = langfuse_core::api_key_hash::hash_secret_key(sk)
                    .map_err(|e| anyhow::anyhow!("bootstrap: hash secret key: {}", e))?;
                let display = langfuse_core::api_key_hash::display_secret_key(sk);

                // Note: `api_keys` is the one table without an `updated_at`
                // column — including it fails the insert and aborts startup.
                sqlx::query(
                    r#"INSERT INTO api_keys (
                           id, project_id, public_key, hashed_secret_key,
                           fast_hashed_secret_key, display_secret_key,
                           note, scope, created_at
                       )
                       VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, 'bootstrap', 'PROJECT', NOW())"#,
                )
                .bind(project_id)
                .bind(pk)
                .bind(&bcrypt_hash)
                .bind(&fast_hash)
                .bind(&display)
                .execute(pool)
                .await
                .context("bootstrap: insert api key")?;

                tracing::info!("bootstrap: api key '{}' created for project '{}'", pk, project_name);
            }
        }
    }

    // 4. Create user (if email + password configured)
    if let (Some(ref email), Some(ref user_name), Some(ref password)) =
        (&user_email, &user_name, &user_password)
    {
        let user_id = if let Some(row) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM users WHERE email = $1",
        )
        .bind(email.to_lowercase())
        .fetch_optional(pool)
        .await
        .context("bootstrap: lookup user")?
        {
            tracing::info!("bootstrap: user '{}' already exists, skipping creation", email);
            row
        } else {
            let password_hash = bcrypt::hash(password, 12)
                .map_err(|e| anyhow::anyhow!("bootstrap: password hash failed: {}", e))?;

            let new_user_id = uuid::Uuid::new_v4().to_string();
            // `email_verified` is a **timestamp** (`DateTime?` in the Prisma
            // model), not a boolean: an operator who sets
            // `LANGFUSE_INIT_USER_EMAIL` is asserting the address, so it is
            // stamped as verified now. Writing `true` here fails the insert
            // with `column "email_verified" is of type timestamp without time
            // zone but expression is of type boolean`.
            sqlx::query(
                r#"INSERT INTO users (id, name, email, password, email_verified, admin, created_at, updated_at)
                   VALUES ($1, $2, $3, $4, NOW(), true, NOW(), NOW())"#,
            )
            .bind(&new_user_id)
            .bind(user_name)
            .bind(email.to_lowercase())
            .bind(&password_hash)
            .execute(pool)
            .await
            .context("bootstrap: insert user")?;
            tracing::info!("bootstrap: user '{}' created", email);
            new_user_id
        };

        // 5. Create organization membership (OWNER)
        let org_membership_id = upsert_org_membership(pool, org_id, &user_id).await?;

        // 6. Create project membership (OWNER) — if project was configured
        if let Some(ref project_id) = project_id {
            upsert_project_membership(pool, project_id, &user_id, &org_membership_id).await?;
        }

        tracing::info!("bootstrap: user '{}' memberships ready", email);
    }

    // 7. Grant existing users access to the provisioned org.
    //
    // `LANGFUSE_INIT_*` creates a fresh organization, which nobody belongs to —
    // so the UI shows an empty project list until someone signs up inside it.
    // An operator who already has an account derives no visible benefit from the
    // auto-provisioned org, and OTLP-ingested data stays unreachable in the
    // browser. This variable binds such accounts to the org (and to the
    // project) as OWNER.
    //
    // Accepts a comma-separated list. Unknown addresses are skipped rather than
    // taken as an instruction to create a user with no password, which would be
    // an unauthenticated account.
    if let Ok(members) = std::env::var("LANGFUSE_INIT_ORG_MEMBER_EMAIL") {
        for email in members
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            let email = email.to_lowercase();

            let Some(user_id) = sqlx::query_scalar::<_, String>(
                "SELECT id FROM users WHERE email = $1",
            )
            .bind(&email)
            .fetch_optional(pool)
            .await
            .context("bootstrap: look up org member")?
            else {
                tracing::warn!(
                    "bootstrap: LANGFUSE_INIT_ORG_MEMBER_EMAIL '{}' matches no user, skipping",
                    email
                );
                continue;
            };

            let org_membership_id = upsert_org_membership(pool, org_id, &user_id).await?;

            if let Some(ref project_id) = project_id {
                upsert_project_membership(pool, project_id, &user_id, &org_membership_id).await?;
            }

            tracing::info!("bootstrap: '{}' added to org '{}' as OWNER", email, org_id);
        }
    }

    tracing::info!("bootstrap: done");
    Ok(())
}

/// Upsert an OWNER organization membership and return its id.
///
/// The id is needed by [`upsert_project_membership`]: `project_memberships`
/// carries a NOT NULL `org_membership_id` foreign key, so a project membership
/// cannot be created without it.
async fn upsert_org_membership(
    pool: &PgPool,
    org_id: &str,
    user_id: &str,
) -> anyhow::Result<String> {
    let id: String = sqlx::query_scalar(
        r#"INSERT INTO organization_memberships (id, org_id, user_id, role, created_at, updated_at)
           VALUES (gen_random_uuid(), $1, $2, 'OWNER', NOW(), NOW())
           ON CONFLICT (org_id, user_id) DO UPDATE SET role = 'OWNER', updated_at = NOW()
           RETURNING id"#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("bootstrap: upsert org membership")?;

    Ok(id)
}

/// Upsert an OWNER project membership.
///
/// `project_memberships` has no `id` column of its own — its primary key is
/// `(project_id, user_id)` plus a NOT NULL `org_membership_id` reference. The
/// previous `INSERT ... (id, project_id, user_id, ...)` shape failed on every
/// run, so bootstrap could never complete.
async fn upsert_project_membership(
    pool: &PgPool,
    project_id: &str,
    user_id: &str,
    org_membership_id: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO project_memberships (project_id, user_id, org_membership_id, role, created_at, updated_at)
           VALUES ($1, $2, $3, 'OWNER', NOW(), NOW())
           ON CONFLICT (project_id, user_id) DO UPDATE SET
               role = 'OWNER',
               org_membership_id = EXCLUDED.org_membership_id,
               updated_at = NOW()"#,
    )
    .bind(project_id)
    .bind(user_id)
    .bind(org_membership_id)
    .execute(pool)
    .await
    .context("bootstrap: upsert project membership")?;

    Ok(())
}
