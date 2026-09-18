//! End-to-end tests for `POST /api/public/ingestion` (Langfuse SDK batch path).
//!
//! The path was rewritten to write synchronously through the same
//! `process_batch` the OTLP endpoint uses, so these tests pin the properties
//! that rewrite was for: rows exist as soon as the response returns, usage is
//! mapped the same way on both paths, and a retried batch stays idempotent.
//!
//! Run with: `cargo test -p langfuse-api --test ingestion`

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

mod common;
use common::{pool, Fixture};

async fn post_batch(fixture: &Fixture, batch: Value) -> (StatusCode, Value) {
    let response = fixture
        .app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/public/ingestion")
                .header(header::AUTHORIZATION, fixture.basic_auth())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "batch": batch, "metadata": {} }))
                        .expect("serialize batch"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router responded");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");

    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "response was not JSON ({}): {}",
                e,
                String::from_utf8_lossy(&bytes)
            )
        }),
    )
}

/// A batch in the shape the Langfuse SDKs send: a trace, a span and a
/// generation, each event wrapping its body.
fn sdk_batch(trace_id: &str, span_id: &str, generation_id: &str) -> Value {
    json!([
        {
            "id": format!("evt-{}", trace_id),
            "type": "trace-create",
            "timestamp": "2026-09-18T04:05:06.000Z",
            "body": {
                "id": trace_id,
                "timestamp": "2026-09-18T04:05:06.000Z",
                "name": "sdk-ingest",
                "userId": "u-1",
                "sessionId": "sess-sdk",
                "input": {"query": "hi"},
                "output": {"answer": "hello"},
                "metadata": {"k": "v"},
                "tags": ["sdk"],
                "release": "r-1"
            }
        },
        {
            "id": format!("evt-{}", span_id),
            "type": "span-create",
            "timestamp": "2026-09-18T04:05:06.000Z",
            "body": {
                "id": span_id,
                "traceId": trace_id,
                "name": "retrieve",
                "startTime": "2026-09-18T04:05:06.000Z",
                "endTime": "2026-09-18T04:05:07.000Z"
            }
        },
        {
            "id": format!("evt-{}", generation_id),
            "type": "generation-create",
            "timestamp": "2026-09-18T04:05:06.000Z",
            "body": {
                "id": generation_id,
                "traceId": trace_id,
                "parentObservationId": span_id,
                "name": "chat.completion",
                "startTime": "2026-09-18T04:05:06.000Z",
                "endTime": "2026-09-18T04:05:07.000Z",
                "model": "qwen-max",
                "modelParameters": {"temperature": 0.5},
                "input": {"messages": []},
                "output": {"content": "hello"},
                "usage": {
                    "input": 1200,
                    "output": 300,
                    "total": 1500,
                    "unit": "TOKENS"
                }
            }
        }
    ])
}

