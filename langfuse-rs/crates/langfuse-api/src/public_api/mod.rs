use axum::Router;
use crate::app::AppState;

pub mod ingestion;
pub mod metrics;
pub mod models;
pub mod otel;
pub mod sessions;
pub mod traces;
pub mod v2;

/// Public health endpoint. Mounted by `app::create_app` **outside** the API-key
/// layer so container healthchecks (`docker HEALTHCHECK`, k8s probes) can reach
/// it without credentials.
///
/// It is deliberately not registered here as well: this router is nested under
/// `/api/public` *behind* the API-key layer, and axum rejects the duplicate
/// `GET /api/public/health` that would result.
pub use ingestion::health;

/// Routes that require a valid API key (the scope is injected by
/// `middleware::api_key::verify_api_key_middleware`).
pub fn router() -> Router<AppState> {
    Router::new()
        // --- ingestion ---------------------------------------------------
        .route("/ingestion", axum::routing::post(ingestion::handler))
        // OTLP/HTTP direct-write path used by Langfuse v3+/v4 SDKs and the Go
        // OTLP exporter (LexQA). See `otel` for the wire contract.
        .route("/otel/v1/traces", axum::routing::post(otel::handler))
        // --- traces ------------------------------------------------------
        .route("/traces", axum::routing::get(traces::list))
        .route("/traces/{traceId}", axum::routing::get(traces::get))
        // --- sessions ----------------------------------------------------
        .route("/sessions", axum::routing::get(sessions::list))
        .route("/sessions/{sessionId}", axum::routing::get(sessions::get))
        // --- metrics -----------------------------------------------------
        .route("/metrics/daily", axum::routing::get(metrics::daily))
        // --- models ------------------------------------------------------
        .route(
            "/models",
            axum::routing::get(models::list).post(models::create),
        )
        .route(
            "/models/{id}",
            axum::routing::get(models::get).delete(models::delete),
        )
        // --- not implemented ---------------------------------------------
        // Registered so callers get an explicit 501 instead of a 404 that looks
        // like a routing mistake. See `v2` for the reason.
        .route("/v2/metrics", axum::routing::get(v2::metrics_not_implemented))
        .route(
            "/v2/observations",
            axum::routing::get(v2::observations_not_implemented),
        )
}
