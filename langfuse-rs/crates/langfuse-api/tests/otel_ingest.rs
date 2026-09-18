//! End-to-end tests for `POST /api/public/otel/v1/traces`.
//!
//! These drive the real router against the real PostgreSQL instance, because
//! the failure modes that matter here are integration failures: the wire
//! decoding, the attribute → column mapping, the enum casts, and the NOT NULL /
//! uniqueness constraints on `observations`. A mocked repository would not have
//! caught the `model_parameters` / `observation_type` identifier-casing bugs,
//! nor the NULL binds into `observations.level` and `trace_sessions.environment`.
//!
//! The payload mirrors what LexQA's OTLP exporter emits
//! (`LexQA/internal/tracing/langfuse/{events,tracer}.go`): a root span carrying
//! `langfuse.observation.type=trace` plus `langfuse.trace.*`, children carrying
//! `langfuse.observation.*`, and usage in a `usage_details` JSON string keyed by
//! usage type.
//!
//! Run with: `cargo test -p langfuse-api --test otel_ingest`

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use base64::Engine;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};
use prost::Message;
use serde_json::{json, Value};
use sqlx::Row;
use std::io::Write;
use tower::ServiceExt;

mod common;
use common::{pool, Fixture};

// ---------------------------------------------------------------------------
// OTLP payload construction
// ---------------------------------------------------------------------------

/// Ids for one export.
///
/// `observations` has a primary key on `id` alone (the `(id, project_id)`
/// unique index is secondary), so a span id is globally unique — not unique per
/// project. Real OTLP span ids are random, but tests that hard-code one would
/// collide with each other since the suite runs in parallel; these are derived
/// per test instead.
struct Ids {
    trace: Vec<u8>,
    root: Vec<u8>,
    child: Vec<u8>,
    generation: Vec<u8>,
    error: Vec<u8>,
}

impl Ids {
    fn new() -> Self {
        let raw = *uuid::Uuid::new_v4().as_bytes();
        Self {
            trace: raw[0..16].to_vec(),
            root: raw[0..8].to_vec(),
            child: raw[8..16].to_vec(),
            generation: raw[1..9].to_vec(),
            error: raw[3..11].to_vec(),
        }
    }

    fn trace_hex(&self) -> String {
        hex::encode(&self.trace)
    }

    fn root_hex(&self) -> String {
        hex::encode(&self.root)
    }

    fn child_hex(&self) -> String {
        hex::encode(&self.child)
    }

    fn generation_hex(&self) -> String {
        hex::encode(&self.generation)
    }

    fn error_hex(&self) -> String {
        hex::encode(&self.error)
    }
}

fn str_attr(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_string())),
        }),
    }
}

/// Structured attributes travel as JSON strings, matching the official SDKs and
/// LexQA (`jsonAttr` in `tracer.go`).
fn json_attr(key: &str, value: &Value) -> KeyValue {
    str_attr(key, &serde_json::to_string(value).expect("serialize attribute"))
}

fn span(
    trace_id: &[u8],
    span_id: &[u8],
    parent_span_id: &[u8],
    name: &str,
    attributes: Vec<KeyValue>,
    status: Option<Status>,
) -> Span {
    Span {
        trace_id: trace_id.to_vec(),
        span_id: span_id.to_vec(),
        parent_span_id: parent_span_id.to_vec(),
        name: name.to_string(),
        start_time_unix_nano: 1_756_000_000_000_000_000,
        end_time_unix_nano: 1_756_000_001_500_000_000,
        attributes,
        status,
        ..Default::default()
    }
}

