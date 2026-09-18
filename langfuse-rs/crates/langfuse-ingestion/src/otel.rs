//! OTLP/HTTP span → Langfuse record mapping.
//!
//! Implements the Langfuse v4 OpenTelemetry semantic conventions on the receive
//! side, so an OTLP/HTTP exporter can write traces without going through the
//! Langfuse SDK batch protocol. This is the path LexQA uses.
//!
//! Wire contract (attribute keys and value encodings) mirrors the official
//! `langfuse-python` v4 SDK and is verified against LexQA's emitter at
//! `LexQA/internal/tracing/langfuse/{events,tracer}.go`:
//!
//! ```text
//! langfuse.observation.type              "trace" | "span" | "generation"
//! langfuse.observation.input|output|metadata|model.parameters   JSON string
//! langfuse.observation.usage_details     JSON string, keyed by usage type
//! langfuse.observation.model.name        plain string
//! langfuse.observation.completion_start_time   ISO-8601
//! langfuse.trace.name|input|output|metadata|tags   JSON string
//! user.id / session.id                   plain string
//! langfuse.release / langfuse.environment   plain string
//! ```
//!
//! Span ids become observation ids (hex), the W3C trace id becomes the trace id,
//! and `parentSpanId` becomes `parent_observation_id`.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use langfuse_core::{ObservationLevel, ObservationRecord, ObservationType, TraceRecord};
use serde_json::Value;
use sqlx::PgPool;

use crate::sink;

// ---------------------------------------------------------------------------
// Attribute keys
// ---------------------------------------------------------------------------

const OBS_TYPE: &str = "langfuse.observation.type";
const OBS_INPUT: &str = "langfuse.observation.input";
const OBS_OUTPUT: &str = "langfuse.observation.output";
const OBS_METADATA: &str = "langfuse.observation.metadata";
const OBS_MODEL: &str = "langfuse.observation.model.name";
const OBS_MODEL_PARAMS: &str = "langfuse.observation.model.parameters";
const OBS_USAGE: &str = "langfuse.observation.usage_details";
const OBS_COST: &str = "langfuse.observation.cost_details";
const OBS_COMPLETION_START: &str = "langfuse.observation.completion_start_time";

const TRACE_NAME: &str = "langfuse.trace.name";
const TRACE_INPUT: &str = "langfuse.trace.input";
const TRACE_OUTPUT: &str = "langfuse.trace.output";
const TRACE_METADATA: &str = "langfuse.trace.metadata";
const TRACE_TAGS: &str = "langfuse.trace.tags";
const TRACE_VERSION: &str = "langfuse.version";

/// Official SDKs emit `user.id` / `session.id`; older ones used the
/// `langfuse.`-prefixed aliases. Accept both.
const USER_ID_KEYS: &[&str] = &["user.id", "langfuse.user.id"];
const SESSION_ID_KEYS: &[&str] = &["session.id", "langfuse.session.id"];
const ENVIRONMENT_KEYS: &[&str] = &["langfuse.environment", "environment"];
const RELEASE_KEYS: &[&str] = &["langfuse.release", "release"];
const SERVICE_NAME: &str = "service.name";

/// OTLP `Status.Code`: 0 = UNSET, 1 = OK, 2 = ERROR.
const STATUS_CODE_ERROR: i32 = 2;

// ---------------------------------------------------------------------------
// Input from the OTLP decoder
// ---------------------------------------------------------------------------

/// One OTLP span, already decoded from protobuf or JSON, with attribute values
/// normalised to `serde_json::Value`.
#[derive(Debug, Clone)]
pub struct OtelSpan {
    /// W3C trace id, lowercase hex (32 chars).
    pub trace_id: String,
    /// Span id, lowercase hex (16 chars).
    pub span_id: String,
    /// Parent span id in hex; empty or all-zero means "root".
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub attributes: BTreeMap<String, Value>,
    pub status_code: i32,
    pub status_message: String,
}

