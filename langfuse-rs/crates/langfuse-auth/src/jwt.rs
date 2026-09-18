use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use langfuse_core::{AppError, Result};

/// JWT Claims — 简化为个人使用，仅包含用户基本信息
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// subject = user_id
    pub sub: String,
    /// 过期时间 (unix timestamp)
    pub exp: usize,
    /// 签发时间
    pub iat: usize,
    /// 用户邮箱
    pub email: String,
    /// 用户名
    pub name: String,
}

/// Session — 从 JWT Claims 提取，注入到 request extensions
#[derive(Debug, Clone)]
pub struct Session {
    pub user_id: String,
    pub email: String,
    pub name: String,
}

impl From<Claims> for Session {
    fn from(claims: Claims) -> Self {
        Session {
            user_id: claims.sub,
            email: claims.email,
            name: claims.name,
        }
    }
}

/// 签发 JWT token
pub fn create_token(
    user_id: &str,
    email: &str,
    name: &str,
    secret: &[u8],
    max_age_minutes: i64,
) -> Result<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        exp: (now + chrono::Duration::minutes(max_age_minutes)).timestamp() as usize,
        iat: now.timestamp() as usize,
        email: email.to_string(),
        name: name.to_string(),
    };

    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret))
        .map_err(|e| AppError::Internal(format!("JWT encode failed: {}", e)))
}

/// Clock-skew tolerance when checking `exp`.
///
/// The issuer and the verifier are the same binary in the shipped topology, so
/// this is slack for a deployment that runs several replicas whose clocks drift
/// slightly — not a grace period for the user. Stated explicitly because
/// `jsonwebtoken`'s default is also 60s: leaving it implicit would make the
/// boundary between "expired" and "still valid" invisible to anyone reading
/// this, and to any test that tries to pin it.
pub const EXPIRY_LEEWAY_SECONDS: u64 = 60;

/// 验证 JWT token
pub fn verify_token(token: &str, secret: &[u8]) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.leeway = EXPIRY_LEEWAY_SECONDS;

    let data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map_err(|e| AppError::Unauthorized(format!("Invalid token: {}", e)))?;

    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        let secret = b"test-secret-key-for-jwt";
        let token = create_token("user1", "a@b.com", "Test User", secret, 60).unwrap();
        let claims = verify_token(&token, secret).unwrap();

        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.email, "a@b.com");
        assert_eq!(claims.name, "Test User");

        let session = Session::from(claims);
        assert_eq!(session.user_id, "user1");
        assert_eq!(session.email, "a@b.com");
    }

    #[test]
    fn test_invalid_token() {
        let result = verify_token("invalid-token", b"wrong-secret");
        assert!(result.is_err());
    }
}
