//! API key secret hashing — the one place the scheme is defined.
//!
//! Both sides of a key's life depend on this agreeing with itself:
//!
//! * **Creation** — `langfuse-db`'s `LANGFUSE_INIT_*` bootstrap, and the UI /
//!   public API when they mint a key.
//! * **Verification** — `langfuse-auth::api_key::verify_api_key`, which runs on
//!   every `/api/public/*` request.
//!
//! They previously disagreed (bootstrap wrote a bare `SHA256(sk)` into a second
//! row while verification expected `SHA256(SHA256(sk) + SALT)` on the row it
//! found by public key), so every bootstrapped key authenticated as 401. Keeping
//! the functions here — in the layer both crates already depend on — is what
//! stops that from drifting apart again.

use crate::{AppError, Result};
use sha2::{Digest, Sha256};

/// Fast hash: `SHA256(SHA256(secretKey) + SALT)`.
///
/// Stored in `api_keys.fast_hashed_secret_key` and compared on every request.
pub fn fast_hash_secret_key(secret_key: &str, salt: &str) -> String {
    let inner_hash = hex::encode(Sha256::digest(secret_key.as_bytes()));
    hex::encode(Sha256::digest(format!("{}{}", inner_hash, salt).as_bytes()))
}

/// bcrypt hash of the secret — stored in `api_keys.hashed_secret_key` and used
/// as the verification fallback when no fast hash is present.
pub fn hash_secret_key(secret_key: &str) -> Result<String> {
    bcrypt::hash(secret_key, 12)
        .map_err(|e| AppError::Internal(format!("api key hash failed: {}", e)))
}

/// Display form of a secret key: prefix plus the last four characters, matching
/// what the Langfuse UI shows for a created key. Safe to log and to store in
/// `api_keys.display_secret_key` (a NOT NULL column).
pub fn display_secret_key(secret_key: &str) -> String {
    let chars: Vec<char> = secret_key.chars().collect();
    if chars.len() <= 8 {
        return "sk-…".to_string();
    }
    let prefix: String = chars.iter().take(7).collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{}…{}", prefix, suffix)
}

/// The SALT used for [`fast_hash_secret_key`], read from the environment.
///
/// A missing SALT falls back to the same development default the Rust backend
/// and the Next.js layer use, so a local stack does not silently produce keys
/// that cannot be verified.
pub fn salt_from_env() -> String {
    std::env::var("SALT").unwrap_or_else(|_| "dev-salt".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_hash_is_salted_and_double_hashed() {
        let salt = "test-salt";
        let a = fast_hash_secret_key("sk-lf-abc", salt);
        let b = fast_hash_secret_key("sk-lf-abc", "other-salt");

        assert_ne!(a, b, "SALT must change the hash");
        // 64 hex chars = SHA-256 output.
        assert_eq!(a.len(), 64);
        // Not the bare SHA-256 of the secret (the old, incompatible scheme).
        let bare = hex::encode(Sha256::digest(b"sk-lf-abc"));
        assert_ne!(a, bare);
    }

    #[test]
    fn fast_hash_is_stable() {
        assert_eq!(
            fast_hash_secret_key("sk-lf-abc", "s"),
            fast_hash_secret_key("sk-lf-abc", "s")
        );
    }

    #[test]
    fn display_secret_key_masks_the_middle() {
        let display = display_secret_key("sk-lf-1234567890abcdef");
        assert!(display.starts_with("sk-lf-1"));
        assert!(display.ends_with("cdef"));
        assert!(!display.contains("67890"));
    }

    #[test]
    fn display_secret_key_handles_short_input() {
        assert_eq!(display_secret_key("short"), "sk-…");
    }
}