/// Resource-level attributes shared by every span in a `ResourceSpans` group.
pub type ResourceAttributes = BTreeMap<String, Value>;

/// Result of ingesting one OTLP request, shaped for the OTLP
/// `ExportTraceServiceResponse.partial_success` field.
#[derive(Debug, Default)]
pub struct IngestOutcome {
    pub accepted_spans: usize,
    pub rejected_spans: usize,
    pub errors: Vec<String>,
}

/// Map and persist every span in an OTLP request.
///
/// Per the OTLP spec this is a *partial success* operation: one malformed span
/// must not discard the rest of the tree, so each span is written on its own and
/// failures are collected instead of aborting the batch.
pub async fn ingest_spans(
    pool: &PgPool,
    project_id: &str,
    resource: &ResourceAttributes,
    spans: &[OtelSpan],
) -> IngestOutcome {
    let mut outcome = IngestOutcome::default();

    for span in spans {
        match ingest_span(pool, project_id, resource, span).await {
            Ok(()) => outcome.accepted_spans += 1,
            Err(e) => {
                outcome.rejected_spans += 1;
                let msg = format!("{}: {}", span.span_id, e);
                tracing::warn!(span_id = %span.span_id, error = %e, "otel: span rejected");
                outcome.errors.push(msg);
            }
        }
    }

    outcome
}