/// A LexQA-shaped export: HTTP root span, an asynq worker span beneath it, a
/// streaming chat generation beneath that, and one failing span.
fn lexqa_export(ids: &Ids) -> ExportTraceServiceRequest {
    let root = span(
        &ids.trace,
        &ids.root,
        &[],
        "POST /api/v1/knowledge-search",
        vec![
            str_attr("langfuse.observation.type", "trace"),
            str_attr("langfuse.trace.name", "POST /api/v1/knowledge-search"),
            json_attr("langfuse.trace.input", &json!({"query": "违约金上限"})),
            json_attr("langfuse.trace.metadata", &json!({"http.method": "POST"})),
            json_attr("langfuse.trace.tags", &json!(["http", "post"])),
            str_attr("user.id", "tenant:42"),
            str_attr("session.id", "session-abc"),
        ],
        None,
    );

    let child = span(
        &ids.trace,
        &ids.child,
        &ids.root,
        "asynq.document:process",
        vec![
            str_attr("langfuse.observation.type", "span"),
            json_attr("langfuse.observation.metadata", &json!({"task_id": "t-1"})),
        ],
        None,
    );

    let generation = span(
        &ids.trace,
        &ids.generation,
        &ids.child,
        "chat.completion.stream",
        vec![
            str_attr("langfuse.observation.type", "generation"),
            str_attr("langfuse.observation.model.name", "qwen-max"),
            json_attr(
                "langfuse.observation.input",
                &json!([{"role": "user", "content": "hi"}]),
            ),
            json_attr("langfuse.observation.output", &json!({"content": "hello"})),
            json_attr(
                "langfuse.observation.usage_details",
                &json!({
                    "input": 1200,
                    "output": 300,
                    "total": 1500,
                    "cache_read_input_tokens": 800,
                    "cache_creation_input_tokens": 100,
                    "cache_miss_input_tokens": 300
                }),
            ),
            str_attr(
                "langfuse.observation.completion_start_time",
                "2026-09-18T04:05:06.000Z",
            ),
        ],
        None,
    );

    let failing = span(
        &ids.trace,
        &ids.error,
        &ids.root,
        "rerank",
        vec![str_attr("langfuse.observation.type", "span")],
        Some(Status {
            code: 2,
            message: "upstream timeout".to_string(),
        }),
    );

    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![
                    str_attr("service.name", "lexqa"),
                    str_attr("langfuse.environment", "production"),
                    str_attr("langfuse.release", "v0.4.2"),
                ],
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans: vec![root, child, generation, failing],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
}

/// POST an OTLP body to the ingest endpoint.
async fn post_otlp(
    fixture: &Fixture,
    content_type: &str,
    body: Vec<u8>,
    gzip: bool,
) -> (StatusCode, Vec<u8>) {
    let (body, encoding) = if gzip {
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&body).expect("gzip body");
        (encoder.finish().expect("finish gzip"), Some("gzip"))
    } else {
        (body, None)
    };

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/public/otel/v1/traces")
        .header(header::AUTHORIZATION, fixture.basic_auth())
        .header(header::CONTENT_TYPE, content_type)
        // Every OTLP exporter sends this; the endpoint must accept it.
        .header("x-langfuse-ingestion-version", "4");

    if let Some(encoding) = encoding {
        request = request.header(header::CONTENT_ENCODING, encoding);
    }

    let response = fixture
        .app()
        .oneshot(request.body(Body::from(body)).expect("build request"))
        .await
        .expect("router responded");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body")
        .to_vec();

    (status, bytes)
}

