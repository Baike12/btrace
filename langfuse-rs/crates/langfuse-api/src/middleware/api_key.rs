use axum::{
    extract::State,
    http::{header::AUTHORIZATION, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use langfuse_auth::api_key::{parse_basic_auth, verify_api_key};
use langfuse_core::AppError;

use crate::app::AppState;
use crate::response::public_error;

/// Authenticate `/api/public/*` requests against the `api_keys` table.
///
/// Credentials are `Authorization: Basic base64(public_key:secret_key)` — the
/// shape every Langfuse SDK and every OTLP exporter emits (the OTLP direct-write
/// path documented for Langfuse v3+ / v4 uses exactly this header).
///
/// The verified scope is injected as a request extension, so handlers can scope
/// every read and write to `scope.project_id` instead of trusting the payload.
///
/// Failures return the official Langfuse public-API error body
/// `{ "message": ..., "error": ... }` rather than a bare status code: clients
/// like the Go OTLP exporter and LexQA only see the status otherwise, which
/// makes "bad secret" indistinguishable from "key expired" or "upstream down".
pub async fn verify_api_key_middleware(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let Some(header) = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return Err(public_error(
            StatusCode::UNAUTHORIZED,
            "Missing Authorization header. Expected: Basic base64(public_key:secret_key)",
            "unauthorized",
        ));
    };

    // Only Basic carries both halves of the credential pair. A Bearer token
    // holds a single opaque value with no secret to verify against, so it
    // cannot be checked here and is rejected explicitly rather than silently
    // trusted.
    let Some((public_key, secret_key)) = parse_basic_auth(header) else {
        return Err(public_error(
            StatusCode::UNAUTHORIZED,
            "Unsupported Authorization scheme. Expected: Basic base64(public_key:secret_key)",
            "unauthorized",
        ));
    };

    let scope = verify_api_key(&state.pool, &public_key, &secret_key)
        .await
        .map_err(|e| match e {
            // Verify failures are the client's fault: surface the reason.
            AppError::Unauthorized(msg) => {
                public_error(StatusCode::UNAUTHORIZED, msg, "unauthorized")
            }
            other => {
                tracing::error!(public_key = %public_key, error = %other, "api key verification failed");
                public_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to verify API key",
                    "internal_error",
                )
            }
        })?;

    req.extensions_mut().insert(scope);
    Ok(next.run(req).await)
}
