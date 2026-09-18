//! `POST /api/public/otel/v1/traces` — OTLP/HTTP trace ingestion.
//!
//! This is the direct-write path Langfuse v3+/v4 documents for OTLP exporters
//! (`OTEL_EXPORTER_OTLP_ENDPOINT=<host>/api/public/otel`), and the only ingest
//! path LexQA uses. It replaces the ClickHouse-backed `OtelIngestionQueue` of
//! upstream Langfuse with a synchronous write into PostgreSQL, matching the
//! "trace is visible as soon as the export returns" contract.
//!
//! Accepts both OTLP/HTTP encodings:
//!
//! | `Content-Type`           | Body                          |
//! |--------------------------|-------------------------------|
//! | `application/x-protobuf` | `ExportTraceServiceRequest`   |
//! | `application/json`       | OTLP/JSON                     |
//!
//! Id fields in JSON bodies are accepted as hex, base64, or byte arrays (see
//! [`decode_json`]). `Content-Encoding: gzip` is handled upstream by
//! `RequestDecompressionLayer`.
//!
//! The `x-langfuse-ingestion-version` header is accepted and ignored: upstream
//! uses it to route between two write paths, and this fork only has one.

use axum::{
    body::Bytes,
    extract::State,
    http::{header::CONTENT_TYPE, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_ingestion::otel::{ingest_spans, OtelSpan, ResourceAttributes};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use prost::Message;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::response::public_error;

/// OTLP/HTTP protobuf content types. `application/protobuf` is not in the spec
/// but is emitted by a few exporters.
const PROTOBUF_TYPES: &[&str] = &["application/x-protobuf", "application/protobuf"];

pub async fn handler(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(
            StatusCode::BAD_REQUEST,
            "This API key is organization-scoped; OTLP ingestion requires a project-scoped key",
            "bad_request",
        );
    };

    let request = match decode(&headers, &body) {
        Ok(request) => request,
        Err(message) => {
            return public_error(StatusCode::BAD_REQUEST, message, "bad_request");
        }
    };

    let wants_json = is_json(&headers);

    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for resource_spans in request.resource_spans {
        let resource: ResourceAttributes = resource_spans
            .resource
            .map(|r| attributes_to_map(&r.attributes))
            .unwrap_or_default();

        for scope_spans in resource_spans.scope_spans {
            let spans: Vec<OtelSpan> = scope_spans
                .spans
                .into_iter()
                .map(|span| OtelSpan {
                    trace_id: hex::encode(&span.trace_id),
                    span_id: hex::encode(&span.span_id),
                    parent_span_id: (!span.parent_span_id.is_empty())
                        .then(|| hex::encode(&span.parent_span_id)),
                    name: span.name,
                    start_time_unix_nano: span.start_time_unix_nano,
                    end_time_unix_nano: span.end_time_unix_nano,
                    attributes: attributes_to_map(&span.attributes),
                    status_code: span.status.as_ref().map(|s| s.code).unwrap_or(0),
                    status_message: span
                        .status
                        .map(|s| s.message)
                        .unwrap_or_default(),
                })
                .collect();

            let outcome = ingest_spans(&state.pool, &project_id, &resource, &spans).await;
            accepted += outcome.accepted_spans;
            rejected += outcome.rejected_spans;
            errors.extend(outcome.errors);
        }
    }

    tracing::debug!(
        project_id = %project_id,
        accepted,
        rejected,
        "otel: trace export processed"
    );

    // Partial success is the OTLP-native way to report per-span failures: the
    // exporter keeps the accepted spans, does not retry, and surfaces the
    // message. A whole-request 4xx would make the client retry the entire tree
    // and duplicate the spans that were already written.
    if rejected > 0 {
        return otlp_response(
            wants_json,
            Some((rejected as i64, truncate_errors(errors))),
        );
    }

    otlp_response(wants_json, None)
}

/// Build an `ExportTraceServiceResponse`, in the encoding the client used.
fn otlp_response(wants_json: bool, partial: Option<(i64, String)>) -> Response {
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTracePartialSuccess;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse;

    let response = ExportTraceServiceResponse {
        partial_success: partial.map(|(rejected_spans, error_message)| {
            ExportTracePartialSuccess {
                rejected_spans,
                error_message,
            }
        }),
    };

    if wants_json {
        // OTLP/JSON uses lowerCamelCase field names.
        let mut body = json!({});
        if let Some(ps) = &response.partial_success {
            body = json!({
                "partialSuccess": {
                    "rejectedSpans": ps.rejected_spans.to_string(),
                    "errorMessage": ps.error_message,
                }
            });
        }
        return (StatusCode::OK, axum::Json(body)).into_response();
    }

    let mut buf = Vec::with_capacity(response.encoded_len());
    match response.encode(&mut buf) {
        Ok(()) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "application/x-protobuf")],
            buf,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "otel: failed to encode response");
            public_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to encode OTLP response",
                "internal_error",
            )
        }
    }
}