async fn ingest_span(
    pool: &PgPool,
    project_id: &str,
    resource: &ResourceAttributes,
    span: &OtelSpan,
) -> Result<(), String> {
    if span.trace_id.is_empty() {
        return Err("missing traceId".into());
    }
    if span.span_id.is_empty() {
        return Err("missing spanId".into());
    }

    let obs_type = span
        .attributes
        .get(OBS_TYPE)
        .and_then(Value::as_str)
        .unwrap_or("");

    if obs_type == "trace" {
        let record = build_trace(project_id, resource, span);
        sink::write_trace(pool, &record)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let record = build_observation(project_id, resource, span, obs_type);
    sink::write_observation(pool, &record)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

fn build_trace(
    project_id: &str,
    resource: &ResourceAttributes,
    span: &OtelSpan,
) -> TraceRecord {
    let mut metadata = span
        .attributes
        .get(TRACE_METADATA)
        .and_then(parse_json_attr)
        .and_then(|v| v.as_object().cloned())
        .map(|m| m.into_iter().collect::<serde_json::Map<_, _>>())
        .unwrap_or_default();

    // `service.name` is resource-scoped, not a user metadata key — only inject
    // it when the caller did not already claim the key.
    if let Some(service) = resource.get(SERVICE_NAME) {
        metadata
            .entry(SERVICE_NAME.to_string())
            .or_insert_with(|| service.clone());
    }
    if let Some(env) = pick(resource, ENVIRONMENT_KEYS).or_else(|| pick(&span.attributes, ENVIRONMENT_KEYS)) {
        // No `environment` column on `traces` in this fork.
        metadata
            .entry("environment".to_string())
            .or_insert_with(|| env.clone());
    }

    TraceRecord {
        id: span.trace_id.clone(),
        project_id: project_id.to_string(),
        external_id: None,
        timestamp: nanos_to_datetime(span.start_time_unix_nano),
        name: span
            .attributes
            .get(TRACE_NAME)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| Some(span.name.clone()).filter(|s| !s.is_empty())),
        user_id: pick(&span.attributes, USER_ID_KEYS)
            .and_then(|v| v.as_str().map(str::to_string)),
        metadata: (!metadata.is_empty()).then_some(Value::Object(metadata)),
        release: pick(&span.attributes, RELEASE_KEYS)
            .or_else(|| pick(resource, RELEASE_KEYS))
            .and_then(|v| v.as_str().map(str::to_string)),
        version: span
            .attributes
            .get(TRACE_VERSION)
            .and_then(Value::as_str)
            .map(str::to_string),
        public: Some(false),
        bookmarked: Some(false),
        tags: span.attributes.get(TRACE_TAGS).and_then(parse_tags),
        input: span.attributes.get(TRACE_INPUT).and_then(parse_json_attr),
        output: span.attributes.get(TRACE_OUTPUT).and_then(parse_json_attr),
        session_id: pick(&span.attributes, SESSION_ID_KEYS)
            .and_then(|v| v.as_str().map(str::to_string)),
    }
}

fn build_observation(
    project_id: &str,
    resource: &ResourceAttributes,
    span: &OtelSpan,
    obs_type: &str,
) -> ObservationRecord {
    let usage = span.attributes.get(OBS_USAGE).and_then(parse_usage_details);

    let mut metadata = span
        .attributes
        .get(OBS_METADATA)
        .and_then(parse_json_attr)
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    if let Some(service) = resource.get(SERVICE_NAME) {
        metadata
            .entry(SERVICE_NAME.to_string())
            .or_insert_with(|| service.clone());
    }

    // `level` and the three token columns are NOT NULL on `observations`. Their
    // database defaults only apply when the column is omitted, and this insert
    // names every column — so an explicit NULL is a constraint violation, not a
    // default. Absent values therefore have to be materialised here.
    let level = if span.status_code == STATUS_CODE_ERROR {
        ObservationLevel::Error
    } else {
        ObservationLevel::Default
    };

    ObservationRecord {
        id: span.span_id.clone(),
        trace_id: Some(span.trace_id.clone()),
        project_id: project_id.to_string(),
        obs_type: classify_observation_type(obs_type, &span.attributes, usage.is_some()),
        start_time: nanos_to_datetime(span.start_time_unix_nano),
        end_time: (span.end_time_unix_nano > 0)
            .then(|| nanos_to_datetime(span.end_time_unix_nano)),
        name: Some(span.name.clone()).filter(|s| !s.is_empty()),
        metadata: (!metadata.is_empty()).then_some(Value::Object(metadata)),
        parent_observation_id: span
            .parent_span_id
            .clone()
            .filter(|p| !p.is_empty() && !is_zero_id(p)),
        level: Some(level),
        status_message: (!span.status_message.is_empty())
            .then(|| span.status_message.clone()),
        version: span
            .attributes
            .get(TRACE_VERSION)
            .and_then(Value::as_str)
            .map(str::to_string),
        model: span
            .attributes
            .get(OBS_MODEL)
            .and_then(Value::as_str)
            .map(str::to_string),
        internal_model: None,
        internal_model_id: None,
        model_parameters: span
            .attributes
            .get(OBS_MODEL_PARAMS)
            .and_then(parse_json_attr),
        input: span.attributes.get(OBS_INPUT).and_then(parse_json_attr),
        output: span.attributes.get(OBS_OUTPUT).and_then(parse_json_attr),
        prompt_tokens: Some(usage.as_ref().and_then(|u| u.prompt_tokens).unwrap_or(0)),
        completion_tokens: Some(usage.as_ref().and_then(|u| u.completion_tokens).unwrap_or(0)),
        total_tokens: Some(usage.as_ref().and_then(|u| u.total_tokens).unwrap_or(0)),
        unit: usage.as_ref().and_then(|u| u.unit.clone()).or_else(|| {
            span.attributes
                .get(OBS_USAGE)
                .and_then(parse_json_attr)
                .and_then(|v| v.get("unit").and_then(Value::as_str).map(str::to_string))
        }),
        usage_details: usage.as_ref().map(|u| u.details.clone()),
        input_cost: None,
        output_cost: None,
        total_cost: None,
        cost_details: span.attributes.get(OBS_COST).and_then(parse_json_attr),
        calculated_input_cost: None,
        calculated_output_cost: None,
        calculated_total_cost: None,
        completion_start_time: span
            .attributes
            .get(OBS_COMPLETION_START)
            .and_then(Value::as_str)
            .and_then(parse_iso8601),
        prompt_id: None,
    }
}

fn classify_observation_type(obs_type: &str, attributes: &BTreeMap<String, Value>, has_usage: bool) -> ObservationType {
    match obs_type {
        "generation" => ObservationType::Generation,
        "agent" => ObservationType::Agent,
        "tool" => ObservationType::Tool,
        "chain" => ObservationType::Chain,
        "retriever" => ObservationType::Retriever,
        "evaluator" => ObservationType::Evaluator,
        "embedding" => ObservationType::Embedding,
        "guardrail" => ObservationType::Guardrail,
        "event" => ObservationType::Event,
        "span" => ObservationType::Span,
        // Unnamed spans are spans — except when they carry the marks of a model
        // call, which is what a generation is. LexQA always sets the type
        // explicitly, so this only affects other OTLP clients.
        _ => {
            if has_usage || attributes.contains_key(OBS_MODEL) {
                ObservationType::Generation
            } else {
                ObservationType::Span
            }
        }
    }
}

/// Token counts decoded from `langfuse.observation.usage_details`.
struct Usage {
    prompt_tokens: Option<i32>,
    completion_tokens: Option<i32>,
    total_tokens: Option<i32>,
    unit: Option<String>,
    details: Value,
}

/// Parse `usage_details`, which official SDKs send as a JSON string keyed by
/// usage type.
///
/// Both the modern keys (`input` / `output` / `total`) and the legacy
/// `*_tokens` spellings are accepted; `total` is derived when absent, because
/// several providers omit it and a zero there would understate cost.
fn parse_usage_details(value: &Value) -> Option<Usage> {
    let obj = parse_json_attr(value)?.as_object().cloned()?;

    let get_int = |keys: &[&str]| -> Option<i32> {
        keys.iter().find_map(|k| {
            obj.get(*k).and_then(|v| match v {
                Value::Number(n) => n.as_i64().map(|n| n as i32),
                Value::String(s) => s.parse::<i32>().ok(),
                _ => None,
            })
        })
    };

    let prompt_tokens = get_int(&["input", "prompt_tokens", "promptTokens"]);
    let completion_tokens = get_int(&["output", "completion_tokens", "completionTokens"]);
    let total_tokens = get_int(&["total", "total_tokens", "totalTokens"]).or_else(|| {
        match (prompt_tokens, completion_tokens) {
            (Some(i), Some(o)) => Some(i + o),
            _ => None,
        }
    });

    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        unit: obj.get("unit").and_then(Value::as_str).map(str::to_string),
        details: Value::Object(obj),
    })
}

