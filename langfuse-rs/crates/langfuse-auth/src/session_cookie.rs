//! The UI session cookie.
//!
//! The Next.js layer authenticates a browser once and the Rust API trusts the
//! resulting JWT on every subsequent `/api/*` call. That JWT travels in an
//! `HttpOnly` cookie: `HttpOnly` keeps it out of reach of page scripts, so an
//! XSS bug cannot exfiltrate a session the way a `localStorage` token would.
//!
//! Both halves live here — reading on the way in and writing on the way out —
//! so the cookie's name and attributes cannot drift between the middleware that
//! verifies it and the handler that issues it.

use cookie::{Cookie, SameSite};

/// Cookie carrying the session JWT. Every read and write goes through this
/// constant; a literal string in either place would be a silent auth bypass.
pub const SESSION_COOKIE_NAME: &str = "langfuse_session";

/// Session lifetime. 30 days matches what the UI's session payload advertises.
pub const SESSION_TTL_MINUTES: i64 = 30 * 24 * 60;

/// `Secure` is applied when the deployment terminates TLS.
///
/// It is opt-in rather than automatic because most self-hosted installs are
/// reached over plain HTTP on a private network: a `Secure` cookie is dropped
/// by the browser over HTTP, which would make sign-in appear to succeed and
/// then silently fail on the next request.
fn secure_cookies_from_env() -> bool {
    matches!(
        std::env::var("LANGFUSE_SECURE_COOKIES").as_deref(),
        Ok("true") | Ok("1")
    )
}

/// Build the `Set-Cookie` value that signs a browser in.
pub fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, token))
        .http_only(true)
        // Lax, not Strict: the UI navigates in from other origins (an SSO
        // redirect, a link in an email), and Strict would drop the cookie on
        // that first navigation and bounce the user back to sign-in.
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(cookie::time::Duration::minutes(SESSION_TTL_MINUTES))
        .secure(secure_cookies_from_env())
        .build()
}

/// Build the `Set-Cookie` value that signs a browser out.
///
/// The attributes must match [`session_cookie`] or the browser treats it as a
/// *different* cookie and the real one stays behind.
pub fn cleared_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(cookie::time::Duration::ZERO)
        .secure(secure_cookies_from_env())
        .build()
}

/// Pull the session JWT out of a `Cookie:` request header.
///
/// Returns `None` when the header is absent or carries no session cookie. The
/// caller turns that into a 401 — this function never falls back to a default
/// identity.
pub fn read_session_cookie(cookie_header: &str) -> Option<String> {
    Cookie::split_parse(cookie_header)
        .filter_map(std::result::Result::ok)
        .find(|c| c.name() == SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_session_cookie_out_of_a_shared_header() {
        let header = "other=1; langfuse_session=abc.def.ghi; another=2";
        assert_eq!(
            read_session_cookie(header).as_deref(),
            Some("abc.def.ghi")
        );
    }

    #[test]
    fn a_missing_or_empty_cookie_is_none_not_a_default_identity() {
        assert!(read_session_cookie("other=1").is_none());
        assert!(read_session_cookie("").is_none());
        assert!(read_session_cookie("langfuse_session=").is_none());
    }

    #[test]
    fn the_issued_cookie_is_httponly_and_scoped_to_the_whole_site() {
        let c = session_cookie("tok".into());
        assert_eq!(c.name(), SESSION_COOKIE_NAME);
        assert_eq!(c.http_only(), Some(true));
        assert_eq!(c.path(), Some("/"));
        assert_eq!(c.same_site(), Some(SameSite::Lax));
    }

    #[test]
    fn clearing_matches_the_attributes_of_the_cookie_being_cleared() {
        // Same name/path/domain/SameSite, otherwise the browser keeps the
        // original cookie and "sign out" silently does nothing.
        let issued = session_cookie("tok".into());
        let cleared = cleared_session_cookie();
        assert_eq!(cleared.name(), issued.name());
        assert_eq!(cleared.path(), issued.path());
        assert_eq!(cleared.same_site(), issued.same_site());
        assert_eq!(cleared.max_age(), Some(cookie::time::Duration::ZERO));
    }

    #[test]
    fn a_round_trip_through_set_cookie_and_cookie_header_works() {
        // `Set-Cookie` → browser → `Cookie` is what the two processes actually
        // exchange; parsing our own output catches attribute-mangling bugs that
        // asserting on the built struct would miss.
        let set_cookie = session_cookie("header.payload.signature".into()).to_string();
        let value = set_cookie
            .strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
            .and_then(|rest| rest.split(';').next())
            .expect("cookie is named as expected");
        let request_header = format!("langfuse_session={value}");
        assert_eq!(
            read_session_cookie(&request_header).as_deref(),
            Some("header.payload.signature")
        );
    }
}