/// Cap the error message so a pathological batch cannot produce an unbounded
/// response body.
fn truncate_errors(errors: Vec<String>) -> String {
    const MAX_ERRORS: usize = 10;
    const MAX_LEN: usize = 2000;

    let total = errors.len();
    let mut message = errors
        .into_iter()
        .take(MAX_ERRORS)
        .collect::<Vec<_>>()
        .join("; ");
    if total > MAX_ERRORS {
        message.push_str(&format!("; … and {} more", total - MAX_ERRORS));
    }
    if message.len() > MAX_LEN {
        message.truncate(MAX_LEN);
        message.push('…');
    }
    message
}

fn is_json(headers: &HeaderMap) -> bool {
    content_type(headers).contains("json")
}

fn content_type(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn decode(headers: &HeaderMap, body: &[u8]) -> Result<ExportTraceServiceRequest, String> {
    let content_type = content_type(headers);

    if content_type.contains("json") {
        return decode_json(body);
    }

    if PROTOBUF_TYPES.iter().any(|t| content_type.starts_with(t)) || content_type.is_empty() {
        match ExportTraceServiceRequest::decode(body) {
            Ok(request) => return Ok(request),
            Err(e) => {
                // Tolerate a JSON body sent without its Content-Type — common
                // when a human is testing with curl.
                if body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
                    return decode_json(body)
                        .map_err(|je| format!("Invalid OTLP payload (protobuf: {}; json: {})", e, je));
                }
                return Err(format!("Invalid OTLP/protobuf payload: {}", e));
            }
        }
    }

    Err(format!(
        "Unsupported Content-Type {:?}. Use application/x-protobuf or application/json",
        content_type
    ))
}

/// Span/trace id fields in OTLP/JSON, in both the trace and span shapes.
const ID_FIELDS: &[&str] = &[
    "traceId",
    "spanId",
    "parentSpanId",
    "linkedTraceId",
    "linkedSpanId",
];

/// Decode an OTLP/JSON body into the protobuf-backed request type.
///
/// `opentelemetry-proto`'s JSON mapping encodes `bytes` as **hex**, whereas the
/// OTLP/JSON spec says base64 and older Langfuse test payloads used raw byte
/// arrays. All three are accepted here so the endpoint works with whatever a
/// client emits; without this, only hex bodies would parse and a standard
/// base64 client would fail with a type error on `traceId`.
fn decode_json(body: &[u8]) -> Result<ExportTraceServiceRequest, String> {
    let mut value: Value = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OTLP/JSON payload: {}", e))?;

    normalize_id_encodings(&mut value);

    serde_json::from_value(value).map_err(|e| format!("Invalid OTLP/JSON payload: {}", e))
}

/// Rewrite every id field to hex in place.
fn normalize_id_encodings(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if ID_FIELDS.contains(&key.as_str()) {
                    if let Some(hex) = encode_id_as_hex(entry) {
                        *entry = Value::String(hex);
                    }
                } else {
                    normalize_id_encodings(entry);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_id_encodings),
        _ => {}
    }
}

/// Convert one id field — hex string, base64 string, or byte array — to hex.
/// Returns `None` for values that are none of those, leaving them untouched so
/// the decoder reports its own error.
fn encode_id_as_hex(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => {
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|i| i.as_u64().map(|n| n as u8))
                .collect();
            Some(hex::encode(bytes?))
        }
        Value::String(s) if !s.is_empty() => {
            // Hex wins ties: it is what this decoder's own serializer emits, and
            // a 32- or 16-character all-hex string is far more likely an id than
            // a base64 encoding of 24 or 12 bytes.
            if s.len() % 2 == 0 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(s.to_ascii_lowercase());
            }
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
            Some(hex::encode(bytes))
        }
        _ => None,
    }
}

fn attributes_to_map(attributes: &[KeyValue]) -> ResourceAttributes {
    attributes
        .iter()
        .map(|kv| {
            (
                kv.key.clone(),
                kv.value
                    .as_ref()
                    .and_then(any_value_to_json)
                    .unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn any_value_to_json(value: &AnyValue) -> Option<Value> {
    match value.value.as_ref()? {
        any_value::Value::StringValue(s) => Some(Value::String(s.clone())),
        any_value::Value::BoolValue(b) => Some(Value::Bool(*b)),
        any_value::Value::IntValue(i) => Some(json!(i)),
        any_value::Value::DoubleValue(d) => Some(json!(d)),
        any_value::Value::BytesValue(b) => Some(Value::String(hex::encode(b))),
        any_value::Value::ArrayValue(array) => Some(Value::Array(
            array
                .values
                .iter()
                .map(|v| any_value_to_json(v).unwrap_or(Value::Null))
                .collect(),
        )),
        any_value::Value::KvlistValue(list) => Some(Value::Object(
            list.values
                .iter()
                .map(|kv| {
                    (
                        kv.key.clone(),
                        kv.value
                            .as_ref()
                            .and_then(any_value_to_json)
                            .unwrap_or(Value::Null),
                    )
                })
                .collect(),
        )),
    }
}
