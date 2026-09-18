//! 密码哈希与验证工具
//!
//! 使用 bcrypt (12 rounds) 进行密码哈希。

use langfuse_core::{AppError, Result};

/// 最小密码长度
const MIN_PASSWORD_LENGTH: usize = 8;

/// 使用 bcrypt 哈希密码 (cost = 12)
pub fn hash_password(password: &str) -> Result<String> {
    bcrypt::hash(password, 12).map_err(|e| AppError::Internal(format!("Password hash failed: {}", e)))
}

/// 验证密码是否匹配哈希
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    bcrypt::verify(password, hash).map_err(|e| AppError::Internal(format!("Password verify failed: {}", e)))
}

/// bcrypt hash of a value nobody knows, at the same cost as real hashes.
///
/// Used when a login names an email that has no account: the handler still
/// runs a bcrypt comparison so that "no such user" and "wrong password" take
/// the same time to answer. Without it, response latency tells an
/// unauthenticated caller which addresses have accounts — a user-enumeration
/// oracle, and the first step of credential stuffing.
///
/// The plaintext is irrelevant: nothing verifies against it successfully.
pub const TIMING_EQUALIZER_HASH: &str =
    "$2b$12$NMZD3f7M37pxG0CLMiTZH.E3IVJJXJVVfu089PyT.s0pKCO4FfJxS";

/// Spend roughly the same time a real password check would.
pub fn equalize_timing() {
    // The result is discarded on purpose — only the elapsed time matters.
    let _ = bcrypt::verify("no-such-account", TIMING_EQUALIZER_HASH);
}

/// 检查密码是否符合基本要求（最小长度 8）
pub fn is_valid_password(password: &str) -> bool {
    password.len() >= MIN_PASSWORD_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let hash = hash_password("test1234").unwrap();
        assert!(verify_password("test1234", &hash).unwrap());
        assert!(!verify_password("wrongpass", &hash).unwrap());
    }

    #[test]
    fn test_valid_password() {
        assert!(is_valid_password("12345678"));
        assert!(!is_valid_password("short"));
        assert!(is_valid_password("a_very_long_password_here"));
    }

    #[test]
    fn the_timing_equalizer_is_a_real_bcrypt_hash_at_the_same_cost() {
        // A truncated or mistyped constant would make `equalize_timing` return
        // instantly, which is exactly the oracle it exists to close.
        let parts: Vec<&str> = TIMING_EQUALIZER_HASH.split('$').collect();
        assert_eq!(parts[1], "2b", "unexpected bcrypt version");
        assert_eq!(parts[2], "12", "cost must match `hash_password`'s 12");

        assert!(equalize_timing_actually_hashes());
    }

    /// Confirms the constant parses: an unparseable hash makes `bcrypt::verify`
    /// return `Err` immediately, which is as fast as not checking at all.
    fn equalize_timing_actually_hashes() -> bool {
        bcrypt::verify("anything", TIMING_EQUALIZER_HASH).is_ok()
    }

    #[test]
    fn an_unknown_account_never_verifies_against_the_equalizer() {
        assert!(!verify_password("no-such-account", TIMING_EQUALIZER_HASH).unwrap());
        assert!(!verify_password("", TIMING_EQUALIZER_HASH).unwrap());
    }
}