/// Assert the OTLP response reports no rejected spans, surfacing the server's
/// error message when it does.
fn assert_full_success(body: &[u8]) {
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse;
    let response = ExportTraceServiceResponse::decode(body).expect("decode OTLP response");
    if let Some(partial) = response.partial_success {
        panic!(
            "expected full success but {} span(s) were rejected: {}",
            partial.rejected_spans, partial.error_message
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn otlp_protobuf_ingest_maps_trace_tree_and_usage() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "protobuf").await;
    let ids = Ids::new();

    let (status, response) = post_otlp(
        &fixture,
        "application/x-protobuf",
        lexqa_export(&ids).encode_to_vec(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "protobuf ingest should succeed");
    assert_full_success(&response);

    // ---- trace row -------------------------------------------------------
    let trace = sqlx::query(
        "SELECT name, user_id, session_id, tags, input, metadata, release
         FROM traces WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.trace_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("trace row exists");

    assert_eq!(trace.get::<String, _>("name"), "POST /api/v1/knowledge-search");
    assert_eq!(
        trace.get::<Option<String>, _>("user_id").as_deref(),
        Some("tenant:42")
    );
    assert_eq!(
        trace.get::<Option<String>, _>("session_id").as_deref(),
        Some("session-abc")
    );
    assert_eq!(
        trace.get::<Option<String>, _>("release").as_deref(),
        Some("v0.4.2")
    );
    let tags: Vec<String> = trace.get("tags");
    assert!(tags.contains(&"http".to_string()), "tags = {:?}", tags);
    let metadata: Value = trace.get("metadata");
    assert_eq!(metadata["service.name"], "lexqa");
    assert_eq!(metadata["environment"], "production");
    assert_eq!(metadata["http.method"], "POST");

    // The session must be registered so the Sessions view can aggregate it.
    let session_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM trace_sessions WHERE id = $1 AND project_id = $2)",
    )
    .bind("session-abc")
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("query trace_sessions");
    assert!(session_exists, "trace_sessions row should be created");

    // ---- root span must NOT become an observation ------------------------
    let observation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM observations WHERE project_id = $1 AND trace_id = $2",
    )
    .bind(&fixture.project_id)
    .bind(ids.trace_hex())
    .fetch_one(&pool)
    .await
    .expect("count observations");
    assert_eq!(
        observation_count, 3,
        "root span is the trace itself; only the 3 child spans are observations"
    );

    // ---- parent/child wiring --------------------------------------------
    let child = sqlx::query(
        "SELECT type::text AS type, parent_observation_id, level::text AS level
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.child_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("child span row");
    assert_eq!(child.get::<String, _>("type"), "SPAN");
    assert_eq!(
        child
            .get::<Option<String>, _>("parent_observation_id")
            .as_deref(),
        Some(ids.root_hex().as_str())
    );
    // A span without an error status must still satisfy the NOT NULL level.
    assert_eq!(child.get::<String, _>("level"), "DEFAULT");

    // ---- generation: usage, cache breakdown, TTFT ------------------------
    let generation = sqlx::query(
        "SELECT type::text AS type, model, level::text AS level,
                prompt_tokens, completion_tokens, total_tokens,
                usage_details, completion_start_time, parent_observation_id,
                input, output
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.generation_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("generation row");

    assert_eq!(generation.get::<String, _>("type"), "GENERATION");
    assert_eq!(
        generation.get::<Option<String>, _>("model").as_deref(),
        Some("qwen-max")
    );
    assert_eq!(generation.get::<i32, _>("prompt_tokens"), 1200);
    assert_eq!(generation.get::<i32, _>("completion_tokens"), 300);
    assert_eq!(generation.get::<i32, _>("total_tokens"), 1500);
    assert_eq!(
        generation
            .get::<Option<String>, _>("parent_observation_id")
            .as_deref(),
        Some(ids.child_hex().as_str())
    );

    let usage: Value = generation
        .get::<Option<Value>, _>("usage_details")
        .expect("usage_details stored");
    assert_eq!(usage["cache_read_input_tokens"], 800);
    assert_eq!(usage["cache_creation_input_tokens"], 100);
    assert_eq!(usage["cache_miss_input_tokens"], 300);

    // TTFT is only computable when completion_start_time survives.
    let completion_start = generation
        .get::<Option<chrono::NaiveDateTime>, _>("completion_start_time")
        .expect("completion_start_time stored");
    assert_eq!(completion_start.to_string(), "2026-09-18 04:05:06");

    // Structured payloads land as JSON, not as double-encoded strings.
    let input: Value = generation.get("input");
    assert!(input.is_array(), "input should be a JSON array, got {}", input);
    assert_eq!(input[0]["role"], "user");

    // ---- error span ------------------------------------------------------
    let error_span = sqlx::query(
        "SELECT level::text AS level, status_message FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.error_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("error span row");
    assert_eq!(error_span.get::<String, _>("level"), "ERROR");
    assert_eq!(
        error_span
            .get::<Option<String>, _>("status_message")
            .as_deref(),
        Some("upstream timeout")
    );

    fixture.cleanup().await;
}

#[tokio::test]
async fn otlp_accepts_gzipped_protobuf() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "gzip").await;
    let ids = Ids::new();

    // The Go otlptracehttp client gzips by default, so this is the shape LexQA
    // actually puts on the wire.
    let (status, response) = post_otlp(
        &fixture,
        "application/x-protobuf",
        lexqa_export(&ids).encode_to_vec(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_full_success(&response);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM observations WHERE project_id = $1 AND trace_id = $2",
    )
    .bind(&fixture.project_id)
    .bind(ids.trace_hex())
    .fetch_one(&pool)
    .await
    .expect("count observations");
    assert_eq!(count, 3, "gzipped export should decode to the same 3 spans");

    fixture.cleanup().await;
}

#[tokio::test]
async fn otlp_accepts_otlp_json() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "json").await;
    let ids = Ids::new();

    let json_body = serde_json::to_vec(&lexqa_export(&ids)).expect("serialize OTLP/JSON");
    let (status, body) = post_otlp(&fixture, "application/json", json_body, false).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "OTLP/JSON ingest should succeed; body = {}",
        String::from_utf8_lossy(&body)
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM observations WHERE project_id = $1 AND trace_id = $2",
    )
    .bind(&fixture.project_id)
    .bind(ids.trace_hex())
    .fetch_one(&pool)
    .await
    .expect("count observations");
    assert_eq!(count, 3, "body = {}", String::from_utf8_lossy(&body));

    fixture.cleanup().await;
}

