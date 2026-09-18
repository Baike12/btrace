use sqlx::PgPool;

use langfuse_core::{AppError, Result};

/// API Key 验证结果
///
/// There is deliberately no `is_ingestion_suspended` here: upstream Cloud has a
/// column for it, this fork's `api_keys` table does not, so the field could
/// only ever have been `false` — including a 403 branch in the middleware that
/// could never fire. Add both together if the column is ever introduced.
#[derive(Debug, Clone)]
pub struct ApiKeyScope {
    /// `None` for an organization-scoped key, which no route accepts yet.
    pub project_id: Option<String>,
    pub org_id: Option<String>,
    pub scope: String,
    pub api_key_id: String,
}

/// 解析 Basic Auth header (publicKey:secretKey)
pub fn parse_basic_auth(header: &str) -> Option<(String, String)> {
    let encoded = header.strip_prefix("Basic ")?;
    let decoded = base64_decode(encoded)?;
    let mut parts = decoded.splitn(2, ':');
    let public_key = parts.next()?.to_string();
    let secret_key = parts.next()?.to_string();
    Some((public_key, secret_key))
}

/// 解析 Bearer Auth header
pub fn parse_bearer_auth(header: &str) -> Option<String> {
    header.strip_prefix("Bearer ").map(String::from)
}

/// Fast-hash for an API key secret: `SHA256(SHA256(secretKey) + SALT)`.
///
/// Thin re-export of [`langfuse_core::api_key_hash::fast_hash_secret_key`] so
/// key *creation* and key *verification* cannot drift apart.
pub use langfuse_core::api_key_hash::fast_hash_secret_key;

/// bcrypt hash for an API key secret — see
/// [`langfuse_core::api_key_hash::hash_secret_key`].
pub use langfuse_core::api_key_hash::hash_secret_key;

/// Display form of a secret key — see
/// [`langfuse_core::api_key_hash::display_secret_key`].
pub use langfuse_core::api_key_hash::display_secret_key;

/// 两阶段 API Key 验证：先 SHA-256 fast hash → fallback bcrypt
pub async fn verify_api_key(
    pool: &PgPool,
    public_key: &str,
    secret_key: &str,
) -> Result<ApiKeyScope> {
    let salt = std::env::var("SALT").unwrap_or_else(|_| "dev-salt".to_string());

    // 1. 计算 SHA-256 fast hash: SHA256(SHA256(secretKey) + SALT)
    let fast_hash = fast_hash_secret_key(secret_key, &salt);

    // 2. 查数据库
    tracing::debug!(public_key = %public_key, "verify_api_key: looking up public_key");
    let key = sqlx::query_as::<_, ApiKeyRow>(
        r#"SELECT id, hashed_secret_key, fast_hashed_secret_key,
           scope::text as scope, project_id, organization_id as org_id,
           expires_at
        FROM api_keys WHERE public_key = $1"#,
    )
    .bind(public_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!(public_key = %public_key, error = %e, "verify_api_key: database query error");
        AppError::Database(e)
    })?
    .ok_or_else(|| {
        tracing::warn!(public_key = %public_key, "verify_api_key: API key not found in database");
        AppError::Unauthorized("Invalid API key".to_string())
    })?;

    // 3. 检查过期
    if let Some(expires_at) = key.expires_at {
        if expires_at < chrono::Utc::now().naive_utc() {
            return Err(AppError::Unauthorized("API key expired".to_string()));
        }
    }

    // 4. Fast hash 验证
    if let Some(ref stored_fast_hash) = key.fast_hashed_secret_key {
        tracing::debug!(computed = %fast_hash, stored = %stored_fast_hash, "verify_api_key: comparing fast hashes");
        if fast_hash == *stored_fast_hash {
            // 更新 last_used_at
            update_last_used(pool, &key.id).await?;
            return Ok(build_scope(&key));
        }
    }

    // 5. Fallback 到 bcrypt (legacy keys)
    if bcrypt::verify(secret_key, &key.hashed_secret_key)
        .map_err(|e| AppError::Internal(format!("bcrypt verify failed: {}", e)))? {
        // 升级：存储 fast hash 供下次使用
        sqlx::query("UPDATE api_keys SET fast_hashed_secret_key = $1 WHERE id = $2")
            .bind(&fast_hash)
            .bind(&key.id)
            .execute(pool)
            .await?;

        update_last_used(pool, &key.id).await?;
        return Ok(build_scope(&key));
    }

    Err(AppError::Unauthorized("Invalid API key".to_string()))
}

fn build_scope(key: &ApiKeyRow) -> ApiKeyScope {
    ApiKeyScope {
        project_id: key.project_id.clone(),
        org_id: key.org_id.clone(),
        scope: key.scope.clone(),
        api_key_id: key.id.clone(),
    }
}

async fn update_last_used(pool: &PgPool, id: &str) -> Result<()> {
    sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn base64_decode(input: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input)
        .ok()?;
    String::from_utf8(bytes).ok()
}

// Database row type (internal). Only the columns verification reads, so a
// column added to `api_keys` does not silently become part of this contract.
#[derive(Debug, sqlx::FromRow)]
struct ApiKeyRow {
    id: String,
    hashed_secret_key: String,
    fast_hashed_secret_key: Option<String>,
    scope: String,
    project_id: Option<String>,
    org_id: Option<String>,
    expires_at: Option<chrono::NaiveDateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_auth_valid() {
        // base64("test-pub-key:test-secret-key") = "dGVzdC1wdWIta2V5OnRlc3Qtc2VjcmV0LWtleQ=="
        let header = "Basic dGVzdC1wdWIta2V5OnRlc3Qtc2VjcmV0LWtleQ==";
        let (pk, sk) = parse_basic_auth(header).unwrap();
        assert_eq!(pk, "test-pub-key");
        assert_eq!(sk, "test-secret-key");
    }

    #[test]
    fn test_parse_basic_auth_invalid() {
        assert!(parse_basic_auth("Invalid header").is_none());
        assert!(parse_basic_auth("Basic invalid-base64!!").is_none());
    }

    #[test]
    fn test_parse_bearer_auth() {
        let pk = parse_bearer_auth("Bearer my-token").unwrap();
        assert_eq!(pk, "my-token");
        assert!(parse_bearer_auth("Invalid").is_none());
    }
}
