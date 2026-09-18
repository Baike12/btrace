//! Event-batch processing for Langfuse SDK ingestion.
//!
//! [`process_batch`] is the single implementation behind both callers:
//!
//! * `POST /api/public/ingestion` — synchronous, in-request
//! * the `ingestion-queue` job processor, for payloads that arrive via `pg_jobs`
//!
//! Both merge the per-entity event list and hand each entity to [`crate::sink`],
//! the same write path the OTLP endpoint uses. Events are written synchronously
//! and individually.
//!
//! The previous implementation buffered into `BatchWriter` (flushing on a 500 ms
//! timer or at 1000 records) and mapped events by hand into untyped JSON. That
//! made visibility depend on unrelated traffic, and left the event → column
//! mapping duplicated between here and the OTLP path, where the two copies had
//! already drifted apart.

use async_trait::async_trait;
use chrono::Utc;
use langfuse_core::{ObservationRecord, PgJob, ScoreRecord, TraceRecord};
use langfuse_queue::dedup::check_and_mark_seen;
use langfuse_queue::JobProcessor;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{merger, sink};

/// Merge one entity's events and write the result to PostgreSQL.
///
/// Deduplicates on `(project, entity type, entity id, file key)` first, so a
/// retried job or a re-sent batch is a no-op rather than a duplicate write.
pub async fn process_batch(
    pool: &PgPool,
    project_id: &str,
    entity_type: &str,
    event_body_id: &str,
    file_key: &str,
    events: &[Value],
) -> Result<(), String> {
    let is_duplicate = check_and_mark_seen(pool, project_id, entity_type, event_body_id, file_key)
        .await
        .map_err(|e| format!("Dedup check failed: {}", e))?;

    if is_duplicate {
        return Ok(());
    }

    match entity_type {
        "trace" => process_trace_events(pool, project_id, events).await,
        "observation" => process_observation_events(pool, project_id, events).await,
        "score" => process_score_events(pool, project_id, events).await,
        "dataset_run_item" => process_dataset_events(pool, project_id, events).await,
        other => Err(format!("Unknown entity type: {}", other)),
    }
}

/// IngestionProcessor — 实现队列 JobProcessor trait
///
/// 对应原 `worker/src/queues/ingestionQueue.ts` 中的 ingestionQueueBuilder
pub struct IngestionProcessor {
    pool: PgPool,
}

