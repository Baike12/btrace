//! Per-account throttling for the sign-in endpoint.
//!
//! `POST /api/auth/login` is the only place in the console where a password is
//! checked, which makes it the only place worth brute-forcing. This caps how
//! many times a single account may fail before further attempts are refused
//! for a cooling-off window.
//!
//! Keyed by **email, not client IP** on purpose. In the shipped topology the
//! browser reaches the API through the Next.js rewrite, so every request
//! arrives from the same address: an IP-keyed limiter would either never
//! trigger or lock out every user at once the moment one attacker tripped it.
//! Keying on the account under attack throttles the attack without touching
//! anybody else.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

/// Failed attempts allowed inside [`WINDOW`] before the account is locked out.
pub const MAX_FAILURES: u32 = 10;

/// How long failures are remembered, and how long a lockout lasts.
pub const WINDOW: Duration = Duration::minutes(15);

/// Hard cap on tracked keys.
///
/// Keys are attacker-chosen email addresses, so an unbounded map is a memory
/// exhaustion vector. On overflow the oldest windows are dropped — they are
/// the closest to expiring anyway.
const MAX_TRACKED_KEYS: usize = 10_000;

#[derive(Debug, Clone, Copy)]
struct Failures {
    count: u32,
    first: DateTime<Utc>,
}

/// In-process failed-login tracker. Cloneable handle over shared state.
#[derive(Debug, Default)]
pub struct LoginThrottle {
    failures: Mutex<HashMap<String, Failures>>,
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `key` may attempt a sign-in right now.
    ///
    /// `Err` carries the whole seconds until the lockout lifts.
    pub fn check(&self, key: &str) -> Result<(), i64> {
        let mut map = self.lock();
        let Some(entry) = map.get(key) else {
            return Ok(());
        };

        let elapsed = Utc::now() - entry.first;
        if elapsed >= WINDOW {
            // The window has rolled over; forget the record rather than hold a
            // stale counter that would lock the account out on its next typo.
            map.remove(key);
            return Ok(());
        }

        if entry.count >= MAX_FAILURES {
            return Err((WINDOW - elapsed).num_seconds().max(1));
        }
        Ok(())
    }

    /// Record a failed attempt, starting a new window when the old one expired.
    pub fn record_failure(&self, key: &str) {
        let now = Utc::now();
        let mut map = self.lock();

        if map.len() >= MAX_TRACKED_KEYS && !map.contains_key(key) {
            map.retain(|_, f| now - f.first < WINDOW);
        }

        map.entry(key.to_string())
            .and_modify(|f| {
                if now - f.first >= WINDOW {
                    *f = Failures { count: 1, first: now };
                } else {
                    f.count = f.count.saturating_add(1);
                }
            })
            .or_insert(Failures { count: 1, first: now });
    }

    /// Clear the record after a successful sign-in.
    pub fn record_success(&self, key: &str) {
        self.lock().remove(key);
    }

    /// A poisoned mutex means another thread panicked while holding it. The
    /// tracker is a best-effort defence, so recovering is preferable to
    /// panicking the request thread — losing the counter only costs rate
    /// limiting, not correctness of the credential check.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Failures>> {
        self.failures.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_account_is_allowed() {
        let t = LoginThrottle::new();
        assert!(t.check("a@b.com").is_ok());
    }

    #[test]
    fn locks_out_after_the_configured_number_of_failures() {
        let t = LoginThrottle::new();
        for _ in 0..MAX_FAILURES - 1 {
            t.record_failure("a@b.com");
        }
        assert!(t.check("a@b.com").is_ok(), "still under the limit");

        t.record_failure("a@b.com");
        let retry_after = t.check("a@b.com").unwrap_err();
        assert!(retry_after > 0 && retry_after <= WINDOW.num_seconds());
    }

    #[test]
    fn a_successful_sign_in_clears_the_counter() {
        let t = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            t.record_failure("a@b.com");
        }
        assert!(t.check("a@b.com").is_err());

        t.record_success("a@b.com");
        assert!(t.check("a@b.com").is_ok());
    }

    #[test]
    fn one_account_being_attacked_does_not_lock_out_another() {
        let t = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            t.record_failure("victim@b.com");
        }
        assert!(t.check("victim@b.com").is_err());
        assert!(t.check("someone-else@b.com").is_ok());
    }

    #[test]
    fn an_expired_window_stops_counting() {
        let t = LoginThrottle::new();
        {
            let mut map = t.lock();
            map.insert(
                "a@b.com".into(),
                Failures {
                    count: MAX_FAILURES,
                    first: Utc::now() - WINDOW - Duration::seconds(1),
                },
            );
        }
        assert!(t.check("a@b.com").is_ok());
        // The stale record is dropped rather than left to re-trip later.
        assert!(t.lock().get("a@b.com").is_none());
    }
}