/// Rewrite every id field of a serialized OTLP/JSON payload into `form`, so the
/// three encodings a client might emit can each be exercised.
fn reencode_ids(payload: &mut Value, form: &str) {
    match payload {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if matches!(key.as_str(), "traceId" | "spanId" | "parentSpanId") {
                    if let Value::String(hex_id) = entry {
                        let bytes = hex::decode(&*hex_id).expect("payload id is hex");
                        *entry = match form {
                            "base64" => Value::String(
                                base64::engine::general_purpose::STANDARD.encode(&bytes),
                            ),
                            "array" => {
                                Value::Array(bytes.iter().map(|b| json!(b)).collect())
                            }
                            _ => unreachable!("unknown id form"),
                        };
                    }
                } else {
                    reencode_ids(entry, form);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| reencode_ids(i, form)),
        _ => {}
    }
}

/// The OTLP/JSON spec says base64, older Langfuse payloads used byte arrays, and
/// `opentelemetry-proto` emits hex — a client should work whichever it sends.
#[tokio::test]
async fn otlp_json_accepts_base64_and_array_ids() {
    let pool = pool().await;

    for form in ["base64", "array"] {
        let fixture = Fixture::new(pool.clone(), form).await;
        let ids = Ids::new();

        let mut payload = serde_json::to_value(lexqa_export(&ids)).expect("serialize OTLP/JSON");
        reencode_ids(&mut payload, form);

        let (status, body) = post_otlp(
            &fixture,
            "application/json",
            serde_json::to_vec(&payload).expect("re-serialize"),
            false,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{form}-encoded ids should be accepted; body = {}",
            String::from_utf8_lossy(&body)
        );

        let trace_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM traces WHERE id = $1 AND project_id = $2)",
        )
        .bind(ids.trace_hex())
        .bind(&fixture.project_id)
        .fetch_one(&pool)
        .await
        .expect("query traces");
        assert!(trace_exists, "{form}-encoded traceId should map to the same hex id");

        fixture.cleanup().await;
    }
}

