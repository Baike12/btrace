//! End-to-end tests for the public read API and the model-pricing endpoint.
//!
//! Run with: `cargo test -p langfuse-api --test public_read`

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;
use common::{pool, Fixture};

/// Send an authenticated request and decode the JSON response.
async fn call(fixture: &Fixture, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, fixture.basic_auth());

    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).expect("serialize body")))
            .expect("build request"),
        None => builder.body(Body::empty()).expect("build request"),
    };

    let response = fixture.app().oneshot(request).await.expect("router responded");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");

    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!(
                "{} {} returned non-JSON: {}",
                method,
                uri,
                String::from_utf8_lossy(&bytes)
            )
        })
    };

    (status, value)
}

/// Ingest one trace with two generations through the SDK batch endpoint.
async fn ingest_trace(fixture: &Fixture, ids: &Ids) -> Value {
    let body = json!({
        "batch": [
            {
                "id": format!("evt-{}", ids.trace),
                "type": "trace-create",
                "timestamp": "2026-09-18T10:00:00.000Z",
                "body": {
                    "id": ids.trace,
                    "timestamp": "2026-09-18T10:00:00.000Z",
                    "name": "read-api-trace",
                    "userId": "user-1",
                    "sessionId": ids.session,
                    "tags": ["api", "read"],
                    "release": "r-1",
                    "input": {"q": "hello"},
                    "output": {"a": "world"}
                }
            },
            {
                "id": format!("evt-{}", ids.generation),
                "type": "generation-create",
                "timestamp": "2026-09-18T10:00:01.000Z",
                "body": {
                    "id": ids.generation,
                    "traceId": ids.trace,
                    "name": "chat.completion",
                    "startTime": "2026-09-18T10:00:01.000Z",
                    "endTime": "2026-09-18T10:00:02.500Z",
                    "model": "qwen-max",
                    "usage": {
                        "input": 1000,
                        "output": 500,
                        "total": 1500,
                        "cache_read_input_tokens": 400,
                        "unit": "TOKENS"
                    }
                }
            },
            {
                "id": format!("evt-{}", ids.other_trace),
                "type": "trace-create",
                "timestamp": "2026-09-18T09:00:00.000Z",
                "body": {
                    "id": ids.other_trace,
                    "timestamp": "2026-09-18T09:00:00.000Z",
                    "name": "other-trace",
                    "userId": "user-2",
                    "tags": ["api"]
                }
            }
        ],
        "metadata": {}
    });

    let (status, body) = call(fixture, "POST", "/api/public/ingestion", Some(body)).await;
    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(body["errors"].as_array().map(Vec::len), Some(0), "{body}");
    body
}

struct Ids {
    trace: String,
    other_trace: String,
    generation: String,
    session: String,
}

impl Ids {
    fn new() -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let short = &suffix[..12];
        Self {
            trace: format!("trace-{}", short),
            other_trace: format!("other-{}", short),
            generation: format!("gen-{}", short),
            session: format!("sess-{}", short),
        }
    }
}

