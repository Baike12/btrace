//! Public v2 endpoints that this fork does not implement.
//!
//! Both are registered so a caller receives an explicit `501 Not Implemented`
//! naming the reason, rather than a `404` that is indistinguishable from a
//! typo'd path.
//!
//! * `GET /api/public/v2/metrics` — a general OLAP query engine (three views,
//!   ~20 dimensions, ~15 measures, 11 aggregation functions including
//!   percentiles and histograms, a filter operator matrix, time granularity,
//!   and ordering). Its contract is deliberately open-ended
//!   (`data: list<map<string, unknown>>`), so a partial implementation would
//!   return plausible-but-wrong numbers. `GET /api/public/metrics/daily` covers
//!   the per-day, per-model usage and cost question.
//!
//! * `GET /api/public/v2/observations` — cursor pagination with ten field groups
//!   (`core`, `basic`, `time`, `io`, `metadata`, `model`, `usage`, `prompt`,
//!   `metrics`, `trace_context`), metadata truncation, and a structured filter
//!   parameter. Observations are reachable today through
//!   `GET /api/public/traces/{traceId}`, which embeds them.

use axum::http::StatusCode;
use crate::response::public_error;

pub async fn metrics_not_implemented() -> axum::response::Response {
    public_error(
        StatusCode::NOT_IMPLEMENTED,
        "GET /api/public/v2/metrics is not implemented in this build. \
         Use GET /api/public/metrics/daily for per-day, per-model usage and cost.",
        "not_implemented",
    )
}

pub async fn observations_not_implemented() -> axum::response::Response {
    public_error(
        StatusCode::NOT_IMPLEMENTED,
        "GET /api/public/v2/observations is not implemented in this build. \
         Observations are available via GET /api/public/traces/{traceId}.",
        "not_implemented",
    )
}