#[tokio::test]
async fn otlp_is_idempotent_for_repeated_batches() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "idempotent").await;
    let ids = Ids::new();

    // Exporters retry. A retry must not duplicate rows, which is why the
    // observation upsert conflicts on (id, project_id).
    for _ in 0..2 {
        let (status, response) = post_otlp(
            &fixture,
            "application/x-protobuf",
            lexqa_export(&ids).encode_to_vec(),
            false,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_full_success(&response);
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

    assert_eq!(traces, 1, "re-sending the batch must not add a second trace");
    assert_eq!(observations, 3, "re-sending the batch must not duplicate spans");

    fixture.cleanup().await;
}

#[tokio::test]
async fn otlp_rejects_missing_and_wrong_credentials() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "auth").await;
    let ids = Ids::new();

    // No Authorization header.
    let response = fixture
        .app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/public/otel/v1/traces")
                .header(header::CONTENT_TYPE, "application/x-protobuf")
                .body(Body::from(lexqa_export(&ids).encode_to_vec()))
                .expect("build request"),
        )
        .await
        .expect("router responded");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let error: Value = serde_json::from_slice(&body).expect("error body is JSON");
    assert!(
        error.get("message").is_some(),
        "401 body must carry a message, got {}",
        String::from_utf8_lossy(&body)
    );

    // Correct public key, wrong secret.
    let raw = format!("{}:{}", fixture.public_key, "sk-lf-not-the-secret");
    let bad_auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw)
    );
    let response = fixture
        .app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/public/otel/v1/traces")
                .header(header::AUTHORIZATION, bad_auth)
                .header(header::CONTENT_TYPE, "application/x-protobuf")
                .body(Body::from(lexqa_export(&ids).encode_to_vec()))
                .expect("build request"),
        )
        .await
        .expect("router responded");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traces WHERE project_id = $1")
        .bind(&fixture.project_id)
        .fetch_one(&pool)
        .await
        .expect("count traces");
    assert_eq!(written, 0, "rejected credentials must not write data");

    fixture.cleanup().await;
}

#[tokio::test]
async fn otlp_reports_partial_success_for_a_malformed_span() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "partial").await;
    let ids = Ids::new();

    // One span without a span id cannot be stored; the rest of the tree must
    // still land, and the OTLP response must say so instead of returning a
    // whole-request error that would make the exporter retry everything.
    let mut request = lexqa_export(&ids);
    request.resource_spans[0].scope_spans[0].spans[1].span_id = Vec::new();

    let (status, body) = post_otlp(
        &fixture,
        "application/x-protobuf",
        request.encode_to_vec(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "partial success is still HTTP 200");

    let response =
        opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse::decode(
            body.as_slice(),
        )
        .expect("decode OTLP response");
    let partial = response.partial_success.expect("partial_success reported");
    assert_eq!(partial.rejected_spans, 1);
    assert!(
        partial.error_message.contains("missing spanId"),
        "error message should name the cause, got {:?}",
        partial.error_message
    );

    let observations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE project_id = $1")
            .bind(&fixture.project_id)
            .fetch_one(&pool)
            .await
            .expect("count observations");
    assert_eq!(
        observations, 2,
        "the two valid non-root spans should be stored"
    );

    fixture.cleanup().await;
}

