//! The single event → PostgreSQL sink for traces, observations and scores.
//!
//! Every write path funnels through here, so the mapping and the SQL exist
//! exactly once:
//!
//! * `POST /api/public/ingestion` — Langfuse SDK batch ingestion
//! * `POST /api/public/otel/v1/traces` — OTLP/HTTP direct write (LexQA)
//!
//! Writes are synchronous and per-record. The buffered `BatchWriter` is not
//! used on these paths: it flushes on a 500 ms timer and only early when a
//! batch reaches 1000 records, so whether a trace became visible "within
//! seconds" depended on unrelated traffic — while the documented contract for
//! both Langfuse SDKs and LexQA is that the trace is queryable right after the
//! ingest call returns.

use langfuse_core::{ObservationRecord, Result, ScoreRecord, TraceRecord};
use langfuse_db::repos::{
    observations::upsert_observation,
    scores::upsert_score,
    sessions::upsert_session,
    traces::{ensure_trace, upsert_trace},
};
use sqlx::PgPool;

use crate::pricing;

/// Persist a trace and, when it carries a session, keep `trace_sessions` in
/// sync so the Sessions view can aggregate it.
pub async fn write_trace(pool: &PgPool, record: &TraceRecord) -> Result<()> {
    upsert_trace(pool, record).await?;

    if let Some(session_id) = record.session_id.as_deref() {
        if !session_id.is_empty() {
            // `traces` has no environment column in this fork, so sessions are
            // never environment-scoped.
            upsert_session(pool, session_id, &record.project_id, None).await?;
        }
    }

    Ok(())
}

/// Persist an observation, creating a placeholder trace row first if the root
/// span has not arrived yet, and computing its cost from the project's model
/// prices.
///
/// OTLP batches are per-process and unordered: LexQA's HTTP root span and the
/// asynq worker spans it parents travel in separate exports, in either order.
/// The trace list reads `traces`, so without the placeholder a tree whose root
/// is still in flight would be invisible. `ensure_trace` never overwrites an
/// existing row, so the real root span still wins whenever it lands.
pub async fn write_observation(pool: &PgPool, record: &ObservationRecord) -> Result<()> {
    let mut record = record.clone();

    // Pricing needs the project's model rows, so it happens here rather than in
    // the mappers — both ingest paths get it without duplicating the lookup.
    let project_id = record.project_id.clone();
    pricing::apply_pricing(pool, &project_id, &mut record).await;

    if let Some(trace_id) = record.trace_id.as_deref() {
        if !trace_id.is_empty() {
            ensure_trace(pool, &placeholder_trace(trace_id, &record)).await?;
        }
    }

    upsert_observation(pool, &record).await
}

/// Persist a score.
pub async fn write_score(pool: &PgPool, record: &ScoreRecord) -> Result<()> {
    upsert_score(pool, record).await
}

/// Persist a dataset run item.
///
/// There is no typed record for this table, so the insert lives here — it is
/// NOT NULL on `dataset_run_id`, `dataset_item_id` and `trace_id`, which is why
/// they are separate parameters rather than a free-form JSON map.
pub async fn write_dataset_run_item(
    pool: &PgPool,
    id: &str,
    project_id: &str,
    dataset_run_id: &str,
    dataset_item_id: &str,
    trace_id: Option<&str>,
    observation_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO dataset_run_items (
               id, project_id, dataset_run_id, dataset_item_id, trace_id, observation_id,
               created_at, updated_at
           ) VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
           ON CONFLICT (id, project_id) DO UPDATE SET
               dataset_run_id = EXCLUDED.dataset_run_id,
               dataset_item_id = EXCLUDED.dataset_item_id,
               trace_id = COALESCE(EXCLUDED.trace_id, dataset_run_items.trace_id),
               observation_id = COALESCE(EXCLUDED.observation_id, dataset_run_items.observation_id),
               updated_at = NOW()"#,
    )
    .bind(id)
    .bind(project_id)
    .bind(dataset_run_id)
    .bind(dataset_item_id)
    .bind(trace_id)
    .bind(observation_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Minimal trace row for an observation whose root span has not been seen.
fn placeholder_trace(trace_id: &str, observation: &ObservationRecord) -> TraceRecord {
    TraceRecord {
        id: trace_id.to_string(),
        project_id: observation.project_id.clone(),
        external_id: None,
        timestamp: observation.start_time,
        name: None,
        user_id: None,
        metadata: None,
        release: None,
        version: None,
        public: Some(false),
        bookmarked: Some(false),
        tags: None,
        input: None,
        output: None,
        session_id: None,
    }
}