// ---------------------------------------------------------------------------
// Attribute decoding helpers
// ---------------------------------------------------------------------------

/// Read an attribute that may be a JSON-encoded string (how official SDKs and
/// LexQA send structured fields) or already-structured JSON (how OTLP/JSON
/// clients may send it).
fn parse_json_attr(value: &Value) -> Option<Value> {
    match value {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }
            // A JSON-encoded attribute that fails to parse is still data worth
            // keeping — fall back to the raw string.
            Some(serde_json::from_str(trimmed).unwrap_or_else(|_| value.clone()))
        }
        Value::Null => None,
        other => Some(other.clone()),
    }
}

/// `langfuse.trace.tags` is a JSON array; tolerate a comma-separated string.
fn parse_tags(value: &Value) -> Option<Vec<String>> {
    match parse_json_attr(value)? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
        ),
        Value::String(s) => Some(
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        _ => None,
    }
}

fn pick<'a>(attributes: &'a BTreeMap<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| attributes.get(*k))
}

fn is_zero_id(id: &str) -> bool {
    id.chars().all(|c| c == '0')
}

/// OTLP carries timestamps as nanoseconds since the Unix epoch.
fn nanos_to_datetime(nanos: u64) -> DateTime<Utc> {
    let secs = (nanos / 1_000_000_000) as i64;
    let sub = (nanos % 1_000_000_000) as u32;
    Utc.timestamp_opt(secs, sub)
        .single()
        .unwrap_or_else(Utc::now)
}

/// `langfuse.observation.completion_start_time` is ISO-8601, e.g.
/// `2026-09-18T04:05:06.000Z`.
fn parse_iso8601(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}