#[tokio::test]
async fn ingestion_writes_rows_before_the_response_returns() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "ingestion").await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let trace_id = format!("trace-{}", suffix);
    let span_id = format!("span-{}", suffix);
    let generation_id = format!("gen-{}", suffix);

    let (status, body) = post_batch(&fixture, sdk_batch(&trace_id, &span_id, &generation_id)).await;

    assert_eq!(status, StatusCode::MULTI_STATUS, "body = {}", body);
    assert_eq!(body["successes"].as_array().map(Vec::len), Some(3), "body = {}", body);
    assert_eq!(body["errors"].as_array().map(Vec::len), Some(0), "body = {}", body);

    // Read straight back: nothing is queued, so the rows must already be there.
    let trace = sqlx::query(
        "SELECT name, user_id, session_id, tags, release FROM traces WHERE id = $1 AND project_id = $2",
    )
    .bind(&trace_id)
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("trace row exists immediately after the response");

    assert_eq!(trace.get::<String, _>("name"), "sdk-ingest");
    assert_eq!(trace.get::<Option<String>, _>("user_id").as_deref(), Some("u-1"));
    assert_eq!(
        trace.get::<Option<String>, _>("session_id").as_deref(),
        Some("sess-sdk")
    );
    assert_eq!(trace.get::<Option<String>, _>("release").as_deref(), Some("r-1"));
    let tags: Vec<String> = trace.get("tags");
    assert!(tags.contains(&"sdk".to_string()), "tags = {:?}", tags);

    let session_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM trace_sessions WHERE id = $1 AND project_id = $2)",
    )
    .bind("sess-sdk")
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("query trace_sessions");
    assert!(session_exists);

    // Usage and parent linkage must match what the OTLP path records.
    let generation = sqlx::query(
        "SELECT type::text AS type, model, \"modelParameters\" AS model_parameters,
                prompt_tokens, completion_tokens, total_tokens, unit,
                usage_details, parent_observation_id, level::text AS level
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(&generation_id)
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("generation row exists");

    assert_eq!(generation.get::<String, _>("type"), "GENERATION");
    assert_eq!(
        generation.get::<Option<String>, _>("model").as_deref(),
        Some("qwen-max")
    );
    assert_eq!(generation.get::<i32, _>("prompt_tokens"), 1200);
    assert_eq!(generation.get::<i32, _>("completion_tokens"), 300);
    assert_eq!(generation.get::<i32, _>("total_tokens"), 1500);
    assert_eq!(generation.get::<Option<String>, _>("unit").as_deref(), Some("TOKENS"));
    assert_eq!(
        generation
            .get::<Option<String>, _>("parent_observation_id")
            .as_deref(),
        Some(span_id.as_str())
    );
    assert_eq!(generation.get::<String, _>("level"), "DEFAULT");

    // `modelParameters` is camelCase in the SDK payload; the serde alias is what
    // keeps it from being dropped.
    let model_parameters = generation
        .get::<Option<Value>, _>("model_parameters")
        .expect("modelParameters should survive ingestion");
    assert_eq!(model_parameters["temperature"], 0.5);

    let usage: Value = generation
        .get::<Option<Value>, _>("usage_details")
        .expect("usage_details should be stored");
    assert_eq!(usage["input"], 1200);

    fixture.cleanup().await;
}

#[tokio::test]
async fn ingestion_is_idempotent_for_a_retried_batch() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "ingestion-dedup").await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let trace_id = format!("trace-{}", suffix);
    let span_id = format!("span-{}", suffix);
    let generation_id = format!("gen-{}", suffix);

    let batch = sdk_batch(&trace_id, &span_id, &generation_id);
    for _ in 0..2 {
        let (status, body) = post_batch(&fixture, batch.clone()).await;
        assert_eq!(status, StatusCode::MULTI_STATUS, "body = {}", body);
        assert_eq!(body["errors"].as_array().map(Vec::len), Some(0), "body = {}", body);
    }

    let traces: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traces WHERE project_id = $1")
        .bind(&fixture.project_id)
        .fetch_one(&pool)
        .await
        .expect("count traces");
    let observations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE project_id = $1")
            .bind(&fixture.project_id)
            .fetch_one(&pool)
            .await
            .expect("count observations");

    assert_eq!(traces, 1, "a retried batch must not add a second trace");
    assert_eq!(observations, 2, "a retried batch must not duplicate spans");

    fixture.cleanup().await;
}

#[tokio::test]
async fn ingestion_reports_per_event_errors() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "ingestion-errors").await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let trace_id = format!("trace-{}", suffix);

    // The second event has no type, so it cannot be routed.
    let batch = json!([
        {
            "id": format!("evt-{}", trace_id),
            "type": "trace-create",
            "timestamp": "2026-09-18T04:05:06.000Z",
            "body": {
                "id": trace_id,
                "timestamp": "2026-09-18T04:05:06.000Z",
                "name": "ok"
            }
        },
        {
            "id": "evt-bad",
            "timestamp": "2026-09-18T04:05:06.000Z",
            "body": {"id": "bad-1"}
        }
    ]);

    let (status, body) = post_batch(&fixture, batch).await;
    assert_eq!(status, StatusCode::MULTI_STATUS);
    assert_eq!(body["successes"].as_array().map(Vec::len), Some(1), "body = {}", body);
    assert_eq!(body["errors"].as_array().map(Vec::len), Some(1), "body = {}", body);

    // The valid event must still have been written — one bad event does not
    // discard the batch.
    let trace_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM traces WHERE id = $1 AND project_id = $2)")
            .bind(&trace_id)
            .bind(&fixture.project_id)
            .fetch_one(&pool)
            .await
            .expect("query traces");
    assert!(trace_exists);

    fixture.cleanup().await;
}