/// Costs must be computed from the project's configured model prices, using the
/// per-usage-type rates so cached tokens get their own (much lower) rate.
#[tokio::test]
async fn otlp_prices_usage_from_configured_model_rates() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "pricing").await;
    let ids = Ids::new();

    // Rates are per single token, matching the seeded defaults
    // (gpt-4o input = 0.0000025, input_cached_tokens = 0.00000125).
    let model_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"INSERT INTO models (
               id, project_id, model_name, match_pattern, unit,
               input_price, output_price, created_at, updated_at
           ) VALUES ($1, $2, 'qwen-max', 'qwen-.*', 'TOKENS', 0.000002, 0.00001, NOW(), NOW())"#,
    )
    .bind(&model_id)
    .bind(&fixture.project_id)
    .execute(&pool)
    .await
    .expect("insert model");

    let tier_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"INSERT INTO pricing_tiers (id, model_id, name, is_default, priority, conditions, created_at, updated_at)
           VALUES ($1, $2, 'default', true, 0, '{}'::jsonb, NOW(), NOW())"#,
    )
    .bind(&tier_id)
    .bind(&model_id)
    .execute(&pool)
    .await
    .expect("insert pricing tier");

    for (usage_type, price) in [
        ("input", "0.000002"),
        ("output", "0.00001"),
        ("cache_read_input_tokens", "0.0000002"),
        ("cache_creation_input_tokens", "0.000001"),
    ] {
        sqlx::query(
            r#"INSERT INTO prices (id, model_id, project_id, pricing_tier_id, usage_type, price, created_at, updated_at)
               VALUES (gen_random_uuid(), $1, $2, $3, $4, $5::numeric, NOW(), NOW())"#,
        )
        .bind(&model_id)
        .bind(&fixture.project_id)
        .bind(&tier_id)
        .bind(usage_type)
        .bind(price)
        .execute(&pool)
        .await
        .expect("insert price");
    }

    let (status, response) = post_otlp(
        &fixture,
        "application/x-protobuf",
        lexqa_export(&ids).encode_to_vec(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_full_success(&response);

    let generation = sqlx::query(
        "SELECT internal_model, internal_model_id,
                calculated_input_cost, calculated_output_cost, calculated_total_cost,
                cost_details
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.generation_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("generation row");

    assert_eq!(
        generation.get::<Option<String>, _>("internal_model").as_deref(),
        Some("qwen-max"),
        "the matched model should be recorded for later re-pricing"
    );
    assert_eq!(
        generation
            .get::<Option<String>, _>("internal_model_id")
            .as_deref(),
        Some(model_id.as_str())
    );

    // usage_details: input 1200, output 300, cache_read 800, cache_creation 100
    // cost_details is keyed by usage type, so the itemised lines are:
    //   input                       1200*0.000002   = 0.0024
    //   cache_read_input_tokens      800*0.0000002  = 0.00016
    //   cache_creation_input_tokens  100*0.000001   = 0.0001
    //   output                       300*0.00001    = 0.003
    // `calculated_*` is then the read path's reduction of that map — every key
    // with the `input` prefix sums into the input column, which is why the two
    // cache lines are *not* part of `calculated_input_cost` (their keys do not
    // start with "input"). This matches `reduceUsageOrCostDetails`, so the
    // stored column and what the UI derives agree.
    let input_cost: rust_decimal::Decimal = generation
        .get::<Option<rust_decimal::Decimal>, _>("calculated_input_cost")
        .expect("calculated_input_cost should be set once prices exist");
    let output_cost: rust_decimal::Decimal = generation
        .get::<Option<rust_decimal::Decimal>, _>("calculated_output_cost")
        .expect("calculated_output_cost should be set");
    let total_cost: rust_decimal::Decimal = generation
        .get::<Option<rust_decimal::Decimal>, _>("calculated_total_cost")
        .expect("calculated_total_cost should be set");

    // Decimal equality ignores trailing-zero scale, so these compare numerically
    // rather than against the literal 30-decimal rendering of numeric(65,30).
    let dec = |s: &str| s.parse::<rust_decimal::Decimal>().expect("test decimal");
    assert_eq!(input_cost, dec("0.0024"), "input = 1200 * 0.000002");
    assert_eq!(output_cost, dec("0.003"));
    assert_eq!(
        total_cost,
        dec("0.00566"),
        "total is the sum of every priced usage type"
    );

    let details: Value = generation
        .get::<Option<Value>, _>("cost_details")
        .expect("cost_details should be set");

    // Numbers, not strings: the frontend keeps a cost entry only when
    // `typeof value === "number"`.
    for key in [
        "input",
        "output",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ] {
        assert!(
            details.get(key).is_some_and(Value::is_number),
            "cost_details[{key}] should be a JSON number, got {details}"
        );
    }
    // `total` is what the tables read (`cost_details['total']`).
    assert_eq!(details.get("total"), Some(&json!(0.00566)), "{details}");

    fixture.cleanup().await;
}

/// A non-token unit bills at `total_price × quantity`; this is how LexQA's ASR
/// calls (audio length in seconds) are priced.
#[tokio::test]
async fn otlp_prices_second_units_with_the_flat_rate() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "pricing-seconds").await;
    let ids = Ids::new();

    let model_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"INSERT INTO models (
               id, project_id, model_name, match_pattern, unit,
               total_price, created_at, updated_at
           ) VALUES ($1, $2, 'whisper-large-v3', 'whisper-.*', 'SECONDS', 0.0001, NOW(), NOW())"#,
    )
    .bind(&model_id)
    .bind(&fixture.project_id)
    .execute(&pool)
    .await
    .expect("insert model");

    // An ASR generation: 12.5 seconds of audio, billed per second.
    let mut request = lexqa_export(&ids);
    let spans = &mut request.resource_spans[0].scope_spans[0].spans;
    spans[2].attributes = vec![
        str_attr("langfuse.observation.type", "generation"),
        str_attr("langfuse.observation.model.name", "whisper-large-v3"),
        json_attr(
            "langfuse.observation.usage_details",
            &json!({"total": 12.5, "unit": "SECONDS"}),
        ),
    ];

    let (status, response) = post_otlp(
        &fixture,
        "application/x-protobuf",
        request.encode_to_vec(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_full_success(&response);

    let row = sqlx::query(
        "SELECT unit, calculated_input_cost, calculated_output_cost, calculated_total_cost
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(ids.generation_hex())
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("ASR observation row");

    assert_eq!(row.get::<Option<String>, _>("unit").as_deref(), Some("SECONDS"));
    assert_eq!(
        row.get::<Option<rust_decimal::Decimal>, _>("calculated_input_cost"),
        None,
        "a flat per-second rate has no input/output split"
    );
    assert_eq!(
        row.get::<Option<rust_decimal::Decimal>, _>("calculated_output_cost"),
        None
    );
    let total: rust_decimal::Decimal = row
        .get::<Option<rust_decimal::Decimal>, _>("calculated_total_cost")
        .expect("ASR cost should be computed");
    // 12.5 * 0.0001
    assert_eq!(
        total,
        "0.00125".parse::<rust_decimal::Decimal>().expect("test decimal")
    );

    fixture.cleanup().await;
}

/// Write the exact protobuf payload LexQA's exporter produces to a file, so it
/// can be replayed with curl against a running server:
///
/// ```bash
/// LANGFUSE_OTLP_PAYLOAD_OUT=/tmp/lexqa-otlp.pb \
///   cargo test -p langfuse-api --test otel_ingest -- --ignored --nocapture \
///   dump_lexqa_otlp_payload
/// curl -u "pk-lf-lexqa-init:sk-lf-lexqa-init" \
///   -H 'Content-Type: application/x-protobuf' \
///   -H 'x-langfuse-ingestion-version: 4' \
///   --data-binary @/tmp/lexqa-otlp.pb \
///   http://127.0.0.1:8010/api/public/otel/v1/traces
/// ```
///
/// Ignored by default: it has no assertions and only exists to produce a fixture
/// whose bytes come from the same encoder the other tests use, rather than being
/// hand-assembled.
#[test]
#[ignore]
fn dump_lexqa_otlp_payload() {
    let path = std::env::var("LANGFUSE_OTLP_PAYLOAD_OUT")
        .expect("set LANGFUSE_OTLP_PAYLOAD_OUT to the file to write");
    let ids = Ids::new();
    let bytes = lexqa_export(&ids).encode_to_vec();
    std::fs::write(&path, &bytes).expect("write payload");

    // The ids are random per run, so print them for the follow-up SQL checks.
    println!("wrote {} bytes to {}", bytes.len(), path);
    println!("trace_id={}", ids.trace_hex());
    println!("root_span={}", ids.root_hex());
    println!("child_span={}", ids.child_hex());
    println!("generation_span={}", ids.generation_hex());
    println!("error_span={}", ids.error_hex());
}

#[tokio::test]
async fn public_health_needs_no_credentials() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "health").await;

    let response = fixture
        .app()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/public/health")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router responded");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "container healthchecks cannot send credentials"
    );

    fixture.cleanup().await;
}