#[tokio::test]
async fn models_created_via_the_api_price_ingested_usage() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "read-models").await;
    let ids = Ids::new();

    // Configure pricing through the public API — the only path a machine client
    // has. Prices are per single token.
    let (status, model) = call(
        &fixture,
        "POST",
        "/api/public/models",
        Some(json!({
            "modelName": "qwen-max",
            "matchPattern": "qwen-.*",
            "unit": "TOKENS",
            "pricingTiers": [
                {
                    "name": "Standard",
                    "isDefault": true,
                    "priority": 0,
                    "conditions": [],
                    "prices": {
                        "input": {"price": 0.000002},
                        "output": {"price": 0.00001},
                        "cache_read_input_tokens": {"price": 0.0000002}
                    }
                }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{model}");
    assert_eq!(model["modelName"], "qwen-max");
    assert_eq!(model["isLangfuseManaged"], false, "a created model is a custom one");
    assert_eq!(model["prices"]["input"]["price"], 0.000002);
    assert_eq!(model["unit"], "TOKENS");
    let model_id = model["id"].as_str().expect("model id").to_string();

    // It must be listed back.
    let (status, listed) = call(&fixture, "GET", "/api/public/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listed["data"]
            .as_array()
            .expect("data array")
            .iter()
            .any(|m| m["id"] == json!(model_id)),
        "created model should appear in the list"
    );

    // Now ingest usage that names that model.
    let body = ingest_trace(&fixture, &ids).await;
    assert_eq!(body["successes"].as_array().map(Vec::len), Some(3), "{body}");

    let costs = sqlx::query_as::<_, (Option<rust_decimal::Decimal>, Option<rust_decimal::Decimal>, Option<rust_decimal::Decimal>, Option<Value>, Option<Value>)>(
        "SELECT calculated_input_cost, calculated_output_cost, calculated_total_cost, usage_details, cost_details
         FROM observations WHERE id = $1 AND project_id = $2",
    )
    .bind(&ids.generation)
    .bind(&fixture.project_id)
    .fetch_one(&pool)
    .await
    .expect("generation row");

    let dec = |s: &str| s.parse::<rust_decimal::Decimal>().expect("test decimal");

    // `cost_details` is keyed by usage type, so the cache read is its own line:
    //   input                     1000*0.000002  = 0.002
    //   cache_read_input_tokens    400*0.0000002 = 0.00008
    //   output                     500*0.00001   = 0.005
    let cost_details = costs.4.expect("cost_details");
    assert_eq!(cost_details["input"], json!(0.002), "{cost_details}");
    assert_eq!(
        cost_details["cache_read_input_tokens"],
        json!(0.00008),
        "{cost_details}"
    );
    assert_eq!(cost_details["total"], json!(0.00708), "{cost_details}");

    // The columns are the read path's reduction of that map, so input is the
    // `input` key alone — `cache_read_input_tokens` does not carry the prefix.
    assert_eq!(costs.0, Some(dec("0.002")), "input cost");
    assert_eq!(costs.1, Some(dec("0.005")), "output cost");
    assert_eq!(costs.2, Some(dec("0.00708")), "total cost");
    assert_eq!(costs.3.expect("usage_details")["cache_read_input_tokens"], 400);

    // Creating the same model twice is rejected, not silently duplicated.
    let (status, error) = call(
        &fixture,
        "POST",
        "/api/public/models",
        Some(json!({
            "modelName": "qwen-max",
            "matchPattern": "qwen-.*",
            "unit": "TOKENS",
            "inputPrice": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    // Mixing flat prices with tiers is rejected too.
    let (status, _) = call(
        &fixture,
        "POST",
        "/api/public/models",
        Some(json!({
            "modelName": "mixed",
            "matchPattern": "mixed.*",
            "inputPrice": 1,
            "pricingTiers": [{"name": "T", "isDefault": true, "priority": 0, "conditions": [], "prices": {"input": {"price": 1}}}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // And the model is deletable.
    let (status, _) = call(
        &fixture,
        "DELETE",
        &format!("/api/public/models/{}", model_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    fixture.cleanup().await;
}

#[tokio::test]
async fn trace_list_and_detail_follow_the_public_contract() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "read-traces").await;
    let ids = Ids::new();
    ingest_trace(&fixture, &ids).await;

    // ---- list ------------------------------------------------------------
    let (status, body) = call(&fixture, "GET", "/api/public/traces?limit=10", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["meta"]["totalItems"], 2, "{body}");
    assert_eq!(body["meta"]["page"], 1);
    assert_eq!(body["meta"]["limit"], 10);

    let traces = body["data"].as_array().expect("data array");
    assert_eq!(traces.len(), 2);
    // Default ordering is timestamp DESC.
    assert_eq!(traces[0]["name"], "read-api-trace");
    assert_eq!(traces[0]["userId"], "user-1");
    assert_eq!(traces[0]["sessionId"], ids.session);
    assert_eq!(traces[0]["release"], "r-1");
    assert_eq!(traces[0]["tags"], json!(["api", "read"]));
    // Latency and cost are rolled up from the trace's observations:
    // 2026-09-18T10:00:01 -> 10:00:02.500 = 1500 ms.
    assert_eq!(traces[0]["latency"], 1500.0);
    assert_eq!(traces[0]["observations"], json!([]), "children are not embedded in the list");

    // ---- filters ---------------------------------------------------------
    let (status, filtered) = call(&fixture, "GET", "/api/public/traces?userId=user-2", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered["meta"]["totalItems"], 1, "{filtered}");
    assert_eq!(filtered["data"][0]["name"], "other-trace");

    // `tags` requires ALL of them, so this matches nothing.
    let (_, both_tags) = call(
        &fixture,
        "GET",
        "/api/public/traces?tags=api&tags=read&tags=missing",
        None,
    )
    .await;
    assert_eq!(both_tags["meta"]["totalItems"], 0, "{both_tags}");

    let (_, one_tag) = call(&fixture, "GET", "/api/public/traces?tags=read", None).await;
    assert_eq!(one_tag["meta"]["totalItems"], 1, "{one_tag}");

    // orderBy is `[field].[asc|desc]`.
    let (_, ordered) = call(&fixture, "GET", "/api/public/traces?orderBy=name.asc", None).await;
    assert_eq!(ordered["data"][0]["name"], "other-trace", "{ordered}");

    let (status, bad_order) = call(&fixture, "GET", "/api/public/traces?orderBy=nope.asc", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_order}");

    // ---- detail ----------------------------------------------------------
    let (status, detail) = call(
        &fixture,
        "GET",
        &format!("/api/public/traces/{}", ids.trace),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["name"], "read-api-trace");
    assert_eq!(detail["input"], json!({"q": "hello"}));

    let observations = detail["observations"].as_array().expect("observations array");
    assert_eq!(observations.len(), 1, "{detail}");
    assert_eq!(observations[0]["id"], ids.generation);
    assert_eq!(observations[0]["type"], "GENERATION");
    assert_eq!(observations[0]["model"], "qwen-max");
    assert_eq!(observations[0]["usage"]["input"], 1000);
    assert_eq!(observations[0]["usage"]["output"], 500);
    // The cache breakdown survives round-trip through the SDK path.
    assert_eq!(
        observations[0]["usageDetails"]["cache_read_input_tokens"],
        400,
        "{detail}"
    );
    assert_eq!(observations[0]["latency"], 1500.0);

    // Unknown trace id.
    let (status, _) = call(&fixture, "GET", "/api/public/traces/does-not-exist", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    fixture.cleanup().await;
}

#[tokio::test]
async fn sessions_group_traces_and_expose_metrics() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "read-sessions").await;
    let ids = Ids::new();
    ingest_trace(&fixture, &ids).await;

    let (status, body) = call(&fixture, "GET", "/api/public/sessions", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["meta"]["totalItems"], 1, "{body}");
    assert_eq!(body["data"][0]["id"], ids.session);
    assert_eq!(body["data"][0]["environment"], "default");

    let (status, detail) = call(
        &fixture,
        "GET",
        &format!("/api/public/sessions/{}", ids.session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let traces = detail["traces"].as_array().expect("traces array");
    assert_eq!(traces.len(), 1, "{detail}");
    assert_eq!(traces[0]["id"], ids.trace);

    let (status, _) = call(&fixture, "GET", "/api/public/sessions/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // ---- daily metrics ---------------------------------------------------
    let (status, daily) = call(
        &fixture,
        "GET",
        "/api/public/metrics/daily?fromTimestamp=2026-09-17T00:00:00Z&toTimestamp=2026-09-19T00:00:00Z",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{daily}");

    let data = daily["data"].as_array().expect("daily data array");
    assert!(!data.is_empty(), "{daily}");

    let day = data
        .iter()
        .find(|d| d["date"] == "2026-09-18")
        .unwrap_or_else(|| panic!("no 2026-09-18 bucket in {daily}"));

    assert_eq!(day["countTraces"], 2, "{day}");
    assert_eq!(day["countObservations"], 1, "{day}");

    let usage = day["usage"].as_array().expect("usage array");
    let model_usage = usage
        .iter()
        .find(|u| u["model"] == "qwen-max")
        .unwrap_or_else(|| panic!("no qwen-max usage bucket in {day}"));
    assert_eq!(model_usage["inputTokens"], 1000);
    assert_eq!(model_usage["outputTokens"], 500);
    assert_eq!(model_usage["totalTokens"], 1500);
    assert_eq!(model_usage["unit"], "TOKENS");

    // An inverted range is a client error, not an empty result.
    let (status, _) = call(
        &fixture,
        "GET",
        "/api/public/metrics/daily?fromTimestamp=2026-09-19T00:00:00Z&toTimestamp=2026-09-17T00:00:00Z",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    fixture.cleanup().await;
}

#[tokio::test]
async fn unimplemented_v2_endpoints_say_so() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "read-v2").await;

    for uri in ["/api/public/v2/metrics", "/api/public/v2/observations"] {
        let (status, body) = call(&fixture, "GET", uri, None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{}: {}", uri, body);
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("not implemented")),
            "{} should explain itself: {}",
            uri,
            body
        );
    }

    fixture.cleanup().await;
}

#[tokio::test]
async fn public_read_requires_a_valid_key() {
    let pool = pool().await;
    let fixture = Fixture::new(pool.clone(), "read-auth").await;

    let response = fixture
        .app()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/public/traces")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router responded");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    fixture.cleanup().await;
}