impl IngestionProcessor {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl JobProcessor for IngestionProcessor {
    async fn process(&self, job: &PgJob) -> Result<(), String> {
        let payload: Value = job.payload.clone();
        let data = payload.get("data").ok_or("Missing 'data' in payload")?;
        let entity_type = data
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or("Missing event type")?;
        let event_body_id = data
            .get("eventBodyId")
            .and_then(|id| id.as_str())
            .ok_or("Missing eventBodyId")?;
        let events = data
            .get("events")
            .and_then(|e| e.as_array())
            .ok_or("Missing 'events' array")?;

        let auth_check = payload.get("authCheck").ok_or("Missing authCheck")?;
        let project_id = auth_check
            .get("project_id")
            .and_then(|p| p.as_str())
            .ok_or("Missing project_id in authCheck")?;

        let file_key = data.get("fileKey").and_then(|k| k.as_str()).unwrap_or("");

        process_batch(
            &self.pool,
            project_id,
            entity_type,
            event_body_id,
            file_key,
            events,
        )
        .await
    }
}

async fn process_trace_events(
    pool: &PgPool,
    project_id: &str,
    events: &[Value],
) -> Result<(), String> {
    let merged = merger::merge_trace_records(events, None);
    let record: TraceRecord = from_merged(merged, project_id, "trace", |obj| {
        // `public` / `bookmarked` are NOT NULL on `traces`.
        obj.entry("timestamp")
            .or_insert_with(|| json!(Utc::now().to_rfc3339()));
        obj.entry("public").or_insert_with(|| json!(false));
        obj.entry("bookmarked").or_insert_with(|| json!(false));
    })?;

    sink::write_trace(pool, &record)
        .await
        .map_err(|e| format!("Trace write failed: {}", e))
}

async fn process_observation_events(
    pool: &PgPool,
    project_id: &str,
    events: &[Value],
) -> Result<(), String> {
    let merged = merger::merge_observation_records(events, None);
    let record: ObservationRecord = from_merged(merged, project_id, "observation", |obj| {
        // SDKs send usage as a nested `usage` object, while the columns are
        // flat. Normalise before the NOT NULL defaults below, so a real count is
        // never overwritten by the zero fallback.
        apply_usage(obj);

        // All of these are NOT NULL on `observations`.
        obj.entry("type").or_insert_with(|| json!("GENERATION"));
        obj.entry("level").or_insert_with(|| json!("DEFAULT"));
        obj.entry("start_time")
            .or_insert_with(|| json!(Utc::now().to_rfc3339()));
        obj.entry("prompt_tokens").or_insert_with(|| json!(0));
        obj.entry("completion_tokens").or_insert_with(|| json!(0));
        obj.entry("total_tokens").or_insert_with(|| json!(0));
    })?;

    sink::write_observation(pool, &record)
        .await
        .map_err(|e| format!("Observation write failed: {}", e))
}

async fn process_score_events(
    pool: &PgPool,
    project_id: &str,
    events: &[Value],
) -> Result<(), String> {
    let merged = merger::merge_score_records(events, None);
    let record: ScoreRecord = from_merged(merged, project_id, "score", |obj| {
        obj.entry("source").or_insert_with(|| json!("API"));
        obj.entry("timestamp")
            .or_insert_with(|| json!(Utc::now().to_rfc3339()));
    })?;

    sink::write_score(pool, &record)
        .await
        .map_err(|e| format!("Score write failed: {}", e))
}

async fn process_dataset_events(
    pool: &PgPool,
    project_id: &str,
    events: &[Value],
) -> Result<(), String> {
    for event in events {
        let body = event.get("body").unwrap_or(event);
        let id = body
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("Missing dataset run item id")?;
        let dataset_run_id = body
            .get("datasetRunId")
            .and_then(|r| r.as_str())
            .ok_or("Missing datasetRunId")?;
        let dataset_item_id = body
            .get("datasetItemId")
            .and_then(|i| i.as_str())
            .ok_or("Missing datasetItemId")?;
        let trace_id = body.get("traceId").and_then(|t| t.as_str());
        let observation_id = body.get("observationId").and_then(|o| o.as_str());

        sink::write_dataset_run_item(
            pool,
            id,
            project_id,
            dataset_run_id,
            dataset_item_id,
            trace_id,
            observation_id,
        )
        .await
        .map_err(|e| format!("Dataset run item write failed: {}", e))?;
    }

    Ok(())
}

/// Normalise a nested SDK `usage` object into the flat token columns plus
/// `usage_details`, so both ingestion paths record the same fields.
///
/// Accepts the modern keys (`input` / `output` / `total`) and the legacy
/// `*Tokens` / `*_tokens` spellings. `total` is derived when absent, because
/// several providers omit it and a zero there would understate cost.
///
/// Existing values win: an SDK that already sent flat `prompt_tokens` alongside
/// `usage` is not second-guessed.
fn apply_usage(obj: &mut serde_json::Map<String, Value>) {
    let Some(usage) = obj.get("usage").and_then(|u| u.as_object()).cloned() else {
        return;
    };

    let pick = |keys: &[&str]| -> Option<i64> {
        keys.iter().find_map(|k| {
            usage.get(*k).and_then(|v| match v {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            })
        })
    };

    let input = pick(&["input", "promptTokens", "prompt_tokens"]);
    let output = pick(&["output", "completionTokens", "completion_tokens"]);
    let total = pick(&["total", "totalTokens", "total_tokens"]).or(match (input, output) {
        (Some(i), Some(o)) => Some(i + o),
        _ => None,
    });

    if let Some(v) = input {
        obj.entry("prompt_tokens").or_insert_with(|| json!(v));
    }
    if let Some(v) = output {
        obj.entry("completion_tokens").or_insert_with(|| json!(v));
    }
    if let Some(v) = total {
        obj.entry("total_tokens").or_insert_with(|| json!(v));
    }
    if let Some(unit) = usage.get("unit").and_then(|u| u.as_str()) {
        obj.entry("unit").or_insert_with(|| json!(unit));
    }
    obj.entry("usage_details")
        .or_insert_with(|| Value::Object(usage));
}

/// Normalise a merged event object and deserialise it into a typed record.
///
/// `project_id` and the NOT NULL columns are filled in by `fill_defaults`
/// before deserialising: the database defaults only apply to columns that are
/// *omitted*, and the typed inserts name every column, so a missing value would
/// otherwise become an explicit NULL and violate the constraint.
fn from_merged<T>(
    merged: Value,
    project_id: &str,
    entity: &str,
    fill_defaults: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let mut obj = merged
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{} payload is not a JSON object", entity))?;

    obj.insert("project_id".to_string(), json!(project_id));
    fill_defaults(&mut obj);

    serde_json::from_value(Value::Object(obj))
        .map_err(|e| format!("Invalid {} payload: {}", entity, e))
}
